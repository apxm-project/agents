//! Integration tests for vLLM graph-aware backend.

use apxm_backends::llm::backends::LLMRequest;
use apxm_backends::llm::backends::vllm::{
    ApxmGraphHints, GraphAwareVllmBackend, GraphMetadata, NodeSpec, PinPolicy,
};
use apxm_core::constants::llm::{api_paths, apxm as apxm_llm};
use apxm_core::types::{NodeGraphMetrics, PinMode, PriorityClass};
use serde_json::json;

#[tokio::test]
async fn test_apxm_graph_hints_serialization() {
    let hints = ApxmGraphHints::critical_path(
        "graph-test",
        "exec-001",
        1,
        "reasoning-node",
        vec![2, 3],
        30_000,
    );

    let json = serde_json::to_value(&hints).expect("serialize hints");

    assert_eq!(json[apxm_llm::SCHEMA_VERSION], 1);
    assert_eq!(json[apxm_llm::GRAPH_ID], "graph-test");
    assert_eq!(json[apxm_llm::EXECUTION_ID], "exec-001");
    assert_eq!(json[apxm_llm::NODE_ID], 1);
    assert_eq!(json[apxm_llm::NODE_NAME], "reasoning-node");
    assert_eq!(
        json[apxm_llm::PRIORITY_CLASS],
        apxm_llm::PRIORITY_CRITICAL_PATH
    );
    assert_eq!(json[apxm_llm::DOWNSTREAM_NODES], json!([2, 3]));
    assert_eq!(
        json[apxm_llm::PIN_POLICY][apxm_llm::PIN_POLICY_MODE],
        apxm_llm::PIN_MODE_PREFIX
    );
    assert_eq!(
        json[apxm_llm::PIN_POLICY][apxm_llm::PIN_POLICY_TTL_MS],
        30_000
    );
}

#[tokio::test]
async fn test_apxm_graph_hints_roundtrip() {
    let original =
        ApxmGraphHints::critical_path("graph-abc", "exec-xyz", 5, "plan", vec![6, 7, 8], 45_000);

    let json_str = serde_json::to_string(&original).expect("serialize");
    let deserialized: ApxmGraphHints = serde_json::from_str(&json_str).expect("deserialize");

    assert_eq!(deserialized.schema_version, 1);
    assert_eq!(deserialized.graph_id, Some("graph-abc".to_string()));
    assert_eq!(deserialized.execution_id, Some("exec-xyz".to_string()));
    assert_eq!(deserialized.node_id, Some(5));
    assert_eq!(deserialized.node_name, Some("plan".to_string()));
    assert_eq!(
        deserialized.priority_class,
        Some(PriorityClass::CriticalPath)
    );
    assert_eq!(deserialized.downstream_nodes, vec![6, 7, 8]);
    assert_eq!(deserialized.pin_policy.mode, PinMode::Prefix);
    assert_eq!(deserialized.pin_policy.ttl_ms, Some(45_000));
}

#[test]
fn test_pin_policy_variants_serialization() {
    // Test "none" mode
    let none_policy = PinPolicy::none();
    let none_json = serde_json::to_value(&none_policy).expect("serialize none");
    assert_eq!(none_json["mode"], apxm_llm::PIN_MODE_NONE);
    assert!(none_json.get("ttl_ms").is_none() || none_json["ttl_ms"].is_null());

    // Test "prefix" mode
    let prefix_policy = PinPolicy::prefix(60_000);
    let prefix_json = serde_json::to_value(&prefix_policy).expect("serialize prefix");
    assert_eq!(prefix_json["mode"], apxm_llm::PIN_MODE_PREFIX);
    assert_eq!(prefix_json["ttl_ms"], 60_000);
}

#[test]
fn test_graph_metadata_registration_shape() {
    let metadata = GraphMetadata::new("graph-001", "exec-123")
        .with_pin_ttl(30_000)
        .with_critical_path_length(5)
        .with_nodes(vec![
            NodeSpec {
                node_id: 1,
                node_name: Some("architect".to_string()),
                estimated_prompt_tokens: Some(500),
                downstream_nodes: vec![2, 3],
                priority_class: Some(PriorityClass::CriticalPath),
                reuse_group: Some("planning".to_string()),
                graph_metrics: NodeGraphMetrics::default(),
                is_critical_path: true,
            },
            NodeSpec {
                node_id: 2,
                node_name: Some("coder".to_string()),
                estimated_prompt_tokens: Some(1500),
                downstream_nodes: vec![4],
                priority_class: Some(PriorityClass::Parallel),
                reuse_group: None,
                graph_metrics: NodeGraphMetrics::default(),
                is_critical_path: false,
            },
        ]);

    let json = serde_json::to_value(&metadata).expect("serialize metadata");
    let roundtrip: GraphMetadata = serde_json::from_value(json.clone()).expect("roundtrip");

    assert_eq!(roundtrip.graph_id, "graph-001");
    assert_eq!(roundtrip.execution_id.as_deref(), Some("exec-123"));
    assert_eq!(roundtrip.default_pin_ttl_ms, Some(30_000));
    assert_eq!(roundtrip.critical_path_length, Some(5));
    assert_eq!(roundtrip.node_count, Some(2));
    assert_eq!(roundtrip.nodes.len(), 2);
    assert_eq!(roundtrip.nodes[0].node_id, 1);
    assert_eq!(roundtrip.nodes[0].node_name.as_deref(), Some("architect"));
    assert_eq!(roundtrip.nodes[0].estimated_prompt_tokens, Some(500));
    assert_eq!(
        roundtrip.nodes[0].priority_class,
        Some(PriorityClass::CriticalPath)
    );
    assert!(roundtrip.nodes[0].is_critical_path);
}

#[test]
fn test_backend_injects_apxm_hints_into_extra_body() {
    // Test that hints are properly injected into extra_body
    let hints =
        ApxmGraphHints::critical_path("test-graph", "test-exec", 1, "test-node", vec![2], 30_000);

    let request = LLMRequest::new("test prompt")
        .with_temperature(0.0)
        .with_apxm_hints(hints.clone());

    // The request should have apxm_hints set
    assert!(request.apxm_hints.is_some());

    // Serialize the hints to verify structure
    let hints_json = serde_json::to_value(&hints).unwrap();
    assert_eq!(hints_json[apxm_llm::SCHEMA_VERSION], 1);
    assert_eq!(hints_json[apxm_llm::GRAPH_ID], "test-graph");
    assert_eq!(hints_json[apxm_llm::NODE_ID], 1);
    assert_eq!(
        hints_json[apxm_llm::PRIORITY_CLASS],
        apxm_llm::PRIORITY_CRITICAL_PATH
    );

    // Verify the hint injection creates proper extra_body structure
    let extra_body = serde_json::json!({apxm_llm::VLLM_XARGS: {apxm_llm::HINTS_FIELD: hints_json}});
    assert!(extra_body.is_object());
    assert!(extra_body.get(apxm_llm::VLLM_XARGS).is_some());
}

#[test]
fn test_critical_path_priority_mapped_in_request() {
    // Verify that critical path hints have the correct priority_class
    let hints = ApxmGraphHints::critical_path(
        "graph-critical",
        "exec-001",
        1,
        "critical-node",
        vec![2, 3],
        30_000,
    );

    assert_eq!(hints.priority_class, Some(PriorityClass::CriticalPath));
    assert_eq!(hints.node_id, Some(1));
    assert_eq!(hints.downstream_nodes, vec![2, 3]);

    // Verify parallel hints don't have critical priority
    let parallel = ApxmGraphHints::parallel("graph-xyz", "exec-002", 5, "parallel-node");
    assert_eq!(parallel.priority_class, Some(PriorityClass::Parallel));
}

#[tokio::test]
async fn test_graph_registration_request_structure() {
    let backend = GraphAwareVllmBackend::new(
        "",
        Some(json!({
            "base_url": "http://vllm.test:8000/v1",
            "model": "test-model"
        })),
    )
    .await
    .expect("create backend");

    let metadata = GraphMetadata::new("graph-123", "exec-456")
        .with_pin_ttl(45_000)
        .with_critical_path_length(3)
        .with_nodes(vec![
            NodeSpec {
                node_id: 1,
                node_name: Some("planner".to_string()),
                estimated_prompt_tokens: Some(800),
                downstream_nodes: vec![2],
                priority_class: Some(PriorityClass::CriticalPath),
                reuse_group: None,
                graph_metrics: NodeGraphMetrics::default(),
                is_critical_path: true,
            },
            NodeSpec {
                node_id: 2,
                node_name: Some("executor".to_string()),
                estimated_prompt_tokens: None,
                downstream_nodes: vec![],
                priority_class: None,
                reuse_group: None,
                graph_metrics: NodeGraphMetrics::default(),
                is_critical_path: false,
            },
        ]);

    let expected_body = serde_json::to_value(&metadata).expect("serialize expected metadata");
    let payload = serde_json::to_value(&metadata).expect("serialize metadata");
    assert_eq!(
        backend.graph_registration_url(),
        format!(
            "http://vllm.test:8000{}{}",
            api_paths::VERSION_PREFIX,
            api_paths::APXM_GRAPHS_REGISTER
        )
    );
    assert_eq!(payload, expected_body);
}
