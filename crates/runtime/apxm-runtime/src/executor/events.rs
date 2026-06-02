//! Execution event emission hooks.

use std::collections::HashMap;
use std::time::Duration;

use apxm_core::types::NodeMetrics;
use apxm_core::types::TimingBreakdown;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use serde::{Deserialize, Serialize};

use crate::executor::token_accounting::TokenUsageSummary;

/// Runtime execution event enum for simple emitters and tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionEvent {
    // ── Existing (3) ────────────────────────────────────────────────
    LlmToken {
        content: String,
    },
    ToolStart {
        name: String,
        args: HashMap<String, Value>,
    },
    ToolEnd {
        name: String,
        result: Value,
    },

    // ── Operation lifecycle ─────────────────────────────────────────
    OperationStart {
        node_id: u64,
        op_type: AISOperationType,
    },
    OperationEnd {
        node_id: u64,
        op_type: AISOperationType,
        #[serde(with = "duration_millis")]
        duration: Duration,
        success: bool,
    },

    // ── Planning events ─────────────────────────────────────────────
    PlanCreated {
        plan_id: String,
        steps: usize,
    },
    PlanStepStarted {
        plan_id: String,
        step_index: usize,
    },
    PlanStepCompleted {
        plan_id: String,
        step_index: usize,
        success: bool,
    },
    PlanGraphEmitted {
        plan_id: String,
        generating_model: String,
        node_count: usize,
        task_ids: Vec<u64>,
        parallel_fanout_max: usize,
    },

    // ── Memory events ───────────────────────────────────────────────
    MemoryRead {
        scope: String,
        key: String,
    },
    MemoryWrite {
        scope: String,
        key: String,
    },

    // ── Checkpoint events ───────────────────────────────────────────
    CheckpointSaved {
        checkpoint_id: String,
    },
    CheckpointRestored {
        checkpoint_id: String,
    },

    // ── Scheduler / timing events ───────────────────────────────────
    SchedulerDecision {
        node_id: u64,
        #[serde(with = "duration_millis")]
        delay: Duration,
        reason: String,
    },

    // ── Hardware events ───────────────────────────────────────────────
    GpuUtilization {
        gpu_id: u32,
        utilization_pct: f32,
        memory_pct: f32,
    },

    // ── Token accounting events ────────────────────────────────────
    TokenUsage {
        node_id: u64,
        input_tokens: usize,
        output_tokens: usize,
    },
    // ── Memoization events ─────────────────────────────────────────
    MemoizationHit {
        node_id: u64,
    },
}

/// Serde helper: serialize/deserialize `Duration` as integer milliseconds.
mod duration_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

/// Optional observer for execution events.
///
/// Implementors receive fine-grained lifecycle callbacks during DAG
/// execution.  Every method has a default no-op implementation so
/// consumers only need to override the events they care about.
pub trait ExecutionEventEmitter: Send + Sync {
    // ── Span hierarchy ─────────────────────────────────────────────
    /// Set the current parent span ID for subsequently emitted events.
    fn set_current_span_id(&self, _span_id: Option<String>) {}

    /// Get the current parent span ID.
    fn current_span_id(&self) -> Option<String> {
        None
    }

    // ── Scope isolation ───────────────────────────────────────────
    /// Set the current scope ID for subsequently emitted events.
    fn set_current_scope_id(&self, _scope_id: Option<String>) {}

    /// Get the current scope ID.
    fn current_scope_id(&self) -> Option<String> {
        None
    }

    // ── Existing ────────────────────────────────────────────────────
    fn emit_llm_token(&self, content: &str);
    /// Emit an extended-thinking ("reasoning") delta as a distinct `thought`
    /// event, kept separate from answer `token`s so clients can render it apart
    /// (the CLI dims it; the studio shows a collapsible thinking block).
    fn emit_llm_thought(&self, _content: &str) {}
    fn emit_tool_start(&self, name: &str, args: &HashMap<String, Value>);
    fn emit_tool_end(&self, name: &str, result: &Value);

    // ── Graph lifecycle ─────────────────────────────────────────────
    fn emit_graph_start(&self, _execution_id: &str, _node_count: usize) {}
    fn emit_graph_end(&self, _execution_id: &str, _node_count: usize, _success: bool) {}

    // ── Operation lifecycle ─────────────────────────────────────────
    fn emit_operation_start(&self, _node_id: u64, _op_type: AISOperationType) {}
    /// Like `emit_operation_start` but with op-specific context
    /// (target_agent for COMMUNICATE, tool_names+model for ASK,
    /// agent_code for SPAWN_AGENT). Default no-ops so out-of-tree
    /// emitters don't have to implement it.
    fn emit_operation_start_with_context(
        &self,
        node_id: u64,
        op_type: AISOperationType,
        _context: serde_json::Value,
    ) {
        // Default routes back to the plain emit so emitters that don't
        // care about context still see the lifecycle event.
        self.emit_operation_start(node_id, op_type);
    }

    // ── Multi-agent / topology ──────────────────────────────────────
    /// A SPAWN_AGENT op produced a new agent record.
    fn emit_agent_spawned(
        &self,
        _node_id: u64,
        _agent_code: &str,
        _parent_execution_id: &str,
        _profile: Option<&str>,
        _process_id: Option<&str>,
        _scope_policy: Option<&str>,
    ) {
    }

    /// A COMMUNICATE op dispatched to a target.
    fn emit_communicate_dispatched(
        &self,
        _node_id: u64,
        _target_agent: &str,
        _protocol: &str,
        _message_excerpt: Option<&str>,
    ) {
    }

    /// A topology edge resolved at runtime.
    fn emit_graph_edge(&self, _from_node_id: u64, _to_node_id: u64, _kind: &str) {}

    fn emit_operation_end(
        &self,
        _node_id: u64,
        _op_type: AISOperationType,
        _duration: Duration,
        _success: bool,
        _tokens: Option<TokenUsageSummary>,
        _timing: Option<TimingBreakdown>,
    ) {
    }
    fn emit_node_output(&self, _node_id: u64, _value: &Value) {}
    fn emit_node_output_with_name(&self, node_id: u64, _node_name: Option<&str>, value: &Value) {
        self.emit_node_output(node_id, value);
    }
    fn emit_node_metrics(&self, _node_id: u64, _metrics: &NodeMetrics) {}
    fn emit_node_metrics_with_name(
        &self,
        node_id: u64,
        _node_name: Option<&str>,
        metrics: &NodeMetrics,
    ) {
        self.emit_node_metrics(node_id, metrics);
    }
    fn emit_llm_prompt(&self, _node_id: u64, _prompt: &str) {}
    fn emit_llm_prompt_with_name(&self, node_id: u64, _node_name: Option<&str>, prompt: &str) {
        self.emit_llm_prompt(node_id, prompt);
    }
    fn emit_llm_token_for_node(&self, _node_id: u64, content: &str) {
        self.emit_llm_token(content);
    }
    fn emit_llm_thought_for_node(&self, _node_id: u64, content: &str) {
        self.emit_llm_thought(content);
    }

    // ── Planning ────────────────────────────────────────────────────
    fn emit_plan_created(&self, _plan_id: &str, _steps: usize) {}
    fn emit_plan_step_started(&self, _plan_id: &str, _step_index: usize) {}
    fn emit_plan_step_completed(&self, _plan_id: &str, _step_index: usize, _success: bool) {}
    fn emit_plan_graph_emitted(
        &self,
        _plan_id: &str,
        _generating_model: &str,
        _node_count: usize,
        _task_ids: &[u64],
        _parallel_fanout_max: usize,
    ) {
    }

    // ── Memory ──────────────────────────────────────────────────────
    fn emit_memory_read(&self, _scope: &str, _key: &str) {}
    fn emit_memory_write(&self, _scope: &str, _key: &str) {}

    // ── Checkpoints ─────────────────────────────────────────────────
    fn emit_checkpoint_saved(&self, _checkpoint_id: &str) {}
    fn emit_checkpoint_restored(&self, _checkpoint_id: &str) {}

    // ── Scheduler ───────────────────────────────────────────────────
    fn emit_scheduler_decision(&self, _node_id: u64, _delay: Duration, _reason: &str) {}
    fn emit_head_of_line_block(
        &self,
        _blocker_node: u64,
        _blocked_node: u64,
        _wait_ms: u64,
        _reason: &str,
    ) {
    }

    // ── Hardware ────────────────────────────────────────────────────
    fn emit_gpu_utilization(&self, _gpu_id: u32, _utilization_pct: f32, _memory_pct: f32) {}

    // ── Token accounting ──────────────────────────────────────────
    fn emit_token_usage(&self, _node_id: u64, _input_tokens: usize, _output_tokens: usize) {}

    // ── Memoization ───────────────────────────────────────────────
    fn emit_memoization_hit(&self, _node_id: u64) {}

    // ── Layer 2 — agent-layer hooks ────────────────────────────────
    //
    // All default to no-ops so out-of-tree emitters and existing tests
    // don't have to implement them. Concrete emitters (e.g. the
    // `EmitterAdapter` that bridges to `ApxmEvent`) should override
    // the ones they care about. See CLAUDE.md §10 for the pairing
    // rules between Layer 1 and Layer 2 events.

    /// The outermost executor entry began (top-level turn start).
    fn emit_turn_started(
        &self,
        _execution_id: &str,
        _turn_id: Option<&str>,
        _coordinator_label: Option<&str>,
    ) {
    }

    /// The outermost executor returned successfully.
    fn emit_turn_complete(&self, _execution_id: &str, _duration_ms: u64, _had_answer: bool) {}

    /// The outermost executor terminated abnormally.
    fn emit_turn_aborted(
        &self,
        _execution_id: &str,
        _duration_ms: u64,
        _reason: &str,
        _error_message_safe: Option<&str>,
    ) {
    }

    /// A SPAWN_AGENT node is opening a new sub-agent scope.
    fn emit_subagent_spawn_begin(
        &self,
        _agent_code: &str,
        _agent_name: Option<&str>,
        _agent_type: Option<&str>,
        _module_key: Option<&str>,
        _autonomy_policy: Option<&str>,
        _parent_span_id: Option<&str>,
    ) {
    }

    /// The new sub-agent scope is fully constructed.
    fn emit_subagent_spawn_end(&self, _agent_code: &str) {}

    /// An ASK node inside an agent scope began an LLM call.
    fn emit_subagent_llm_call_begin(
        &self,
        _agent_code: &str,
        _model: &str,
        _backend: &str,
        _tool_manifest_count: usize,
    ) {
    }

    /// An ASK node inside an agent scope returned a response.
    fn emit_subagent_llm_call_end(
        &self,
        _agent_code: &str,
        _finish_reason: &str,
        _input_tokens: usize,
        _output_tokens: usize,
        _content_len: usize,
    ) {
    }

    /// An INV_TOOL node inside an agent scope began a tool call.
    fn emit_tool_call_begin(&self, _agent_code: &str, _tool_name: &str, _argument_keys: &[String]) {
    }

    /// An INV_TOOL node inside an agent scope returned a result.
    fn emit_tool_call_end(
        &self,
        _agent_code: &str,
        _tool_name: &str,
        _result_keys: &[String],
        _status: &str,
        _latency_ms: u64,
    ) {
    }

    /// A sub-agent scope exited cleanly.
    fn emit_subagent_done(
        &self,
        _agent_code: &str,
        _total_tool_calls: usize,
        _input_tokens_total: usize,
        _output_tokens_total: usize,
        _evidence_excerpt: Option<&str>,
    ) {
    }

    /// A sub-agent scope exited with an error.
    fn emit_subagent_failed(
        &self,
        _agent_code: &str,
        _error_class: &str,
        _error_message_safe: &str,
    ) {
    }

    /// The coordinator emitted a final answer payload.
    fn emit_agent_message(
        &self,
        _text: &str,
        _item_id: Option<&str>,
        _response_id: Option<&str>,
        _input_tokens: Option<usize>,
        _output_tokens: Option<usize>,
    ) {
    }

    /// A human-in-the-loop approval gate was opened.
    fn emit_approval_request(
        &self,
        _agent_code: &str,
        _tool_name: &str,
        _approval_id: &str,
        _risk_level: &str,
    ) {
    }

    /// A previously-requested approval was resolved.
    fn emit_approval_resolved(&self, _approval_id: &str, _decision: &str) {}
}
