//! Runtime-owned post-commit native-usage handoff.
//!
//! The runtime publishes only a committed native `model.call` measurement to
//! an injected composition port. Server owns whether and how that measurement
//! becomes an admitted operational usage fact, including its authority,
//! pricing, reservation, evidence shape, and delivery transport.

use std::fmt;

use async_trait::async_trait;

use apxm_inference::Usage;

/// The immutable native measurement associated with a successful runtime
/// execution commit. Peer/ACP usage has no representation here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommittedNativeUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl From<Usage> for CommittedNativeUsage {
    fn from(value: Usage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
        }
    }
}

/// The generic committed native measurement a composition-owned publisher
/// receives after the atomic execution commit succeeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedNativeUsageFact {
    pub commit_id: String,
    pub invocation_ref: String,
    pub evidence_position_ref: String,
    pub native_usage: CommittedNativeUsage,
}

/// The narrow post-commit publication seam. Its implementation is supplied by
/// composition; the runtime neither discovers a destination nor owns the
/// admitted operational-usage representation.
#[async_trait]
pub trait OperationalUsageFactPort: Send + Sync {
    async fn publish(
        &self,
        fact: CommittedNativeUsageFact,
    ) -> Result<(), OperationalUsageFactError>;
}

/// A composition publisher's closed delivery result as reported by runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationalUsageFactError {
    Unavailable,
    Rejected,
}

impl fmt::Display for OperationalUsageFactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for OperationalUsageFactError {}

/// The report-visible post-commit publication outcome. A failed publication
/// remains explicit and cannot turn a committed runtime execution into a
/// fabricated admitted usage fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationalUsageOutcome {
    NotApplicable,
    NotConfigured,
    Published,
    Failed(OperationalUsageFactError),
}
