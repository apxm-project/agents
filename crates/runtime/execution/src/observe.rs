//! Product-neutral live execution observations.
//!
//! Observations are a bounded, redacted view of decisions already made by the
//! driver. They are not runtime evidence, do not authorize work, and never
//! establish an invocation's terminal state. Durable adapters may copy the
//! same references after the Execution Commit boundary; the in-memory recorder
//! below is deliberately non-authoritative and reconnect-safe only while its
//! bounded retention lasts.

use apxm_runtime_protocol::execution_contracts::OccurrenceId;
use apxm_runtime_protocol::{
    AttemptId, Commitment, ContentRef, EXECUTION_OBSERVATION_CONTRACT, EventObservationRef,
    EvidenceRef, ExecutionCursor, ExecutionObservation, NodeExecutionId, ObservationId,
    ObservationKind, ObservationTiming, OutputRef, ProgramInvocationId, RegionOccurrenceId,
};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// The default bound for the process-local observation recorder.
pub const DEFAULT_MAX_OBSERVATIONS: usize = 4096;
/// The default byte bound for the process-local observation recorder.
pub const DEFAULT_MAX_OBSERVATION_BYTES: usize = 1024 * 1024;

/// A live sink rejected an observation. The driver applies its explicit
/// [`ObservationFailurePolicy`] rather than allowing a sink to decide whether
/// execution should stop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationSinkError {
    message: String,
}

impl ObservationSinkError {
    /// Construct a sink error without accepting content from the execution
    /// payload. Sink implementations should use references and diagnostics,
    /// not prompts, arguments, or model output, in this message.
    pub fn rejected(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ObservationSinkError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ObservationSinkError {}

/// Whether a live observation failure can stop the abstract-machine run.
///
/// Fail-open is the default because observations are non-authoritative. A
/// composition root that requires delivery may opt into fail-closed; in that
/// mode the driver returns an explicit execution error and does not cross
/// Execution Commit. Neither mode turns a dropped observation into evidence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ObservationFailurePolicy {
    #[default]
    FailOpen,
    FailClosed,
}

/// Product-neutral sink for redacted execution observations.
pub trait ObservationSink: Send + Sync {
    /// Receive one already-sequenced observation.
    ///
    /// The embedded cursor is a private, process-local live position. It is
    /// useful for ordering/reconnect diagnostics only and must never be
    /// accepted as a durable read cursor; a store-backed subscriber must mint
    /// its own scope/filter-bound cursor.
    fn publish(&self, observation: ExecutionObservation) -> Result<(), ObservationSinkError>;
}

/// Bounded process-local recorder for tests and short-lived reconnects.
///
/// This recorder does not persist data and is not an authority for execution
/// truth. Once its bound is reached it rejects new observations; a caller
/// using the default fail-open policy continues execution and may reconnect
/// only from the retained cursor range.
pub struct ObservationRecorder {
    observations: Mutex<Vec<ExecutionObservation>>,
    bytes: Mutex<usize>,
    notifier: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    max_observations: usize,
    max_bytes: usize,
}

impl Default for ObservationRecorder {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_OBSERVATIONS, DEFAULT_MAX_OBSERVATION_BYTES)
    }
}

impl ObservationRecorder {
    /// Construct a recorder with explicit count and byte bounds.
    #[must_use]
    pub fn new(max_observations: usize, max_bytes: usize) -> Self {
        Self {
            observations: Mutex::new(Vec::new()),
            bytes: Mutex::new(0),
            notifier: Mutex::new(None),
            max_observations,
            max_bytes,
        }
    }

    /// Bind a best-effort wakeup to publication. The callback is invoked only
    /// after an observation has been retained, so subscribers never wake for
    /// an item that the bounded recorder rejected.
    pub fn set_notifier(&self, notifier: Arc<dyn Fn() + Send + Sync>) {
        *self.notifier.lock().expect("observation notifier lock") = Some(notifier);
    }

    /// Return a snapshot of retained observations in sequence order.
    ///
    /// This is an in-process diagnostic snapshot. Its embedded live cursors
    /// are non-resumable outside this recorder and are not authoritative read
    /// cursors for a durable observation stream.
    #[must_use]
    pub fn snapshot(&self) -> Vec<ExecutionObservation> {
        self.observations
            .lock()
            .expect("observation recorder lock")
            .clone()
    }

    /// Return the first retained sequence, if any.
    #[must_use]
    pub fn retention_floor(&self) -> Option<u64> {
        self.observations
            .lock()
            .expect("observation recorder lock")
            .first()
            .map(|observation| observation.sequence)
    }

    /// Return the most recently retained sequence, if any.
    #[must_use]
    pub fn high_watermark(&self) -> Option<u64> {
        self.observations
            .lock()
            .expect("observation recorder lock")
            .last()
            .map(|observation| observation.sequence)
    }
}

impl ObservationSink for ObservationRecorder {
    fn publish(&self, observation: ExecutionObservation) -> Result<(), ObservationSinkError> {
        observation.validate().map_err(|error| {
            ObservationSinkError::rejected(format!("invalid observation: {error}"))
        })?;
        let bytes = serde_json::to_vec(&observation)
            .map_err(|error| {
                ObservationSinkError::rejected(format!("encode observation: {error}"))
            })?
            .len();
        let mut observations = self.observations.lock().expect("observation recorder lock");
        let mut retained_bytes = self.bytes.lock().expect("observation byte counter lock");
        if observations.len() >= self.max_observations
            || retained_bytes
                .checked_add(bytes)
                .is_none_or(|total| total > self.max_bytes)
        {
            return Err(ObservationSinkError::rejected(
                "bounded live observation retention exhausted",
            ));
        }
        observations.push(observation);
        *retained_bytes += bytes;
        if let Some(notifier) = self
            .notifier
            .lock()
            .expect("observation notifier lock")
            .as_ref()
        {
            notifier();
        }
        Ok(())
    }
}

/// Build one observation from the driver's authoritative coordinates. This is
/// kept in the execution crate so adapters cannot mint sequence, cursor, or
/// timing values independently.
#[allow(clippy::too_many_arguments)]
pub(crate) fn make_observation(
    invocation_id: &str,
    sequence: u64,
    timing: ObservationTiming,
    kind: ObservationKind,
    commitment: Commitment,
    node_execution_id: Option<&str>,
    occurrence_id: Option<&str>,
    attempt_id: Option<&str>,
    region_occurrence_id: Option<&str>,
    content_ref: Option<&str>,
    output_ref: Option<&str>,
    evidence_ref: Option<&str>,
    event_ref: Option<&str>,
) -> Result<ExecutionObservation, ObservationSinkError> {
    let invocation_id = ProgramInvocationId::new(invocation_id).map_err(|error| {
        ObservationSinkError::rejected(format!("invalid invocation ref: {error}"))
    })?;
    // This cursor is an internal live-stream position, not a reader cursor.
    // Reader/service adapters mint scope/filter-bound cursors at their own
    // boundary. Include the exact position here so reconnect bookkeeping is
    // unambiguous when a sink drops an observation between deliveries.
    let cursor = ExecutionCursor::new(
        sequence,
        format!("execution-cursor.{}.{}", invocation_id.as_str(), sequence),
    )
    .map_err(|error| {
        ObservationSinkError::rejected(format!("invalid observation cursor: {error}"))
    })?;
    let typed_occurrence = occurrence_id
        .map(OccurrenceId::new)
        .transpose()
        .map_err(|error| {
            ObservationSinkError::rejected(format!("invalid occurrence ref: {error}"))
        })?;
    let typed = ExecutionObservation {
        contract: EXECUTION_OBSERVATION_CONTRACT.to_owned(),
        observation_id: ObservationId::new(format!(
            "observation.{}.{}",
            invocation_id.as_str(),
            sequence
        ))
        .map_err(|error| {
            ObservationSinkError::rejected(format!("invalid observation id: {error}"))
        })?,
        program_invocation_id: invocation_id,
        node_execution_id: node_execution_id
            .map(NodeExecutionId::new)
            .transpose()
            .map_err(|error| {
                ObservationSinkError::rejected(format!("invalid node ref: {error}"))
            })?,
        occurrence_id: typed_occurrence,
        event_ref: event_ref.map(|reference| EventObservationRef {
            event_ref: reference.to_owned(),
            generation: None,
            occurrence_id: None,
        }),
        attempt_id: attempt_id
            .map(AttemptId::new)
            .transpose()
            .map_err(|error| {
                ObservationSinkError::rejected(format!("invalid attempt ref: {error}"))
            })?,
        region_occurrence_id: region_occurrence_id
            .map(RegionOccurrenceId::new)
            .transpose()
            .map_err(|error| {
                ObservationSinkError::rejected(format!("invalid region ref: {error}"))
            })?,
        sequence,
        cursor,
        timing,
        observation_kind: kind,
        commitment,
        content_ref: content_ref
            .map(ContentRef::new)
            .transpose()
            .map_err(|error| {
                ObservationSinkError::rejected(format!("invalid content ref: {error}"))
            })?,
        output_ref: output_ref
            .map(OutputRef::new)
            .transpose()
            .map_err(|error| {
                ObservationSinkError::rejected(format!("invalid output ref: {error}"))
            })?,
        evidence_ref: evidence_ref
            .map(EvidenceRef::new)
            .transpose()
            .map_err(|error| {
                ObservationSinkError::rejected(format!("invalid evidence ref: {error}"))
            })?,
    };
    typed
        .validate()
        .map_err(|error| ObservationSinkError::rejected(format!("invalid observation: {error}")))?;
    Ok(typed)
}

/// Current wall-clock time for the observation contract. Duration is supplied
/// by the driver's monotonic [`std::time::Instant`] measurements.
#[must_use]
pub(crate) fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().try_into().unwrap_or(u64::MAX)
        })
}

/// Closed approval decision for unresolved `Ask`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Caller allowed the Capability.
    Allow,
    /// Caller denied the Capability.
    Deny,
    /// Broker timed out; fail closed.
    Timeout,
}

/// Approval-broker Port Contract (ADR-0023).
#[async_trait]
pub trait ApprovalBroker: Send + Sync {
    /// Resolve one unresolved Ask. Denied and timeout must not execute.
    async fn resolve_ask(&self, ask_id: &str) -> ApprovalDecision;
}

/// Broker that always denies, used to prove Ask cannot execute.
#[derive(Default)]
pub struct DenyBroker;

#[async_trait]
impl ApprovalBroker for DenyBroker {
    async fn resolve_ask(&self, _ask_id: &str) -> ApprovalDecision {
        ApprovalDecision::Deny
    }
}

/// Broker that times out, used to prove Ask cannot execute on timeout.
#[derive(Default)]
pub struct TimeoutBroker;

#[async_trait]
impl ApprovalBroker for TimeoutBroker {
    async fn resolve_ask(&self, _ask_id: &str) -> ApprovalDecision {
        ApprovalDecision::Timeout
    }
}

/// Broker that allows Ask.
#[derive(Default)]
pub struct AllowBroker;

#[async_trait]
impl ApprovalBroker for AllowBroker {
    async fn resolve_ask(&self, _ask_id: &str) -> ApprovalDecision {
        ApprovalDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_preserves_typed_order_and_bounds_reconnect() {
        let recorder = ObservationRecorder::new(2, usize::MAX);
        for sequence in 1..=2 {
            recorder
                .publish(
                    make_observation(
                        "invocation.1",
                        sequence,
                        ObservationTiming {
                            observed_at_unix_ms: sequence,
                            duration_ms: None,
                        },
                        ObservationKind::NodeStarted,
                        Commitment::Provisional,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    )
                    .expect("observation"),
                )
                .expect("retained");
        }
        assert_eq!(recorder.retention_floor(), Some(1));
        assert_eq!(recorder.high_watermark(), Some(2));
        assert!(
            recorder
                .publish(
                    make_observation(
                        "invocation.1",
                        3,
                        ObservationTiming {
                            observed_at_unix_ms: 3,
                            duration_ms: None,
                        },
                        ObservationKind::NodeStarted,
                        Commitment::Provisional,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    )
                    .expect("observation")
                )
                .is_err()
        );
    }

    #[test]
    fn recorder_notifies_only_after_live_publication() {
        let recorder = ObservationRecorder::new(4, usize::MAX);
        let notifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = notifications.clone();
        recorder.set_notifier(Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        recorder
            .publish(
                make_observation(
                    "invocation.live",
                    1,
                    ObservationTiming {
                        observed_at_unix_ms: 1,
                        duration_ms: None,
                    },
                    ObservationKind::InvocationStarted,
                    Commitment::Provisional,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .expect("observation"),
            )
            .expect("retained");
        assert_eq!(notifications.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(recorder.snapshot().len(), 1);
    }

    #[tokio::test]
    async fn approval_brokers_remain_fail_closed_types() {
        assert_eq!(DenyBroker.resolve_ask("ask").await, ApprovalDecision::Deny);
        assert_eq!(
            TimeoutBroker.resolve_ask("ask").await,
            ApprovalDecision::Timeout
        );
        assert_eq!(
            AllowBroker.resolve_ask("ask").await,
            ApprovalDecision::Allow
        );
    }
}
