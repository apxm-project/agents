//! Shared embedded/reference-runtime composition for one exact APXM profile.
//!
//! This module is intentionally transport-neutral. An embedded caller and a
//! reference runtime service both construct this same profile from one verified
//! [`RuntimeAdmission`], then drive the same [`crate::driver::execute`] path.
//! The profile retains the immutable binding bundle for its whole lifetime and
//! keeps shutdown state instance-local; it never discovers, replaces, or
//! falls back to another implementation.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde_json::Value;

use apxm_inference::ModelCallRequestMetadataPort;
use apxm_kernel::{
    EventRef, ExactPortBinding, ProgramInstanceRef, ResourceCeilings, RuntimeAdmission,
};
use apxm_program::artifact::SchemaDigestRef;

use crate::bundle::ExecutionPortBundle;
use crate::driver::{
    CancellationToken, CapturedHookBodyHandler, ExecutionError, ExecutionPorts, ExecutionRequest,
    RunReport, StaticHookHandlerPort,
    execute_resumable_with_resource_ceilings as drive_execute_resumable,
    execute_with_resource_ceilings as drive_execute,
    resume_event_with_resource_ceilings as drive_resume_event,
    resume_with_resource_ceilings as drive_resume,
};
use crate::observe::{ObservationFailurePolicy, ObservationSink};
use crate::ports::{CompositionPort, EventPort};
use crate::resume::RunOutcome;

/// Why a profile could not be constructed or used.
#[derive(Debug)]
pub enum RuntimeProfileError {
    /// The event/composition binding set was not an exact match.
    Binding(apxm_kernel::BundleError),
    /// A driver implementation was supplied outside the immutable admission.
    BindingNotAdmitted(apxm_kernel::PortSlot),
    /// The kernel admission did not contain every driver port required by the
    /// canonical execution driver.
    MissingDriverPort(apxm_kernel::PortSlot),
    /// The profile was closed before a new invocation was admitted.
    Closed,
    /// The canonical driver rejected the invocation.
    Execution(ExecutionError),
    /// The invocation exceeded the admitted wall-clock ceiling. The in-flight
    /// driver future is cancelled by the owning timeout; no detached task is
    /// left to continue after this error is returned.
    WallTimeExceeded { max_wall_ms: u64 },
}

impl std::fmt::Display for RuntimeProfileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Binding(error) => error.fmt(formatter),
            Self::BindingNotAdmitted(slot) => {
                write!(
                    formatter,
                    "runtime profile binding was not admitted: {}",
                    slot.as_str()
                )
            }
            Self::MissingDriverPort(slot) => {
                write!(
                    formatter,
                    "runtime profile is missing driver port {}",
                    slot.as_str()
                )
            }
            Self::Closed => formatter.write_str("runtime profile is closed"),
            Self::Execution(error) => error.fmt(formatter),
            Self::WallTimeExceeded { max_wall_ms } => {
                write!(
                    formatter,
                    "runtime profile exceeded max wall time of {max_wall_ms}ms"
                )
            }
        }
    }
}

impl std::error::Error for RuntimeProfileError {}

/// One immutable APXM runtime composition shared by embedded and service
/// composition roots.
pub struct RuntimeProfile {
    // Retain the bundle so exact binding proofs remain part of the live
    // profile rather than becoming an unchecked construction-time detail.
    _bundle: ExecutionPortBundle,
    ports: ExecutionPorts,
    accepting: AtomicBool,
    cancellation: CancellationToken,
    resource_ceilings: ResourceCeilings,
}

/// Exact external driver bindings supplied alongside an admitted kernel.
pub struct RuntimeDriverBindings {
    pub expected_event_contract: SchemaDigestRef,
    pub event_binding: ExactPortBinding,
    pub events: Arc<dyn EventPort>,
    pub expected_composition_contract: SchemaDigestRef,
    pub composition_binding: ExactPortBinding,
    pub composition: Arc<dyn CompositionPort>,
}

impl RuntimeProfile {
    /// Attach the cancellation signal owned by the active service invocation.
    /// The signal is shared with the driver and with the wall-clock watchdog;
    /// cancelling either side therefore follows the same commit path.
    #[must_use]
    pub fn with_cancellation_token(mut self, cancellation: CancellationToken) -> Self {
        self.ports = self.ports.with_cancellation_token(cancellation.clone());
        self.cancellation = cancellation;
        self
    }

    /// Attach a bounded, non-authoritative observation sink to this admitted
    /// profile. The sink can observe scheduler/effect boundaries but cannot
    /// change execution or establish terminal truth.
    #[must_use]
    pub fn with_observation_sink(
        mut self,
        sink: Arc<dyn ObservationSink>,
        policy: ObservationFailurePolicy,
    ) -> Self {
        self.ports = self.ports.with_observation_sink(sink);
        self.ports = self.ports.with_observation_failure_policy(policy);
        self
    }

    /// Construct a profile from one already-admitted kernel runtime and two
    /// exact driver bindings. No implementation is selected here: the outer
    /// composition root supplies every adapter and the admission boundary has
    /// already verified its descriptor and confinement attestation.
    pub fn from_admission(
        admission: RuntimeAdmission,
        bindings: RuntimeDriverBindings,
        model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
        hook_handlers: Arc<dyn StaticHookHandlerPort>,
    ) -> Result<Self, RuntimeProfileError> {
        let RuntimeDriverBindings {
            expected_event_contract,
            event_binding,
            events,
            expected_composition_contract,
            composition_binding,
            composition,
        } = bindings;
        for (slot, supplied) in [
            (apxm_kernel::PortSlot::DurableEvent, &event_binding),
            (
                apxm_kernel::PortSlot::ProgramComposition,
                &composition_binding,
            ),
        ] {
            let admitted = admission
                .verified()
                .port_bindings
                .iter()
                .find(|binding| binding.slot == slot)
                .ok_or(RuntimeProfileError::BindingNotAdmitted(slot))?;
            if admitted != supplied {
                return Err(RuntimeProfileError::BindingNotAdmitted(slot));
            }
        }
        let resource_ceilings = admission.resource_ceilings().clone();
        let kernel = Arc::new(admission.into_bundle());
        let bundle = ExecutionPortBundle::construct(
            kernel,
            expected_event_contract,
            event_binding,
            events,
            expected_composition_contract,
            composition_binding,
            composition,
        )
        .map_err(RuntimeProfileError::Binding)?;
        let cancellation = CancellationToken::new();
        let ports = ExecutionPorts::from_admitted_bundle(
            &bundle,
            model_call_request_metadata,
            hook_handlers,
        )
        .map_err(|error| match error {
            crate::driver::ExecutionPortsError::MissingAdmittedPort(slot) => {
                RuntimeProfileError::MissingDriverPort(slot)
            }
        })?
        .with_resource_ceilings(resource_ceilings.clone())
        .with_cancellation_token(cancellation.clone());
        Ok(Self {
            _bundle: bundle,
            ports,
            accepting: AtomicBool::new(true),
            cancellation,
            resource_ceilings,
        })
    }

    /// Construct the shared profile from a bundle whose event and composition
    /// implementations were admitted by the kernel itself. This is the only
    /// composition path an external caller should use.
    pub fn from_fully_admitted(
        admission: RuntimeAdmission,
        model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
        hook_handlers: Arc<dyn StaticHookHandlerPort>,
    ) -> Result<Self, RuntimeProfileError> {
        let resource_ceilings = admission.resource_ceilings().clone();
        let bundle = ExecutionPortBundle::from_admitted_kernel(Arc::new(admission.into_bundle()))
            .map_err(RuntimeProfileError::Binding)?;
        let cancellation = CancellationToken::new();
        let ports = ExecutionPorts::from_admitted_bundle(
            &bundle,
            model_call_request_metadata,
            hook_handlers,
        )
        .map_err(|error| match error {
            crate::driver::ExecutionPortsError::MissingAdmittedPort(slot) => {
                RuntimeProfileError::MissingDriverPort(slot)
            }
        })?
        .with_resource_ceilings(resource_ceilings.clone())
        .with_cancellation_token(cancellation.clone());
        Ok(Self {
            _bundle: bundle,
            ports,
            accepting: AtomicBool::new(true),
            cancellation,
            resource_ceilings,
        })
    }

    /// Construct a fully admitted profile that runs Hooks from their captured
    /// bodies.
    pub fn from_fully_admitted_with_captured_hooks(
        admission: RuntimeAdmission,
        model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
    ) -> Result<Self, RuntimeProfileError> {
        Self::from_fully_admitted(
            admission,
            model_call_request_metadata,
            Arc::new(CapturedHookBodyHandler),
        )
    }

    /// Construct a profile that runs Hooks from their captured bodies.
    ///
    /// The caller must still provide the exact runtime admission, driver
    /// bindings, and model metadata port. This convenience only supplies the
    /// canonical captured-body handler; it does not select an implementation or
    /// weaken admission.
    pub fn from_admission_with_captured_hooks(
        admission: RuntimeAdmission,
        bindings: RuntimeDriverBindings,
        model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
    ) -> Result<Self, RuntimeProfileError> {
        Self::from_admission(
            admission,
            bindings,
            model_call_request_metadata,
            Arc::new(CapturedHookBodyHandler),
        )
    }

    /// Execute one admitted request through the canonical driver.
    ///
    /// Shutdown is instance-local and fail-closed: it prevents new work, while
    /// the caller-owned async runtime remains responsible for any invocation
    /// already admitted before shutdown.
    pub async fn execute(
        &self,
        request: ExecutionRequest,
        initial_context: Value,
    ) -> Result<RunReport, RuntimeProfileError> {
        if !self.accepting.load(Ordering::Acquire) {
            return Err(RuntimeProfileError::Closed);
        }
        let max_wall_ms = self.resource_ceilings.max_wall_ms;
        enforce_wall_time_with_token(max_wall_ms, self.cancellation.clone(), async {
            drive_execute(
                &self.ports,
                request,
                initial_context,
                Some(&self.resource_ceilings),
            )
            .await
            .map_err(RuntimeProfileError::Execution)
        })
        .await
    }

    /// Execute one admitted request with durable park/resume semantics under
    /// the same wall-clock ceiling as [`Self::execute`]. A parked continuation
    /// is a successful result of this call; the timeout only owns the active
    /// driver future and never spawns work that can outlive the caller.
    pub async fn execute_resumable(
        &self,
        request: ExecutionRequest,
        initial_context: Value,
    ) -> Result<RunOutcome, RuntimeProfileError> {
        if !self.accepting.load(Ordering::Acquire) {
            return Err(RuntimeProfileError::Closed);
        }
        let max_wall_ms = self.resource_ceilings.max_wall_ms;
        enforce_wall_time_with_token(max_wall_ms, self.cancellation.clone(), async {
            drive_execute_resumable(
                &self.ports,
                request,
                initial_context,
                Some(&self.resource_ceilings),
            )
            .await
            .map_err(RuntimeProfileError::Execution)
        })
        .await
    }

    /// Resume an already admitted structural continuation under the active
    /// call's wall-clock ceiling. Shutdown does not reject this method because
    /// it completes work that was admitted before shutdown.
    pub async fn resume(
        &self,
        program_instance_ref: &ProgramInstanceRef,
        delivered: Value,
    ) -> Result<RunOutcome, RuntimeProfileError> {
        let max_wall_ms = self.resource_ceilings.max_wall_ms;
        Box::pin(enforce_wall_time_with_token(
            max_wall_ms,
            self.cancellation.clone(),
            async {
                drive_resume(
                    &self.ports,
                    program_instance_ref,
                    delivered,
                    Some(&self.resource_ceilings),
                )
                .await
                .map_err(RuntimeProfileError::Execution)
            },
        ))
        .await
    }

    /// Resume an already admitted event continuation under the active call's
    /// wall-clock ceiling.
    pub async fn resume_event(
        &self,
        program_instance_ref: &ProgramInstanceRef,
        event_ref: EventRef,
        delivered: Value,
    ) -> Result<RunOutcome, RuntimeProfileError> {
        let max_wall_ms = self.resource_ceilings.max_wall_ms;
        Box::pin(enforce_wall_time_with_token(
            max_wall_ms,
            self.cancellation.clone(),
            async {
                drive_resume_event(
                    &self.ports,
                    program_instance_ref,
                    event_ref,
                    delivered,
                    Some(&self.resource_ceilings),
                )
                .await
                .map_err(RuntimeProfileError::Execution)
            },
        ))
        .await
    }

    /// Stop admitting new invocations. This operation is idempotent and has no
    /// process-global effect.
    pub fn shutdown(&self) {
        self.accepting.store(false, Ordering::Release);
        self.cancellation.cancel();
    }

    /// Whether this instance will accept another invocation.
    #[must_use]
    pub fn is_accepting(&self) -> bool {
        self.accepting.load(Ordering::Acquire)
    }

    /// Return the exact ceilings carried by the immutable runtime authority.
    #[must_use]
    pub fn resource_ceilings(&self) -> &ResourceCeilings {
        &self.resource_ceilings
    }
}

/// Run one active driver future under an admitted wall-clock ceiling.
///
/// `tokio::time::timeout` owns the future directly. On expiry it drops that
/// future before returning, so an invocation cannot keep executing in a
/// detached task after the profile reports the limit. A zero ceiling is
/// rejected explicitly because the admission contract defines zero as
/// refusing the resource class, rather than relying on timer edge behavior.
async fn enforce_wall_time_with_token<T, F>(
    max_wall_ms: u64,
    cancellation: CancellationToken,
    operation: F,
) -> Result<T, RuntimeProfileError>
where
    F: Future<Output = Result<T, RuntimeProfileError>>,
{
    if max_wall_ms == 0 {
        cancellation.cancel();
        return Err(RuntimeProfileError::WallTimeExceeded { max_wall_ms });
    }

    let mut operation = Box::pin(operation);
    tokio::select! {
        result = &mut operation => result,
        _ = tokio::time::sleep(Duration::from_millis(max_wall_ms)) => {
            cancellation.cancel();
            operation.await
        }
    }
}

#[cfg(test)]
async fn enforce_wall_time<T, F>(max_wall_ms: u64, operation: F) -> Result<T, RuntimeProfileError>
where
    F: Future<Output = Result<T, RuntimeProfileError>>,
{
    if max_wall_ms == 0 {
        return Err(RuntimeProfileError::WallTimeExceeded { max_wall_ms });
    }

    tokio::time::timeout(Duration::from_millis(max_wall_ms), operation)
        .await
        .map_err(|_| RuntimeProfileError::WallTimeExceeded { max_wall_ms })?
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;

    struct DropMarker(Arc<AtomicBool>);

    impl Drop for DropMarker {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn wall_limit_cancels_the_owned_operation_at_the_deadline() {
        let dropped = Arc::new(AtomicBool::new(false));
        let marker = DropMarker(dropped.clone());
        let operation = async move {
            let _marker = marker;
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<_, RuntimeProfileError>(())
        };

        let pending = enforce_wall_time(25, operation);
        tokio::pin!(pending);
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(25)).await;

        let error = pending.await.expect_err("wall limit must terminate work");
        assert!(matches!(
            error,
            RuntimeProfileError::WallTimeExceeded { max_wall_ms: 25 }
        ));
        assert!(
            dropped.load(Ordering::Acquire),
            "timeout must drop the owned operation rather than detach it"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn wall_limit_allows_normal_completion_before_the_deadline() {
        let pending = enforce_wall_time(25, async {
            tokio::time::sleep(Duration::from_millis(5)).await;
            Ok::<_, RuntimeProfileError>("completed")
        });
        tokio::pin!(pending);
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(5)).await;

        assert_eq!(
            pending.await.expect("normal work fits the ceiling"),
            "completed"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn wall_limit_preserves_a_suspended_continuation_outcome() {
        let pending = enforce_wall_time(25, async {
            Ok::<_, RuntimeProfileError>(RunOutcome::Suspended {
                continuation_id: "continuation.test".into(),
                event_ref: None,
                operational_usage:
                    crate::operational_usage::CommittedNativeModelUsageOutcome::NotConfigured,
            })
        });

        let outcome = pending.await.expect("parking is a normal bounded outcome");
        assert!(matches!(
            outcome,
            RunOutcome::Suspended {
                continuation_id,
                event_ref: None,
                ..
            } if continuation_id == "continuation.test"
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn zero_wall_limit_refuses_the_operation_without_polling_it() {
        let polled = Arc::new(AtomicBool::new(false));
        let operation_polled = polled.clone();
        let operation = async move {
            operation_polled.store(true, Ordering::Release);
            Ok::<_, RuntimeProfileError>(())
        };

        let error = enforce_wall_time(0, operation)
            .await
            .expect_err("zero means refuse the resource class");
        assert!(matches!(
            error,
            RuntimeProfileError::WallTimeExceeded { max_wall_ms: 0 }
        ));
        assert!(!polled.load(Ordering::Acquire));
    }
}
