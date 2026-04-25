use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use apxm_core::events::kind;
use apxm_core::events::payload::{
    CheckpointRestoredPayload, CheckpointSavedPayload, GpuUtilizationPayload,
    HeadOfLineBlockPayload, MemoizationHitPayload, MemoryReadPayload, MemoryWritePayload,
    OperationEndPayload, OperationStartPayload, PlanCreatedPayload, PlanStepCompletedPayload,
    PlanStepStartedPayload, SchedulerDecisionPayload, TokenPayload, TokenUsagePayload,
    ToolEndPayload, ToolStartPayload,
};
use apxm_core::events::{ApxmEvent, EventEmitter, EventPayload, EventSource};
use apxm_core::types::TimingBreakdown;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use apxm_runtime::{EmitterAdapter, ExecutionEventEmitter, TokenUsageSummary};
use serde::Deserialize;

const TRACE_ID: &str = "trace-adapter";
const TOKEN_TEXT: &str = "token";
const NODE_TOKEN_TEXT: &str = "node-token";
const TOOL_NAME: &str = "mock-tool";
const TOOL_ARG: &str = "mock-arg";
const TOOL_RESULT: &str = "mock-result";
const PLAN_ID: &str = "mock-plan";
const MEMORY_SCOPE: &str = "mock-scope";
const MEMORY_KEY: &str = "mock-key";
const CHECKPOINT_ID: &str = "mock-checkpoint";
const SCHEDULER_REASON: &str = "dependency";
const HOL_REASON: &str = "resource";
const PARENT_SPAN_ID: &str = "parent-span";
const SCOPE_ID: &str = "scope-id";

#[derive(Default)]
struct CollectingEmitter {
    events: Mutex<Vec<ApxmEvent>>,
}

impl CollectingEmitter {
    fn events(&self) -> Vec<ApxmEvent> {
        self.events.lock().expect("events").clone()
    }
}

impl EventEmitter for CollectingEmitter {
    fn emit(&self, event: ApxmEvent) {
        self.events.lock().expect("events").push(event);
    }
}

#[derive(Default)]
struct RecordingExecutionEmitter {
    tokens: Mutex<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct SerializedEvent {
    payload: SerializedPayload,
}

#[derive(Debug, Deserialize)]
struct SerializedPayload {
    kind: String,
}

impl RecordingExecutionEmitter {
    fn tokens(&self) -> Vec<String> {
        self.tokens.lock().expect("tokens").clone()
    }
}

impl ExecutionEventEmitter for RecordingExecutionEmitter {
    fn emit_llm_token(&self, content: &str) {
        self.tokens
            .lock()
            .expect("tokens")
            .push(content.to_string());
    }

    fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}

    fn emit_tool_end(&self, _name: &str, _result: &Value) {}
}

fn collect_adapter() -> (Arc<CollectingEmitter>, EmitterAdapter) {
    let collector = Arc::new(CollectingEmitter::default());
    let adapter = EmitterAdapter::new(collector.clone(), EventSource::Runtime, TRACE_ID);
    (collector, adapter)
}

fn payload<T: EventPayload>(event: &ApxmEvent) -> &T {
    event
        .payload
        .downcast_ref::<T>()
        .expect("typed event payload")
}

fn assert_runtime_source(event: &ApxmEvent) {
    match &event.meta.source {
        EventSource::Runtime => {}
        other => panic!("expected runtime event source, got {other:?}"),
    }
}

#[test]
fn emitter_adapter_maps_supported_hooks_to_typed_event_kinds() {
    let (collector, adapter) = collect_adapter();
    let mut tool_args = HashMap::new();
    tool_args.insert(TOOL_ARG.to_string(), Value::String(TOOL_ARG.to_string()));

    adapter.emit_llm_token(TOKEN_TEXT);
    adapter.emit_llm_token_for_node(42, NODE_TOKEN_TEXT);
    adapter.emit_tool_start(TOOL_NAME, &tool_args);
    adapter.emit_tool_end(TOOL_NAME, &Value::String(TOOL_RESULT.to_string()));
    adapter.emit_operation_start(7, AISOperationType::Ask);
    adapter.emit_operation_end(
        7,
        AISOperationType::Ask,
        Duration::from_millis(13),
        true,
        Some(TokenUsageSummary {
            input_tokens: 3,
            output_tokens: 5,
            total_tokens: 8,
            call_count: 1,
        }),
        Some(TimingBreakdown {
            prefill_ms: 2.0,
            decode_ms: 4.0,
        }),
    );
    adapter.emit_plan_created(PLAN_ID, 2);
    adapter.emit_plan_step_started(PLAN_ID, 1);
    adapter.emit_plan_step_completed(PLAN_ID, 1, true);
    adapter.emit_memory_read(MEMORY_SCOPE, MEMORY_KEY);
    adapter.emit_memory_write(MEMORY_SCOPE, MEMORY_KEY);
    adapter.emit_checkpoint_saved(CHECKPOINT_ID);
    adapter.emit_checkpoint_restored(CHECKPOINT_ID);
    adapter.emit_scheduler_decision(7, Duration::from_millis(9), SCHEDULER_REASON);
    adapter.emit_head_of_line_block(1, 2, 3, HOL_REASON);
    adapter.emit_gpu_utilization(0, 50.0, 75.0);
    adapter.emit_token_usage(7, 11, 17);
    adapter.emit_memoization_hit(7);

    let events = collector.events();
    let kinds: Vec<_> = events.iter().map(ApxmEvent::kind).collect();
    assert_eq!(
        kinds,
        vec![
            kind::TOKEN,
            kind::TOKEN,
            kind::TOOL_START,
            kind::TOOL_END,
            kind::OPERATION_START,
            kind::OPERATION_END,
            kind::PLAN_CREATED,
            kind::PLAN_STEP_STARTED,
            kind::PLAN_STEP_COMPLETED,
            kind::MEMORY_READ,
            kind::MEMORY_WRITE,
            kind::CHECKPOINT_SAVED,
            kind::CHECKPOINT_RESTORED,
            kind::SCHEDULER_DECISION,
            kind::HEAD_OF_LINE_BLOCK,
            kind::GPU_UTILIZATION,
            kind::TOKEN_USAGE,
            kind::MEMOIZATION_HIT,
        ]
    );

    assert_eq!(payload::<TokenPayload>(&events[0]).text, TOKEN_TEXT);
    assert_eq!(payload::<TokenPayload>(&events[1]).text, NODE_TOKEN_TEXT);
    assert_eq!(payload::<ToolStartPayload>(&events[2]).name, TOOL_NAME);
    assert_eq!(
        payload::<ToolStartPayload>(&events[2]).args[TOOL_ARG],
        serde_json::json!(TOOL_ARG)
    );
    assert_eq!(
        payload::<ToolEndPayload>(&events[3]).result,
        serde_json::json!(TOOL_RESULT)
    );
    assert_eq!(payload::<OperationStartPayload>(&events[4]).node_id, 7);
    assert_eq!(
        payload::<OperationStartPayload>(&events[4]).op_type,
        AISOperationType::Ask
    );
    assert_eq!(payload::<OperationEndPayload>(&events[5]).duration_ms, 13);
    assert!(payload::<OperationEndPayload>(&events[5]).success);
    assert_eq!(payload::<PlanCreatedPayload>(&events[6]).steps, 2);
    assert_eq!(payload::<PlanStepStartedPayload>(&events[7]).step_index, 1);
    assert!(payload::<PlanStepCompletedPayload>(&events[8]).success);
    assert_eq!(payload::<MemoryReadPayload>(&events[9]).key, MEMORY_KEY);
    assert_eq!(
        payload::<MemoryWritePayload>(&events[10]).scope,
        MEMORY_SCOPE
    );
    assert_eq!(
        payload::<CheckpointSavedPayload>(&events[11]).checkpoint_id,
        CHECKPOINT_ID
    );
    assert_eq!(
        payload::<CheckpointRestoredPayload>(&events[12]).checkpoint_id,
        CHECKPOINT_ID
    );
    assert_eq!(payload::<SchedulerDecisionPayload>(&events[13]).delay_ms, 9);
    assert_eq!(
        payload::<HeadOfLineBlockPayload>(&events[14]).blocked_node,
        2
    );
    assert_eq!(
        payload::<GpuUtilizationPayload>(&events[15]).memory_pct,
        75.0
    );
    assert_eq!(payload::<TokenUsagePayload>(&events[16]).output_tokens, 17);
    assert_eq!(payload::<MemoizationHitPayload>(&events[17]).node_id, 7);
}

#[test]
fn emitter_adapter_propagates_metadata_without_backend() {
    let (collector, adapter) = collect_adapter();

    adapter.emit_llm_token(TOKEN_TEXT);
    adapter.set_current_span_id(Some(PARENT_SPAN_ID.to_string()));
    adapter.set_current_scope_id(Some(SCOPE_ID.to_string()));
    adapter.emit_memory_read(MEMORY_SCOPE, MEMORY_KEY);
    adapter.set_current_span_id(None);
    adapter.set_current_scope_id(None);
    adapter.emit_memory_write(MEMORY_SCOPE, MEMORY_KEY);

    let events = collector.events();
    assert_eq!(events.len(), 3);

    for (seq, event) in events.iter().enumerate() {
        assert_eq!(event.meta.seq, seq as u64);
        assert_eq!(event.meta.trace_id, TRACE_ID);
        assert_runtime_source(event);
    }

    assert!(events[0].meta.parent_span_id.is_none());
    assert!(events[0].meta.scope_id.is_none());
    assert_eq!(
        events[1].meta.parent_span_id.as_deref(),
        Some(PARENT_SPAN_ID)
    );
    assert_eq!(events[1].meta.scope_id.as_deref(), Some(SCOPE_ID));
    assert!(events[2].meta.parent_span_id.is_none());
    assert!(events[2].meta.scope_id.is_none());
}

#[test]
fn emitter_adapter_serializes_core_kind_boundary() {
    let (collector, adapter) = collect_adapter();
    adapter.emit_tool_start(TOOL_NAME, &HashMap::new());

    let events = collector.events();
    let serialized: SerializedEvent =
        serde_json::from_value(serde_json::to_value(&events[0]).expect("serialized event"))
            .expect("typed serialized event");

    assert_eq!(serialized.payload.kind, kind::TOOL_START.name());
}

#[test]
fn execution_event_emitter_defaults_are_noop_except_node_token_delegation() {
    let emitter = RecordingExecutionEmitter::default();

    emitter.set_current_span_id(Some(PARENT_SPAN_ID.to_string()));
    emitter.set_current_scope_id(Some(SCOPE_ID.to_string()));
    emitter.emit_graph_start(TRACE_ID, 1);
    emitter.emit_graph_end(TRACE_ID, 1, true);
    emitter.emit_operation_start(7, AISOperationType::Ask);
    emitter.emit_operation_end(
        7,
        AISOperationType::Ask,
        Duration::from_millis(1),
        true,
        None,
        None,
    );
    emitter.emit_llm_token_for_node(7, NODE_TOKEN_TEXT);

    assert!(emitter.current_span_id().is_none());
    assert!(emitter.current_scope_id().is_none());
    assert_eq!(emitter.tokens(), vec![NODE_TOKEN_TEXT.to_string()]);
}
