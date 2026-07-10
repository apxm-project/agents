use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use apxm_core::types::TimingBreakdown;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use apxm_runtime::{ExecutionEventEmitter, TokenUsageSummary};

pub struct MultiEmitter {
    children: Vec<Arc<dyn ExecutionEventEmitter>>,
}

impl MultiEmitter {
    pub fn new(children: Vec<Arc<dyn ExecutionEventEmitter>>) -> Self {
        Self { children }
    }

    fn for_each(&self, event_name: &'static str, mut f: impl FnMut(&dyn ExecutionEventEmitter)) {
        for child in &self.children {
            if catch_unwind(AssertUnwindSafe(|| f(child.as_ref()))).is_err() {
                tracing::warn!(
                    event = event_name,
                    "Execution hook emitter panicked; continuing"
                );
            }
        }
    }

    fn first_some<T>(
        &self,
        event_name: &'static str,
        mut f: impl FnMut(&dyn ExecutionEventEmitter) -> Option<T>,
    ) -> Option<T> {
        for child in &self.children {
            match catch_unwind(AssertUnwindSafe(|| f(child.as_ref()))) {
                Ok(Some(value)) => return Some(value),
                Ok(None) => {}
                Err(_) => {
                    tracing::warn!(
                        event = event_name,
                        "Execution hook emitter panicked while reading state; continuing"
                    );
                }
            }
        }
        None
    }
}

impl ExecutionEventEmitter for MultiEmitter {
    fn set_current_span_id(&self, span_id: Option<String>) {
        self.for_each("set_current_span_id", |child| {
            child.set_current_span_id(span_id.clone());
        });
    }

    fn current_span_id(&self) -> Option<String> {
        self.first_some("current_span_id", |child| child.current_span_id())
    }

    fn set_current_scope_id(&self, scope_id: Option<String>) {
        self.for_each("set_current_scope_id", |child| {
            child.set_current_scope_id(scope_id.clone());
        });
    }

    fn current_scope_id(&self) -> Option<String> {
        self.first_some("current_scope_id", |child| child.current_scope_id())
    }

    fn emit_llm_token(&self, content: &str) {
        self.for_each("emit_llm_token", |child| child.emit_llm_token(content));
    }

    fn emit_tool_start(&self, name: &str, args: &HashMap<String, Value>) {
        self.for_each("emit_tool_start", |child| child.emit_tool_start(name, args));
    }

    fn emit_tool_end(&self, name: &str, result: &Value) {
        self.for_each("emit_tool_end", |child| child.emit_tool_end(name, result));
    }

    fn emit_graph_start(&self, execution_id: &str, node_count: usize) {
        self.for_each("emit_graph_start", |child| {
            child.emit_graph_start(execution_id, node_count);
        });
    }

    fn emit_graph_end(&self, execution_id: &str, node_count: usize, success: bool) {
        self.for_each("emit_graph_end", |child| {
            child.emit_graph_end(execution_id, node_count, success);
        });
    }

    fn emit_operation_start(&self, node_id: u64, op_type: AISOperationType) {
        self.for_each("emit_operation_start", |child| {
            child.emit_operation_start(node_id, op_type);
        });
    }

    fn emit_operation_end(
        &self,
        node_id: u64,
        op_type: AISOperationType,
        duration: Duration,
        success: bool,
        tokens: Option<TokenUsageSummary>,
        timing: Option<TimingBreakdown>,
    ) {
        self.for_each("emit_operation_end", |child| {
            child.emit_operation_end(node_id, op_type, duration, success, tokens.clone(), timing);
        });
    }

    fn emit_node_output(&self, node_id: u64, value: &Value) {
        self.for_each("emit_node_output", |child| {
            child.emit_node_output(node_id, value);
        });
    }

    fn emit_llm_prompt(&self, node_id: u64, prompt: &str) {
        self.for_each("emit_llm_prompt", |child| {
            child.emit_llm_prompt(node_id, prompt);
        });
    }

    fn emit_llm_token_for_node(&self, node_id: u64, content: &str) {
        self.for_each("emit_llm_token_for_node", |child| {
            child.emit_llm_token_for_node(node_id, content);
        });
    }

    fn emit_plan_created(&self, plan_id: &str, steps: usize) {
        self.for_each("emit_plan_created", |child| {
            child.emit_plan_created(plan_id, steps);
        });
    }

    fn emit_plan_step_started(&self, plan_id: &str, step_index: usize) {
        self.for_each("emit_plan_step_started", |child| {
            child.emit_plan_step_started(plan_id, step_index);
        });
    }

    fn emit_plan_step_completed(&self, plan_id: &str, step_index: usize, success: bool) {
        self.for_each("emit_plan_step_completed", |child| {
            child.emit_plan_step_completed(plan_id, step_index, success);
        });
    }

    fn emit_workflow_started(&self, workflow_name: &str, session_dir: &str, step_count: usize) {
        self.for_each("emit_workflow_started", |child| {
            child.emit_workflow_started(workflow_name, session_dir, step_count);
        });
    }

    fn emit_workflow_step_started(
        &self,
        workflow_name: &str,
        workflow_session_dir: &str,
        step_id: &str,
        step_index: usize,
        step_count: usize,
    ) {
        self.for_each("emit_workflow_step_started", |child| {
            child.emit_workflow_step_started(
                workflow_name,
                workflow_session_dir,
                step_id,
                step_index,
                step_count,
            );
        });
    }

    fn emit_workflow_step_completed(
        &self,
        workflow_name: &str,
        workflow_session_dir: &str,
        step_id: &str,
        step_index: usize,
        status: &str,
        success: bool,
        duration: Duration,
        session_dir: Option<&str>,
        error: Option<&str>,
    ) {
        self.for_each("emit_workflow_step_completed", |child| {
            child.emit_workflow_step_completed(
                workflow_name,
                workflow_session_dir,
                step_id,
                step_index,
                status,
                success,
                duration,
                session_dir,
                error,
            );
        });
    }

    fn emit_workflow_finished(
        &self,
        workflow_name: &str,
        session_dir: &str,
        status: &str,
        success: bool,
        duration: Duration,
        step_count: usize,
    ) {
        self.for_each("emit_workflow_finished", |child| {
            child.emit_workflow_finished(
                workflow_name,
                session_dir,
                status,
                success,
                duration,
                step_count,
            );
        });
    }

    fn emit_memory_read(&self, scope: &str, key: &str) {
        self.for_each("emit_memory_read", |child| {
            child.emit_memory_read(scope, key);
        });
    }

    fn emit_memory_write(&self, scope: &str, key: &str) {
        self.for_each("emit_memory_write", |child| {
            child.emit_memory_write(scope, key);
        });
    }

    fn emit_checkpoint_saved(&self, checkpoint_id: &str) {
        self.for_each("emit_checkpoint_saved", |child| {
            child.emit_checkpoint_saved(checkpoint_id);
        });
    }

    fn emit_checkpoint_restored(&self, checkpoint_id: &str) {
        self.for_each("emit_checkpoint_restored", |child| {
            child.emit_checkpoint_restored(checkpoint_id);
        });
    }

    fn emit_scheduler_decision(&self, node_id: u64, delay: Duration, reason: &str) {
        self.for_each("emit_scheduler_decision", |child| {
            child.emit_scheduler_decision(node_id, delay, reason);
        });
    }

    fn emit_head_of_line_block(
        &self,
        blocker_node: u64,
        blocked_node: u64,
        wait_ms: u64,
        reason: &str,
    ) {
        self.for_each("emit_head_of_line_block", |child| {
            child.emit_head_of_line_block(blocker_node, blocked_node, wait_ms, reason);
        });
    }

    fn emit_gpu_utilization(&self, gpu_id: u32, utilization_pct: f32, memory_pct: f32) {
        self.for_each("emit_gpu_utilization", |child| {
            child.emit_gpu_utilization(gpu_id, utilization_pct, memory_pct);
        });
    }

    fn emit_token_usage(&self, node_id: u64, input_tokens: usize, output_tokens: usize) {
        self.for_each("emit_token_usage", |child| {
            child.emit_token_usage(node_id, input_tokens, output_tokens);
        });
    }

    fn emit_memoization_hit(&self, node_id: u64) {
        self.for_each("emit_memoization_hit", |child| {
            child.emit_memoization_hit(node_id);
        });
    }

    // ── Layer 2 — agent-layer hooks ────────────────────────────────
    //
    // `MultiEmitter` fans out to every child via `for_each`, so a fixed
    // child (e.g. `EmitterAdapter`) composed through `compose_emitters`
    // would still be silently swallowed if these stayed at the trait's
    // no-op default — the fan-out regression `multi_emitter_forwards_
    // layer2_calls_to_children` pins below.

    fn emit_turn_started(
        &self,
        execution_id: &str,
        turn_id: Option<&str>,
        coordinator_label: Option<&str>,
    ) {
        self.for_each("emit_turn_started", |child| {
            child.emit_turn_started(execution_id, turn_id, coordinator_label);
        });
    }

    fn emit_turn_complete(&self, execution_id: &str, duration_ms: u64, had_answer: bool) {
        self.for_each("emit_turn_complete", |child| {
            child.emit_turn_complete(execution_id, duration_ms, had_answer);
        });
    }

    fn emit_turn_aborted(
        &self,
        execution_id: &str,
        duration_ms: u64,
        reason: &str,
        error_message_safe: Option<&str>,
    ) {
        self.for_each("emit_turn_aborted", |child| {
            child.emit_turn_aborted(execution_id, duration_ms, reason, error_message_safe);
        });
    }

    fn emit_subagent_spawn_begin(
        &self,
        agent_code: &str,
        agent_name: Option<&str>,
        agent_type: Option<&str>,
        module_key: Option<&str>,
        autonomy_policy: Option<&str>,
        parent_span_id: Option<&str>,
    ) {
        self.for_each("emit_subagent_spawn_begin", |child| {
            child.emit_subagent_spawn_begin(
                agent_code,
                agent_name,
                agent_type,
                module_key,
                autonomy_policy,
                parent_span_id,
            );
        });
    }

    fn emit_subagent_spawn_end(&self, agent_code: &str) {
        self.for_each("emit_subagent_spawn_end", |child| {
            child.emit_subagent_spawn_end(agent_code);
        });
    }

    fn emit_subagent_llm_call_begin(
        &self,
        agent_code: &str,
        model: &str,
        backend: &str,
        tool_manifest_count: usize,
    ) {
        self.for_each("emit_subagent_llm_call_begin", |child| {
            child.emit_subagent_llm_call_begin(agent_code, model, backend, tool_manifest_count);
        });
    }

    fn emit_subagent_llm_call_end(
        &self,
        agent_code: &str,
        finish_reason: &str,
        input_tokens: usize,
        output_tokens: usize,
        content_len: usize,
    ) {
        self.for_each("emit_subagent_llm_call_end", |child| {
            child.emit_subagent_llm_call_end(
                agent_code,
                finish_reason,
                input_tokens,
                output_tokens,
                content_len,
            );
        });
    }

    fn emit_tool_call_begin(&self, agent_code: &str, tool_name: &str, argument_keys: &[String]) {
        self.for_each("emit_tool_call_begin", |child| {
            child.emit_tool_call_begin(agent_code, tool_name, argument_keys);
        });
    }

    fn emit_tool_call_end(
        &self,
        agent_code: &str,
        tool_name: &str,
        result_keys: &[String],
        status: &str,
        latency_ms: u64,
    ) {
        self.for_each("emit_tool_call_end", |child| {
            child.emit_tool_call_end(agent_code, tool_name, result_keys, status, latency_ms);
        });
    }

    fn emit_subagent_done(
        &self,
        agent_code: &str,
        total_tool_calls: usize,
        input_tokens_total: usize,
        output_tokens_total: usize,
        evidence_excerpt: Option<&str>,
    ) {
        self.for_each("emit_subagent_done", |child| {
            child.emit_subagent_done(
                agent_code,
                total_tool_calls,
                input_tokens_total,
                output_tokens_total,
                evidence_excerpt,
            );
        });
    }

    fn emit_subagent_failed(&self, agent_code: &str, error_class: &str, error_message_safe: &str) {
        self.for_each("emit_subagent_failed", |child| {
            child.emit_subagent_failed(agent_code, error_class, error_message_safe);
        });
    }

    fn emit_agent_message(
        &self,
        text: &str,
        item_id: Option<&str>,
        response_id: Option<&str>,
        input_tokens: Option<usize>,
        output_tokens: Option<usize>,
    ) {
        self.for_each("emit_agent_message", |child| {
            child.emit_agent_message(text, item_id, response_id, input_tokens, output_tokens);
        });
    }

    fn emit_approval_request(
        &self,
        agent_code: &str,
        tool_name: &str,
        approval_id: &str,
        risk_level: &str,
    ) {
        self.for_each("emit_approval_request", |child| {
            child.emit_approval_request(agent_code, tool_name, approval_id, risk_level);
        });
    }

    fn emit_approval_resolved(&self, approval_id: &str, decision: &str) {
        self.for_each("emit_approval_resolved", |child| {
            child.emit_approval_resolved(approval_id, decision);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex as PlMutex;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    /// Records every Layer-2 (and approval) call it receives, by wire
    /// kind name, so the fan-out test can assert both children saw the
    /// same sequence of calls.
    #[derive(Default)]
    struct RecordingEmitter {
        calls: PlMutex<Vec<&'static str>>,
        span_id: PlMutex<Option<String>>,
        seq: AtomicUsize,
    }

    impl RecordingEmitter {
        fn record(&self, name: &'static str) {
            self.calls.lock().push(name);
            self.seq.fetch_add(1, AtomicOrdering::Relaxed);
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().clone()
        }
    }

    impl ExecutionEventEmitter for RecordingEmitter {
        fn set_current_span_id(&self, span_id: Option<String>) {
            *self.span_id.lock() = span_id;
        }

        fn current_span_id(&self) -> Option<String> {
            self.span_id.lock().clone()
        }

        fn emit_llm_token(&self, _content: &str) {}

        fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}

        fn emit_tool_end(&self, _name: &str, _result: &Value) {}

        fn emit_turn_started(
            &self,
            _execution_id: &str,
            _turn_id: Option<&str>,
            _coordinator_label: Option<&str>,
        ) {
            self.record("turn_started");
        }

        fn emit_turn_complete(&self, _execution_id: &str, _duration_ms: u64, _had_answer: bool) {
            self.record("turn_complete");
        }

        fn emit_subagent_spawn_begin(
            &self,
            _agent_code: &str,
            _agent_name: Option<&str>,
            _agent_type: Option<&str>,
            _module_key: Option<&str>,
            _autonomy_policy: Option<&str>,
            _parent_span_id: Option<&str>,
        ) {
            self.record("subagent_spawn_begin");
        }

        fn emit_subagent_spawn_end(&self, _agent_code: &str) {
            self.record("subagent_spawn_end");
        }

        fn emit_tool_call_begin(
            &self,
            _agent_code: &str,
            _tool_name: &str,
            _argument_keys: &[String],
        ) {
            self.record("tool_call_begin");
        }

        fn emit_tool_call_end(
            &self,
            _agent_code: &str,
            _tool_name: &str,
            _result_keys: &[String],
            _status: &str,
            _latency_ms: u64,
        ) {
            self.record("tool_call_end");
        }

        fn emit_agent_message(
            &self,
            _text: &str,
            _item_id: Option<&str>,
            _response_id: Option<&str>,
            _input_tokens: Option<usize>,
            _output_tokens: Option<usize>,
        ) {
            self.record("agent_message");
        }

        fn emit_approval_request(
            &self,
            _agent_code: &str,
            _tool_name: &str,
            _approval_id: &str,
            _risk_level: &str,
        ) {
            self.record("approval_request");
        }

        fn emit_approval_resolved(&self, _approval_id: &str, _decision: &str) {
            self.record("approval_resolved");
        }
    }

    /// Fan-out regression pin: two recording children behind
    /// `compose_emitters`/`MultiEmitter` must **both** receive every
    /// Layer-2 call. Before this WP, `MultiEmitter` had no Layer-2
    /// overrides, so this test would have failed with both children
    /// recording nothing at all — the "fixed `EmitterAdapter` alone
    /// looks complete while `MultiEmitter` stays silently broken" trap
    /// required by the event-delivery contract.
    #[test]
    fn multi_emitter_forwards_layer2_calls_to_children() {
        let child_a = Arc::new(RecordingEmitter::default());
        let child_b = Arc::new(RecordingEmitter::default());
        let multi = MultiEmitter::new(vec![
            child_a.clone() as Arc<dyn ExecutionEventEmitter>,
            child_b.clone() as Arc<dyn ExecutionEventEmitter>,
        ]);

        multi.emit_turn_started("exec-1", Some("turn-1"), Some("Cleo"));
        multi.emit_turn_complete("exec-1", 100, true);
        multi.emit_subagent_spawn_begin("agent-1", Some("Agent One"), None, None, None, None);
        multi.emit_subagent_spawn_end("agent-1");
        multi.emit_tool_call_begin("agent-1", "web_search", &["q".to_string()]);
        multi.emit_tool_call_end("agent-1", "web_search", &["r".to_string()], "ok", 12);
        multi.emit_agent_message("final answer", None, None, Some(1), Some(2));

        let expected = vec![
            "turn_started",
            "turn_complete",
            "subagent_spawn_begin",
            "subagent_spawn_end",
            "tool_call_begin",
            "tool_call_end",
            "agent_message",
        ];
        assert_eq!(
            child_a.calls(),
            expected,
            "child A must see every Layer-2 call"
        );
        assert_eq!(
            child_b.calls(),
            expected,
            "child B must see every Layer-2 call"
        );
    }
}
