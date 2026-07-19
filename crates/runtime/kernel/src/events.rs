//! Non-authoritative telemetry emitted after commit.
//!
//! Telemetry is strictly separate from runtime evidence: it is emitted only
//! after the atomic commit and can never establish, alter, or override lifecycle
//! truth. A telemetry sink is best-effort observation, not a commit path.

/// One post-commit telemetry note referencing the commit that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelemetryNote {
    pub commit_id: String,
    pub message: String,
}

/// A non-authoritative telemetry sink. Implementations must not participate in
/// the commit boundary or gate lifecycle progress.
pub trait EventSink: Send + Sync {
    fn emit(&self, note: TelemetryNote);
}

/// A telemetry sink that discards notes. Useful when no observer is attached.
pub struct NullEventSink;

impl EventSink for NullEventSink {
    fn emit(&self, _note: TelemetryNote) {}
}
