//! Internal scheduler state types.
//!
//! These types are used internally by the scheduler for tracking operation
//! and token state. They are not part of the public API.

use std::time::Instant;

use crate::aam::effects::OperationEffects;
use apxm_core::types::{NodeId, OpStatus, Value};

/// Internal operation state tracked during execution.
///
/// This is used by the scheduler to track the status of each operation
/// as it moves through the execution pipeline.
#[derive(Debug)]
pub(crate) struct OpState {
    /// Current execution status.
    pub status: OpStatus,
    /// Number of retry attempts made.
    pub retries: u32,
    /// Last error message (if any).
    pub last_error: Option<String>,
    /// Time when execution started.
    pub started_at: Option<Instant>,
    /// Time when execution finished.
    pub finished_at: Option<Instant>,
    /// Operation effect metadata
    pub effects: OperationEffects,
}

impl OpState {
    /// Create a new operation state in Pending status.
    pub fn new() -> Self {
        Self::new_with_effects(OperationEffects::new())
    }

    pub fn new_with_effects(effects: OperationEffects) -> Self {
        Self {
            status: OpStatus::Pending,
            retries: 0,
            last_error: None,
            started_at: None,
            finished_at: None,
            effects,
        }
    }

    pub fn effects(&self) -> &OperationEffects {
        &self.effects
    }
}

impl Default for OpState {
    fn default() -> Self {
        Self::new()
    }
}

/// Internal token state tracked during execution.
///
/// Tokens represent data dependencies in the dataflow graph. This type
/// tracks whether a token is ready and what value it holds.
///
/// Note: Token delegation (for switch/case sub-DAG splicing) is tracked
/// separately in `SchedulerState::delegated_tokens` to avoid per-token overhead.
#[derive(Debug)]
pub(crate) struct TokenState {
    /// Whether the token value is ready.
    pub ready: bool,
    /// The token's value (if ready).
    pub value: Option<Value>,
    /// List of downstream consumers waiting for this token.
    pub consumers: Vec<NodeId>,
}

impl TokenState {
    /// Create a new token state (not ready).
    pub fn new() -> Self {
        Self {
            ready: false,
            value: None,
            consumers: Vec::new(),
        }
    }
}

impl Default for TokenState {
    fn default() -> Self {
        Self::new()
    }
}

/// Promise state for tracking flow call results.
///
/// When a flow call operation executes, it creates a promise token that
/// will be resolved when the sub-flow completes.
#[derive(Debug, Clone)]
pub struct PromiseState {
    /// Target agent name.
    pub target_agent: String,
    /// Target flow name.
    pub target_flow: String,
    /// Time when the promise was created.
    pub created_at: Instant,
    /// Whether the promise has been resolved.
    pub resolved: bool,
    /// The resolved value (if resolved).
    pub value: Option<Value>,
}

impl PromiseState {
    /// Create a new unresolved promise.
    pub fn new(target_agent: String, target_flow: String) -> Self {
        Self {
            target_agent,
            target_flow,
            created_at: Instant::now(),
            resolved: false,
            value: None,
        }
    }
}

/// Execution frame for tracking recursive flow calls.
///
/// Each sub-flow execution pushes a frame onto the stack.
#[derive(Debug, Clone)]
pub struct ExecutionFrame {
    /// Unique execution ID for this frame.
    pub execution_id: String,
    /// The flow being executed.
    pub flow_name: String,
    /// The promise token that will be resolved when this flow completes.
    pub parent_promise: Option<apxm_core::types::TokenId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::Value;

    // ── OpState tests ──────────────────────────────────────────────────

    #[test]
    fn test_op_state_new_defaults() {
        let op = OpState::new();
        assert_eq!(op.status, OpStatus::Pending);
        assert_eq!(op.retries, 0);
        assert!(op.last_error.is_none());
        assert!(op.started_at.is_none());
        assert!(op.finished_at.is_none());
        assert!(!op.effects.has_side_effects);
    }

    #[test]
    fn test_op_state_default_matches_new() {
        let from_new = OpState::new();
        let from_default = OpState::default();
        assert_eq!(from_new.status, from_default.status);
        assert_eq!(from_new.retries, from_default.retries);
        assert_eq!(from_new.last_error, from_default.last_error);
    }

    #[test]
    fn test_op_state_new_with_effects() {
        use crate::aam::effects::{AamComponent, OperationEffects};
        let effects = OperationEffects::new()
            .read(AamComponent::Beliefs)
            .write(AamComponent::Goals);

        let op = OpState::new_with_effects(effects);
        assert_eq!(op.status, OpStatus::Pending);
        assert!(op.effects().reads.contains(&AamComponent::Beliefs));
        assert!(op.effects().writes.contains(&AamComponent::Goals));
        assert!(op.effects().has_side_effects);
    }

    #[test]
    fn test_op_state_transition_pending_to_running() {
        let mut op = OpState::new();
        assert_eq!(op.status, OpStatus::Pending);

        op.status = OpStatus::Running;
        op.started_at = Some(std::time::Instant::now());
        assert_eq!(op.status, OpStatus::Running);
        assert!(op.started_at.is_some());
    }

    #[test]
    fn test_op_state_transition_running_to_completed() {
        let mut op = OpState::new();
        op.status = OpStatus::Running;
        op.started_at = Some(std::time::Instant::now());

        op.status = OpStatus::Completed;
        op.finished_at = Some(std::time::Instant::now());
        assert_eq!(op.status, OpStatus::Completed);
        assert!(op.finished_at.is_some());
    }

    #[test]
    fn test_op_state_transition_running_to_failed() {
        let mut op = OpState::new();
        op.status = OpStatus::Running;
        op.started_at = Some(std::time::Instant::now());

        op.status = OpStatus::Failed;
        op.retries = 3;
        op.last_error = Some("timeout".to_string());
        op.finished_at = Some(std::time::Instant::now());

        assert_eq!(op.status, OpStatus::Failed);
        assert_eq!(op.retries, 3);
        assert_eq!(op.last_error.as_deref(), Some("timeout"));
    }

    #[test]
    fn test_op_state_duration_tracking() {
        let mut op = OpState::new();
        let start = std::time::Instant::now();
        op.started_at = Some(start);

        // Simulate some passage of time
        std::thread::sleep(std::time::Duration::from_millis(1));
        op.finished_at = Some(std::time::Instant::now());

        let duration = op
            .finished_at
            .unwrap()
            .duration_since(op.started_at.unwrap());
        assert!(duration.as_millis() >= 1);
    }

    // ── TokenState tests ───────────────────────────────────────────────

    #[test]
    fn test_token_state_new_defaults() {
        let ts = TokenState::new();
        assert!(!ts.ready);
        assert!(ts.value.is_none());
        assert!(ts.consumers.is_empty());
    }

    #[test]
    fn test_token_state_default_matches_new() {
        let from_new = TokenState::new();
        let from_default = TokenState::default();
        assert_eq!(from_new.ready, from_default.ready);
        assert_eq!(from_new.value.is_none(), from_default.value.is_none());
        assert_eq!(from_new.consumers.len(), from_default.consumers.len());
    }

    #[test]
    fn test_token_state_set_ready_with_value() {
        let mut ts = TokenState::new();
        ts.ready = true;
        ts.value = Some(Value::String("hello".to_string()));

        assert!(ts.ready);
        assert_eq!(ts.value, Some(Value::String("hello".to_string())));
    }

    #[test]
    fn test_token_state_add_consumers() {
        let mut ts = TokenState::new();
        ts.consumers.push(10);
        ts.consumers.push(20);
        ts.consumers.push(30);

        assert_eq!(ts.consumers.len(), 3);
        assert_eq!(ts.consumers, vec![10, 20, 30]);
    }

    #[test]
    fn test_token_state_null_value() {
        let mut ts = TokenState::new();
        ts.ready = true;
        ts.value = Some(Value::Null);

        assert!(ts.ready);
        assert_eq!(ts.value, Some(Value::Null));
    }

    // ── PromiseState tests ─────────────────────────────────────────────

    #[test]
    fn test_promise_state_new_defaults() {
        let ps = PromiseState::new("agent_a".to_string(), "flow_main".to_string());
        assert_eq!(ps.target_agent, "agent_a");
        assert_eq!(ps.target_flow, "flow_main");
        assert!(!ps.resolved);
        assert!(ps.value.is_none());
    }

    #[test]
    fn test_promise_state_resolve() {
        let mut ps = PromiseState::new("agent_a".to_string(), "flow_main".to_string());

        ps.resolved = true;
        ps.value = Some(Value::String("result".to_string()));

        assert!(ps.resolved);
        assert_eq!(ps.value, Some(Value::String("result".to_string())));
    }

    #[test]
    fn test_promise_state_created_at_is_recent() {
        let before = std::time::Instant::now();
        let ps = PromiseState::new("a".to_string(), "f".to_string());
        let after = std::time::Instant::now();

        assert!(ps.created_at >= before);
        assert!(ps.created_at <= after);
    }

    #[test]
    fn test_promise_state_clone() {
        let ps = PromiseState::new("agent".to_string(), "flow".to_string());
        let cloned = ps.clone();

        assert_eq!(cloned.target_agent, ps.target_agent);
        assert_eq!(cloned.target_flow, ps.target_flow);
        assert_eq!(cloned.resolved, ps.resolved);
    }

    // ── ExecutionFrame tests ───────────────────────────────────────────

    #[test]
    fn test_execution_frame_without_parent_promise() {
        let frame = ExecutionFrame {
            execution_id: "exec-001".to_string(),
            flow_name: "main".to_string(),
            parent_promise: None,
        };

        assert_eq!(frame.execution_id, "exec-001");
        assert_eq!(frame.flow_name, "main");
        assert!(frame.parent_promise.is_none());
    }

    #[test]
    fn test_execution_frame_with_parent_promise() {
        let frame = ExecutionFrame {
            execution_id: "exec-002".to_string(),
            flow_name: "sub_flow".to_string(),
            parent_promise: Some(42),
        };

        assert_eq!(frame.parent_promise, Some(42));
    }

    #[test]
    fn test_execution_frame_clone() {
        let frame = ExecutionFrame {
            execution_id: "exec-003".to_string(),
            flow_name: "handler".to_string(),
            parent_promise: Some(99),
        };
        let cloned = frame.clone();

        assert_eq!(cloned.execution_id, frame.execution_id);
        assert_eq!(cloned.flow_name, frame.flow_name);
        assert_eq!(cloned.parent_promise, frame.parent_promise);
    }
}
