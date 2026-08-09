//! Shared embedded/reference-runtime composition for one exact APXM profile.
//!
//! This module is intentionally transport-neutral. An embedded caller and a
//! reference runtime service both construct this same profile from one verified
//! [`RuntimeAdmission`], then drive the same [`crate::driver::execute`] path.
//! The profile retains the immutable binding bundle for its whole lifetime and
//! keeps shutdown state instance-local; it never discovers, replaces, or
//! falls back to another implementation.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde_json::Value;

use apxm_inference::ModelCallRequestMetadataPort;
use apxm_kernel::{ExactPortBinding, ResourceCeilings, RuntimeAdmission};
use apxm_program::artifact::SchemaDigestRef;

use crate::bundle::ExecutionPortBundle;
use crate::driver::{
    ExecutionError, ExecutionPorts, ExecutionRequest, NoopStaticHookHandler, RunReport,
    StaticHookHandlerPort, execute,
};
use crate::ports::{CompositionPort, EventPort};

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
        let ports = ExecutionPorts::from_admitted_bundle(
            &bundle,
            model_call_request_metadata,
            hook_handlers,
        )
        .map_err(|error| match error {
            crate::driver::ExecutionPortsError::MissingAdmittedPort(slot) => {
                RuntimeProfileError::MissingDriverPort(slot)
            }
        })?;
        Ok(Self {
            _bundle: bundle,
            ports,
            accepting: AtomicBool::new(true),
            resource_ceilings,
        })
    }

    /// Construct the shared profile from a bundle whose event and composition
    /// implementations were admitted by the kernel itself. This is the only
    /// composition path a reference host should use.
    pub fn from_fully_admitted(
        admission: RuntimeAdmission,
        model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
        hook_handlers: Arc<dyn StaticHookHandlerPort>,
    ) -> Result<Self, RuntimeProfileError> {
        let resource_ceilings = admission.resource_ceilings().clone();
        let bundle = ExecutionPortBundle::from_admitted_kernel(Arc::new(admission.into_bundle()))
            .map_err(RuntimeProfileError::Binding)?;
        let ports = ExecutionPorts::from_admitted_bundle(
            &bundle,
            model_call_request_metadata,
            hook_handlers,
        )
        .map_err(|error| match error {
            crate::driver::ExecutionPortsError::MissingAdmittedPort(slot) => {
                RuntimeProfileError::MissingDriverPort(slot)
            }
        })?;
        Ok(Self {
            _bundle: bundle,
            ports,
            accepting: AtomicBool::new(true),
            resource_ceilings,
        })
    }

    /// Construct a fully admitted profile without static Hook handlers.
    pub fn from_fully_admitted_without_hooks(
        admission: RuntimeAdmission,
        model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
    ) -> Result<Self, RuntimeProfileError> {
        Self::from_fully_admitted(
            admission,
            model_call_request_metadata,
            Arc::new(NoopStaticHookHandler),
        )
    }

    /// Construct a profile for an AIR with no static Hook handlers.
    ///
    /// The caller must still provide the exact runtime admission, driver
    /// bindings, and model metadata port. This convenience only supplies the
    /// canonical no-op handler for the empty-hook case; it does not select an
    /// implementation or weaken admission.
    pub fn from_admission_without_hooks(
        admission: RuntimeAdmission,
        bindings: RuntimeDriverBindings,
        model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
    ) -> Result<Self, RuntimeProfileError> {
        Self::from_admission(
            admission,
            bindings,
            model_call_request_metadata,
            Arc::new(NoopStaticHookHandler),
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
        execute(&self.ports, request, initial_context)
            .await
            .map_err(RuntimeProfileError::Execution)
    }

    /// Stop admitting new invocations. This operation is idempotent and has no
    /// process-global effect.
    pub fn shutdown(&self) {
        self.accepting.store(false, Ordering::Release);
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
