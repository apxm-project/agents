//! Round-trip tests proving `DispatchIrV1` preserves the existing
//! `ApxmGraphHints` field set without loss.

use std::collections::HashMap;

use apxm_core::constants::llm::apxm::dispatch_fields as df;
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
            required: vec![df::PRIORITY.to_owned(), df::PREFIX_COHORTS.to_owned()],
            optional: vec![df::PIN_RELEASE.to_owned()],
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

mod gating {
    use super::*;
    use crate::dispatch::v1::{
        DispatchFallback, dispatch_ir_accounting_json, evaluate_required_capabilities,
    };
    use apxm_core::types::BackendGraphCapabilities;
    use std::collections::HashMap;

    fn caps_full() -> BackendGraphCapabilities {
        BackendGraphCapabilities {
            supports_graph_registration: true,
            supports_request_hints: true,
            supports_priority: true,
            supports_prefix_cohorts: true,
            supports_pin_release: true,
            supports_structured_outputs: true,
            supports_backend_queue_state: true,
            supports_backend_cache_state: true,
            supports_cancel_groups: true,
            supports_dispatch_ir_v1_internal: true,
            supports_admin_reset_prefix_cache: true,
        }
    }

    fn caps_no_graph_registration() -> BackendGraphCapabilities {
        let mut c = caps_full();
        c.supports_graph_registration = false;
        c
    }

    fn one_node_ir() -> super::super::plan::DispatchIrV1 {
        let metadata = sample_metadata();
        let mut node_hints = HashMap::new();
        node_hints.insert(7, sample_hints());
        lower_graph(
            &metadata,
            &node_hints,
            BackendCapabilityRequirements {
                backend: None,
                protocol: None,
                required: vec![
                    df::GRAPH_REGISTRATION.to_owned(),
                    df::REQUEST_HINTS.to_owned(),
                ],
                optional: vec![df::PRIORITY.to_owned()],
            },
            TelemetryContract::default(),
        )
    }

    #[test]
    fn evaluate_required_capabilities_returns_none_when_backend_full() {
        let ir = one_node_ir();
        let caps = caps_full();
        assert!(evaluate_required_capabilities("vllm", &caps, &ir).is_none());
    }

    #[test]
    fn evaluate_required_capabilities_returns_fallback_when_required_missing() {
        let ir = one_node_ir();
        let caps = caps_no_graph_registration();
        let fb = evaluate_required_capabilities("vllm", &caps, &ir)
            .expect("backend missing required capability must produce a fallback");
        assert_eq!(fb.backend_name, "vllm");
        assert_eq!(fb.missing_required, vec![df::GRAPH_REGISTRATION.to_owned()]);
        assert!(
            fb.reason.contains("flat-HTTP"),
            "fallback reason must name the degraded path so the claim cannot \
             inherit graph-registered semantics; got: {}",
            fb.reason
        );
    }

    #[test]
    fn accounting_json_surfaces_fallback_records_and_new_keys() {
        let ir = one_node_ir();
        let mut backend_capabilities = HashMap::new();
        backend_capabilities.insert("vllm".to_owned(), caps_full());

        let fb = DispatchFallback {
            backend_name: "openai-shim".to_owned(),
            missing_required: vec![df::REQUEST_HINTS.to_owned()],
            reason: "test fallback".to_owned(),
        };

        let no_honored = HashMap::<String, Vec<String>>::new();
        let json = dispatch_ir_accounting_json(
            Some(&ir),
            &backend_capabilities,
            &[],
            &[fb],
            &no_honored,
        );

        assert_eq!(json["fallback_triggered"], true);
        assert_eq!(json["fallbacks"][0]["backend"], "openai-shim");
        assert_eq!(
            json["fallbacks"][0]["missing_required"][0],
            df::REQUEST_HINTS
        );
        // fields_honored is the per-request runtime-evidence channel
        // It MUST start empty under a populated
        // fields_sent because no fork-side x-apxm-fields-honored
        // emitter is installed in this test.
        assert!(json["fields_honored"].is_object());
        assert_eq!(json["fields_honored"].as_object().unwrap().len(), 0);
        // fields_passthrough_only must list the v1 vLLM scheduler's
        // unhonored telemetry fields explicitly.
        assert!(
            json["fields_passthrough_only"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "latency_class"),
            "passthrough list must name latency_class explicitly"
        );
    }

    #[test]
    fn accounting_json_surfaces_runtime_fields_honored() {
        // Plan 07 §2 closure — proves the per-execution aggregator's
        // snapshot lands in dispatch_ir_metrics.fields_honored. The
        // collector itself (FieldsHonoredCollector) has its own unit
        // tests; this test pins the JSON contract.
        let ir = one_node_ir();
        let mut backend_capabilities = HashMap::new();
        backend_capabilities.insert("vllm-fork".to_owned(), caps_full());

        let mut fields_honored = HashMap::new();
        fields_honored.insert(
            "vllm-fork".to_owned(),
            vec![
                df::PRIORITY.to_owned(),
                df::PIN_RELEASE.to_owned(),
                df::PREFIX_COHORTS.to_owned(),
            ],
        );

        let json = dispatch_ir_accounting_json(
            Some(&ir),
            &backend_capabilities,
            &[],
            &[],
            &fields_honored,
        );

        let honored = &json["fields_honored"]["vllm-fork"];
        assert_eq!(honored.as_array().unwrap().len(), 3);
        assert!(
            honored
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == df::PRIORITY),
            "expected priority in fields_honored, got: {honored}"
        );
    }

    #[test]
    fn dispatch_ir_round_trips_through_json() {
        // Pin the wire-format round-trip so any field
        // added to DispatchIrV1 must deserialize cleanly. If you add
        // a field and this test still passes, you forgot to wire
        // serde — or the field is optional in a way that lets
        // round-trip-lossy schemas slip through.
        let ir = one_node_ir();
        let json = serde_json::to_value(&ir).expect("serialize");
        let parsed: super::super::plan::DispatchIrV1 =
            serde_json::from_value(json.clone()).expect("deserialize");
        let reserialized = serde_json::to_value(&parsed).expect("re-serialize");
        assert_eq!(
            json, reserialized,
            "round-trip must be lossless; mismatched keys: {} vs {}",
            json, reserialized
        );
        assert_eq!(parsed.schema_version, super::DISPATCH_IR_V1_SCHEMA_VERSION);
        assert_eq!(parsed.nodes.len(), 1);
    }

    #[test]
    fn accounting_json_reports_no_fallback_when_all_backends_meet_required() {
        let ir = one_node_ir();
        let mut backend_capabilities = HashMap::new();
        backend_capabilities.insert("vllm".to_owned(), caps_full());
        let no_honored = HashMap::<String, Vec<String>>::new();
        let json = dispatch_ir_accounting_json(
            Some(&ir),
            &backend_capabilities,
            &[],
            &[],
            &no_honored,
        );
        assert_eq!(json["fallback_triggered"], false);
        assert!(json["fallbacks"].as_array().unwrap().is_empty());
    }
}
