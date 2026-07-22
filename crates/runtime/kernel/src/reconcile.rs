//! Lifecycle reconstruction from durable evidence.
//!
//! Lifecycle truth is the latest scoped state fact. A crash leaves only the
//! facts that were atomically committed, so an interrupted, uncommitted step is
//! never reconstructed as success, and an `effect.outcome_unknown` fact never
//! upgrades to a committed outcome.

use apxm_program::runtime_evidence::{FactKind, InstanceState, InvocationState, RuntimeEvidence};

/// The reconstructed view of an instance/invocation from evidence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LifecycleView {
    pub instance_state: Option<InstanceState>,
    pub invocation_state: Option<InvocationState>,
    pub committed: bool,
    pub outcome_unknown: bool,
    pub last_event_sequence: u64,
}

/// Reconstruct the lifecycle view from a durable evidence sequence.
#[must_use]
pub fn reconstruct(evidence: &RuntimeEvidence) -> LifecycleView {
    let mut view = LifecycleView::default();

    for fact in &evidence.facts {
        view.last_event_sequence = fact.event_sequence();

        if let Some(runtime) = fact.runtime() {
            if let Some(state) = runtime.instance_state {
                view.instance_state = Some(state);
            }
            if let Some(state) = runtime.invocation_state {
                view.invocation_state = Some(state);
                if state.is_committed() {
                    view.committed = true;
                }
            }
            if fact.is_kind(FactKind::EffectOutcomeUnknown) {
                view.outcome_unknown = true;
            }
            if fact.is_kind(FactKind::InvocationCommitted) {
                view.committed = true;
            }
        }
    }

    view
}
