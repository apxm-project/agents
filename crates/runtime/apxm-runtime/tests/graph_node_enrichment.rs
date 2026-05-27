//! Phase 14.8.A — Per-graph-node event enrichment.
//!
//! These tests exercise the additional event kinds (`agent_spawned`,
//! `communicate_dispatched`, `graph_edge`) and the new `context` field on
//! `OperationStartPayload`. The runtime emits them through the
//! `ExecutionEventEmitter` so out-of-tree consumers can observe a
//! labelled dispatch tree without re-scraping op attributes.

use std::sync::{Arc, Mutex};

use apxm_core::events::kind;
use apxm_core::events::payload::{
    AgentSpawnedPayload, CommunicateDispatchedPayload, GraphEdgePayload, OperationStartPayload,
};
use apxm_core::events::{ApxmEvent, EventEmitter, EventPayload, EventSource};
use apxm_core::types::operations::AISOperationType;
use apxm_runtime::EmitterAdapter;

const TRACE_ID: &str = "trace-enrichment";

#[derive(Default)]
struct Collector {
    events: Mutex<Vec<ApxmEvent>>,
}

impl Collector {
    fn events(&self) -> Vec<ApxmEvent> {
        self.events.lock().expect("events lock").clone()
    }
}

impl EventEmitter for Collector {
    fn emit(&self, event: ApxmEvent) {
        self.events.lock().expect("events lock").push(event);
    }
}

fn adapter() -> (Arc<Collector>, EmitterAdapter) {
    let collector: Arc<Collector> = Arc::new(Collector::default());
    let adapter = EmitterAdapter::new(collector.clone(), EventSource::Runtime, TRACE_ID);
    (collector, adapter)
}

fn payload<T: EventPayload>(event: &ApxmEvent) -> &T {
    event
        .payload
        .downcast_ref::<T>()
        .expect("payload downcast")
}

#[test]
fn agent_spawned_event_carries_agent_code_and_parent_execution_id() {
    use apxm_runtime::ExecutionEventEmitter;
    let (collector, adapter) = adapter();

    adapter.emit_agent_spawned(
        17,
        "task.crm.lead",
        "exec-42",
        Some("acp-codex"),
        Some("pid-9"),
        Some("snapshot"),
    );

    let events = collector.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind(), kind::AGENT_SPAWNED);

    let p = payload::<AgentSpawnedPayload>(&events[0]);
    assert_eq!(p.node_id, 17);
    assert_eq!(p.agent_code, "task.crm.lead");
    assert_eq!(p.parent_execution_id, "exec-42");
    assert_eq!(p.profile.as_deref(), Some("acp-codex"));
    assert_eq!(p.process_id.as_deref(), Some("pid-9"));
    assert_eq!(p.scope_policy.as_deref(), Some("snapshot"));
}

#[test]
fn operation_start_for_communicate_includes_target_agent_in_context() {
    use apxm_runtime::ExecutionEventEmitter;
    let (collector, adapter) = adapter();

    adapter.emit_operation_start_with_context(
        21,
        AISOperationType::Communicate,
        serde_json::json!({
            "target_agent": "module.revenue",
            "protocol": "local",
        }),
    );

    let events = collector.events();
    let p = payload::<OperationStartPayload>(&events[0]);
    let ctx = p.context.as_ref().expect("context present");
    assert_eq!(ctx["target_agent"], "module.revenue");
    assert_eq!(ctx["protocol"], "local");
}

#[test]
fn operation_start_for_ask_includes_tool_names_when_tool_choice_set() {
    use apxm_runtime::ExecutionEventEmitter;
    let (collector, adapter) = adapter();

    adapter.emit_operation_start_with_context(
        9,
        AISOperationType::Ask,
        serde_json::json!({
            "model": "qwen-3-coder",
            "backend": "ollama",
            "tool_names": ["search_leads", "open_lead"],
            "tool_choice": "auto",
        }),
    );

    let events = collector.events();
    let p = payload::<OperationStartPayload>(&events[0]);
    let ctx = p.context.as_ref().expect("context present");
    assert_eq!(ctx["model"], "qwen-3-coder");
    let names = ctx["tool_names"].as_array().expect("tool_names");
    assert_eq!(names.len(), 2);
    assert_eq!(names[0], "search_leads");
}

#[test]
fn communicate_dispatched_payload_carries_protocol_and_excerpt() {
    use apxm_runtime::ExecutionEventEmitter;
    let (collector, adapter) = adapter();

    adapter.emit_communicate_dispatched(
        21,
        "module.crm",
        "http",
        Some("please triage incoming leads"),
    );

    let events = collector.events();
    let p = payload::<CommunicateDispatchedPayload>(&events[0]);
    assert_eq!(p.target_agent, "module.crm");
    assert_eq!(p.protocol, "http");
    assert_eq!(p.message_excerpt.as_deref(), Some("please triage incoming leads"));
}

#[test]
fn graph_edge_emitted_for_dispatch_and_tool_invocation() {
    use apxm_runtime::ExecutionEventEmitter;
    let (collector, adapter) = adapter();

    adapter.emit_graph_edge(3, 4, "dispatch");
    adapter.emit_graph_edge(4, 7, "tool_invocation");
    adapter.emit_graph_edge(7, 9, "synthesis_feed");

    let events = collector.events();
    assert_eq!(events.len(), 3);
    let edges: Vec<&GraphEdgePayload> = events.iter().map(payload::<GraphEdgePayload>).collect();
    assert_eq!(edges[0].from_node_id, 3);
    assert_eq!(edges[0].to_node_id, 4);
    assert_eq!(edges[0].kind, "dispatch");
    assert_eq!(edges[1].kind, "tool_invocation");
    assert_eq!(edges[2].kind, "synthesis_feed");
}

#[test]
fn operation_start_payload_serializes_with_optional_context() {
    let payload = OperationStartPayload {
        node_id: 1,
        op_type: AISOperationType::SpawnAgent,
        context: Some(serde_json::json!({"agent_code": "task.crm.lead"})),
    };

    let json = serde_json::to_value(&payload).expect("serialize");
    assert_eq!(json["context"]["agent_code"], "task.crm.lead");

    let payload_none = OperationStartPayload {
        node_id: 2,
        op_type: AISOperationType::Nop,
        context: None,
    };
    let json_none = serde_json::to_value(&payload_none).expect("serialize");
    // Optional + skip_serializing_if must elide `context` entirely when None.
    assert!(json_none.get("context").is_none());
}
