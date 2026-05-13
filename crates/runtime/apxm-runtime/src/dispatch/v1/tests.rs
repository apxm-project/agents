//! Round-trip tests proving `DispatchIrV1` preserves the existing
//! `ApxmGraphHints` field set without loss.

use std::collections::HashMap;

use apxm_core::types::graph_hints::{
    ApxmGraphHints, GraphMetadata, NodeSpec, PinMode, PinPolicy, PriorityClass,
};
use apxm_core::types::graph_metrics::{LatencyClass, NodeGraphMetrics};

use super::lower::{derive_apxm_hints, lower_graph};
use super::plan::{
    BackendCapabilityRequirements, DISPATCH_IR_V1_SCHEMA_VERSION, TelemetryContract,
};

fn sample_metrics() -> NodeGraphMetrics {
    NodeGraphMetrics::default()
        .with_fanout_count(8)
        .with_remaining_path_len(4)
        .with_latency_class(LatencyClass::Long)
        .with_batch_group("review-fanout")
        .with_stage_index(2)
        .with_estimated_dynamic_tokens(256)
}

fn sample_metadata() -> GraphMetadata {
    GraphMetadata::new("graph-rt", "exec-rt")
        .with_pin_ttl(30_000)
        .with_critical_path_length(4)
        .with_nodes(vec![NodeSpec {
            node_id: 7,
            node_name: Some("security_review".to_owned()),
            estimated_prompt_tokens: Some(1024),
            downstream_nodes: vec![11],
            priority_class: Some(PriorityClass::CriticalPath),
            reuse_group: Some("shared-review-context".to_owned()),
            graph_metrics: sample_metrics(),
            is_critical_path: true,
        }])
}

fn sample_hints() -> ApxmGraphHints {
    ApxmGraphHints {
        schema_version: 1,
        graph_id: Some("graph-rt".to_owned()),
        execution_id: Some("exec-rt".to_owned()),
        node_id: Some(7),
        node_name: Some("security_review".to_owned()),
        priority_class: Some(PriorityClass::CriticalPath),
        downstream_nodes: vec![11],
        reuse_group: Some("shared-review-context".to_owned()),
        graph_metrics: sample_metrics(),
        pin_policy: PinPolicy::prefix(30_000),
        compiler_hints: Default::default(),
    }
}

#[test]
fn lower_graph_carries_graph_identity_and_shape() {
    let metadata = sample_metadata();
    let hints = sample_hints();
    let mut node_hints = HashMap::new();
    node_hints.insert(7, hints);

    let ir = lower_graph(
        &metadata,
        &node_hints,
        BackendCapabilityRequirements::default(),
        TelemetryContract::default(),
    );

    assert_eq!(ir.schema_version, DISPATCH_IR_V1_SCHEMA_VERSION);
    assert_eq!(ir.graph.graph_id, "graph-rt");
    assert_eq!(ir.graph.execution_id.as_deref(), Some("exec-rt"));
    assert_eq!(ir.graph.critical_path_length, Some(4));
    assert_eq!(ir.graph.default_pin_ttl_ms, Some(30_000));
    assert_eq!(ir.graph.node_count, Some(1));
    assert_eq!(ir.nodes.len(), 1);
}

#[test]
fn lower_graph_preserves_node_intent_fields() {
    let metadata = sample_metadata();
    let hints = sample_hints();
    let mut node_hints = HashMap::new();
    node_hints.insert(7, hints);

    let ir = lower_graph(
        &metadata,
        &node_hints,
        BackendCapabilityRequirements::default(),
        TelemetryContract::default(),
    );

    let node = &ir.nodes[0];
    assert_eq!(node.node_id, 7);
    assert_eq!(node.node_name.as_deref(), Some("security_review"));
    assert_eq!(node.priority_class, Some(PriorityClass::CriticalPath));
    assert_eq!(node.latency_class, Some(LatencyClass::Long));
    assert_eq!(node.downstream_nodes, vec![11]);
    assert_eq!(node.prefix_cohort.as_deref(), Some("shared-review-context"));
    assert_eq!(node.reuse_group.as_deref(), Some("shared-review-context"));
    assert_eq!(node.batch_group.as_deref(), Some("review-fanout"));
    assert_eq!(node.fanout_count, Some(8));
    assert_eq!(node.remaining_path_len, Some(4));
    assert_eq!(node.stage_index, Some(2));
    assert_eq!(node.estimated_dynamic_tokens, Some(256));
    assert_eq!(node.pin_policy.mode, PinMode::Prefix);
    assert_eq!(node.pin_policy.ttl_ms, Some(30_000));
}

#[test]
fn derive_apxm_hints_round_trips_existing_fields() {
    let metadata = sample_metadata();
    let hints_in = sample_hints();
    let mut node_hints = HashMap::new();
    node_hints.insert(7, hints_in.clone());

    let ir = lower_graph(
        &metadata,
        &node_hints,
        BackendCapabilityRequirements::default(),
        TelemetryContract::default(),
    );

    let derived = derive_apxm_hints(&ir, &ir.nodes[0]);

    assert_eq!(derived.schema_version, hints_in.schema_version);
    assert_eq!(derived.graph_id, hints_in.graph_id);
    assert_eq!(derived.execution_id, hints_in.execution_id);
    assert_eq!(derived.node_id, hints_in.node_id);
    assert_eq!(derived.node_name, hints_in.node_name);
    assert_eq!(derived.priority_class, hints_in.priority_class);
    assert_eq!(derived.downstream_nodes, hints_in.downstream_nodes);
    assert_eq!(derived.reuse_group, hints_in.reuse_group);
    assert_eq!(derived.graph_metrics, hints_in.graph_metrics);
    assert_eq!(derived.pin_policy.mode, hints_in.pin_policy.mode);
    assert_eq!(derived.pin_policy.ttl_ms, hints_in.pin_policy.ttl_ms);
}

#[test]
fn lower_graph_falls_back_to_node_spec_when_hint_missing() {
    let metadata = sample_metadata();
    let node_hints: HashMap<u32, ApxmGraphHints> = HashMap::new();

    let ir = lower_graph(
        &metadata,
        &node_hints,
        BackendCapabilityRequirements::default(),
        TelemetryContract::default(),
    );

    let node = &ir.nodes[0];
    assert_eq!(node.priority_class, Some(PriorityClass::CriticalPath));
    assert_eq!(node.downstream_nodes, vec![11]);
    assert_eq!(node.reuse_group.as_deref(), Some("shared-review-context"));
    assert_eq!(node.fanout_count, Some(8));
    assert_eq!(node.batch_group.as_deref(), Some("review-fanout"));
    assert_eq!(node.pin_policy.mode, PinMode::None);
}

#[test]
fn dispatch_ir_serializes_with_expected_schema_version() {
    let metadata = sample_metadata();
    let hints = sample_hints();
    let mut node_hints = HashMap::new();
    node_hints.insert(7, hints);

    let ir = lower_graph(
        &metadata,
        &node_hints,
        BackendCapabilityRequirements {
            backend: Some("vllm-fork".to_owned()),
            protocol: Some("vllm".to_owned()),
            required: vec!["priority".to_owned(), "prefix_cohorts".to_owned()],
            optional: vec!["pin_release".to_owned()],
        },
        TelemetryContract {
            required_labels: vec!["graph_id".to_owned(), "node_id".to_owned()],
            requested_metrics: vec!["queue_wait_ms".to_owned(), "ttft_ms".to_owned()],
        },
    );

    let json = serde_json::to_value(&ir).expect("serialize");
    assert_eq!(json["schema_version"], DISPATCH_IR_V1_SCHEMA_VERSION);
    assert_eq!(json["graph"]["graph_id"], "graph-rt");
    assert_eq!(json["requirements"]["backend"], "vllm-fork");
    assert_eq!(json["nodes"][0]["fanout_count"], 8);
    assert_eq!(json["telemetry"]["required_labels"][0], "graph_id");
}
