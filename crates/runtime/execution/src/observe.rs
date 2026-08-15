//! Execution-internal live observation and approval-broker ports.
//!
//! These traits are bound by the Runtime Composition Root. They do not own
//! Program behavior and cannot fabricate terminal outcomes from a stream.

use async_trait::async_trait;

/// Ordered live observations the service may project to authorized clients.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation {
    /// Provisional model content identified by content reference.
    ProvisionalContent {
        /// Content reference, not inline payload.
        content_ref: String,
    },
    /// Authorized Event lifecycle observation without leaking payloads.
    EventLifecycle {
        /// Event id without occurrence payload.
        event_id: String,
        /// Closed lifecycle label.
        phase: String,
    },
    /// Terminal commit has been recorded.
    TerminalCommit {
        /// Commit id.
        commit_id: String,
    },
}

/// Execution-internal observer. Implementations must not commit evidence.
pub trait ExecutionObserver: Send + Sync {
    /// Publish one observation after it is already decided by the driver.
    fn observe(&self, observation: Observation);
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

/// Observer that records observations for deterministic tests.
#[derive(Default)]
pub struct RecordingObserver {
    pub observations: std::sync::Mutex<Vec<Observation>>,
}

impl ExecutionObserver for RecordingObserver {
    fn observe(&self, observation: Observation) {
        self.observations
            .lock()
            .expect("observer lock")
            .push(observation);
    }
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

/// Broker that allows Ask. Deny and Timeout remain the fail-closed defaults.
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
    fn observer_order_is_preserved() {
        let observer = RecordingObserver::default();
        observer.observe(Observation::ProvisionalContent {
            content_ref: "c1".to_owned(),
        });
        observer.observe(Observation::TerminalCommit {
            commit_id: "commit".to_owned(),
        });
        let items = observer.observations.lock().unwrap();
        assert!(matches!(items[0], Observation::ProvisionalContent { .. }));
        assert!(matches!(items[1], Observation::TerminalCommit { .. }));
    }

    #[tokio::test]
    async fn denied_ask_is_not_allow() {
        assert_eq!(DenyBroker.resolve_ask("ask").await, ApprovalDecision::Deny);
    }
}
