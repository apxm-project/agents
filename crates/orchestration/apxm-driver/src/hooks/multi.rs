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
            child.set_current_span_id(span_id.clone())
        });
    }

    fn current_span_id(&self) -> Option<String> {
        self.first_some("current_span_id", |child| child.current_span_id())
    }

    fn set_current_scope_id(&self, scope_id: Option<String>) {
        self.for_each("set_current_scope_id", |child| {
            child.set_current_scope_id(scope_id.clone())
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
            child.emit_graph_start(execution_id, node_count)
        });
    }

    fn emit_graph_end(&self, execution_id: &str, node_count: usize, success: bool) {
        self.for_each("emit_graph_end", |child| {
            child.emit_graph_end(execution_id, node_count, success)
        });
    }

    fn emit_operation_start(&self, node_id: u64, op_type: AISOperationType) {
        self.for_each("emit_operation_start", |child| {
            child.emit_operation_start(node_id, op_type)
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
            child.emit_operation_end(
                node_id,
                op_type,
                duration,
                success,
                tokens.clone(),
                timing.clone(),
            )
        });
    }

    fn emit_node_output(&self, node_id: u64, value: &Value) {
        self.for_each("emit_node_output", |child| {
            child.emit_node_output(node_id, value)
        });
    }

    fn emit_llm_prompt(&self, node_id: u64, prompt: &str) {
        self.for_each("emit_llm_prompt", |child| {
            child.emit_llm_prompt(node_id, prompt)
        });
    }

    fn emit_llm_token_for_node(&self, node_id: u64, content: &str) {
        self.for_each("emit_llm_token_for_node", |child| {
            child.emit_llm_token_for_node(node_id, content)
        });
    }

    fn emit_plan_created(&self, plan_id: &str, steps: usize) {
        self.for_each("emit_plan_created", |child| {
            child.emit_plan_created(plan_id, steps)
        });
    }

    fn emit_plan_step_started(&self, plan_id: &str, step_index: usize) {
        self.for_each("emit_plan_step_started", |child| {
            child.emit_plan_step_started(plan_id, step_index)
        });
    }

    fn emit_plan_step_completed(&self, plan_id: &str, step_index: usize, success: bool) {
        self.for_each("emit_plan_step_completed", |child| {
            child.emit_plan_step_completed(plan_id, step_index, success)
        });
    }

    fn emit_workflow_started(&self, workflow_name: &str, session_dir: &str, step_count: usize) {
        self.for_each("emit_workflow_started", |child| {
            child.emit_workflow_started(workflow_name, session_dir, step_count)
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
            )
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
            )
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
            )
        });
    }

    fn emit_memory_read(&self, scope: &str, key: &str) {
        self.for_each("emit_memory_read", |child| {
            child.emit_memory_read(scope, key)
        });
    }

    fn emit_memory_write(&self, scope: &str, key: &str) {
        self.for_each("emit_memory_write", |child| {
            child.emit_memory_write(scope, key)
        });
    }

    fn emit_checkpoint_saved(&self, checkpoint_id: &str) {
        self.for_each("emit_checkpoint_saved", |child| {
            child.emit_checkpoint_saved(checkpoint_id)
        });
    }

    fn emit_checkpoint_restored(&self, checkpoint_id: &str) {
        self.for_each("emit_checkpoint_restored", |child| {
            child.emit_checkpoint_restored(checkpoint_id)
        });
    }

    fn emit_scheduler_decision(&self, node_id: u64, delay: Duration, reason: &str) {
        self.for_each("emit_scheduler_decision", |child| {
            child.emit_scheduler_decision(node_id, delay, reason)
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
            child.emit_head_of_line_block(blocker_node, blocked_node, wait_ms, reason)
        });
    }

    fn emit_gpu_utilization(&self, gpu_id: u32, utilization_pct: f32, memory_pct: f32) {
        self.for_each("emit_gpu_utilization", |child| {
            child.emit_gpu_utilization(gpu_id, utilization_pct, memory_pct)
        });
    }

    fn emit_token_usage(&self, node_id: u64, input_tokens: usize, output_tokens: usize) {
        self.for_each("emit_token_usage", |child| {
            child.emit_token_usage(node_id, input_tokens, output_tokens)
        });
    }

    fn emit_memoization_hit(&self, node_id: u64) {
        self.for_each("emit_memoization_hit", |child| {
            child.emit_memoization_hit(node_id)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    struct RecordingEmitter {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingEmitter {
        fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
            Self { events }
        }
    }

    impl ExecutionEventEmitter for RecordingEmitter {
        fn emit_llm_token(&self, _content: &str) {}

        fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}

        fn emit_tool_end(&self, _name: &str, _result: &Value) {}

        fn emit_graph_start(&self, execution_id: &str, node_count: usize) {
            self.events
                .lock()
                .push(format!("graph_start:{execution_id}:{node_count}"));
        }

        fn emit_operation_start(&self, node_id: u64, op_type: AISOperationType) {
            self.events
                .lock()
                .push(format!("operation_start:{node_id}:{op_type}"));
        }
    }

    struct PanickingEmitter;

    impl ExecutionEventEmitter for PanickingEmitter {
        fn emit_llm_token(&self, _content: &str) {}

        fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}

        fn emit_tool_end(&self, _name: &str, _result: &Value) {}

        fn emit_graph_start(&self, _execution_id: &str, _node_count: usize) {
            panic!("boom");
        }
    }

    #[test]
    fn panicking_child_does_not_break_siblings() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let emitter = MultiEmitter::new(vec![
            Arc::new(PanickingEmitter),
            Arc::new(RecordingEmitter::new(Arc::clone(&events))),
        ]);

        emitter.emit_graph_start("exec-1", 3);
        emitter.emit_operation_start(7, AISOperationType::Ask);

        assert_eq!(
            events.lock().clone(),
            vec![
                "graph_start:exec-1:3".to_string(),
                "operation_start:7:ASK".to_string()
            ]
        );
    }
}
