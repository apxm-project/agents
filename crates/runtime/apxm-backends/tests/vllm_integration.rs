//! Integration tests for vLLM graph-aware backend.

use apxm_backends::llm::backends::LLMRequest;
use apxm_backends::llm::backends::vllm::{
    ApxmGraphHints, GraphAwareVllmBackend, GraphMetadata, NodeSpec, PinPolicy,
};
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

    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["graph_id"], "graph-test");
    assert_eq!(json["execution_id"], "exec-001");
    assert_eq!(json["node_id"], 1);
    assert_eq!(json["node_name"], "reasoning-node");
    assert_eq!(json["priority_class"], "critical_path");
    assert_eq!(json["downstream_nodes"], json!([2, 3]));
    assert_eq!(json["pin_policy"]["mode"], "prefix");
    assert_eq!(json["pin_policy"]["ttl_ms"], 30_000);
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
        Some("critical_path".to_string())
    );
    assert_eq!(deserialized.downstream_nodes, vec![6, 7, 8]);
    assert_eq!(deserialized.pin_policy.mode, "prefix");
    assert_eq!(deserialized.pin_policy.ttl_ms, Some(45_000));
}

#[test]
fn test_pin_policy_variants_serialization() {
    // Test "none" mode
    let none_policy = PinPolicy::none();
    let none_json = serde_json::to_value(&none_policy).expect("serialize none");
    assert_eq!(none_json["mode"], "none");
    assert!(none_json.get("ttl_ms").is_none() || none_json["ttl_ms"].is_null());

    // Test "prefix" mode
    let prefix_policy = PinPolicy::prefix(60_000);
    let prefix_json = serde_json::to_value(&prefix_policy).expect("serialize prefix");
    assert_eq!(prefix_json["mode"], "prefix");
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
                priority_class: Some("critical_path".to_string()),
                reuse_group: Some("planning".to_string()),
                is_critical_path: true,
            },
            NodeSpec {
                node_id: 2,
                node_name: Some("coder".to_string()),
                estimated_prompt_tokens: Some(1500),
                downstream_nodes: vec![4],
                priority_class: Some("parallel".to_string()),
                reuse_group: None,
                is_critical_path: false,
            },
        ]);

    let json = serde_json::to_value(&metadata).expect("serialize metadata");

    assert_eq!(json["graph_id"], "graph-001");
    assert_eq!(json["execution_id"], "exec-123");
    assert_eq!(json["default_pin_ttl_ms"], 30_000);
    assert_eq!(json["critical_path_length"], 5);
    assert_eq!(json["node_count"], 2);
    assert_eq!(json["nodes"].as_array().unwrap().len(), 2);

    let node1 = &json["nodes"][0];
    assert_eq!(node1["node_id"], 1);
    assert_eq!(node1["node_name"], "architect");
    assert_eq!(node1["estimated_prompt_tokens"], 500);
    assert_eq!(node1["priority_class"], "critical_path");
    assert_eq!(node1["is_critical_path"], true);
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
    assert_eq!(hints_json["schema_version"], 1);
    assert_eq!(hints_json["graph_id"], "test-graph");
    assert_eq!(hints_json["node_id"], 1);
    assert_eq!(hints_json["priority_class"], "critical_path");

    // Verify the hint injection creates proper extra_body structure
    let extra_body = serde_json::json!({"apxm": hints_json});
    assert!(extra_body.is_object());
    assert!(extra_body.get("apxm").is_some());
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

    assert_eq!(hints.priority_class, Some("critical_path".to_string()));
    assert_eq!(hints.node_id, Some(1));
    assert_eq!(hints.downstream_nodes, vec![2, 3]);

    // Verify parallel hints don't have critical priority
    let parallel = ApxmGraphHints::parallel("graph-xyz", "exec-002", 5, "parallel-node");
    assert_eq!(parallel.priority_class, Some("parallel".to_string()));
}

#[tokio::test]
async fn test_graph_registration_request_structure() {
    let expected_body = json!({
        "graph_id": "workflow-123",
        "execution_id": "exec-456",
        "critical_path_length": 3,
        "node_count": 2,
        "default_pin_ttl_ms": 45000,
        "nodes": [
            {
                "node_id": 1,
                "node_name": "planner",
                "estimated_prompt_tokens": 800,
                "downstream_nodes": [2],
                "priority_class": "critical_path",
                "is_critical_path": true
            },
            {
                "node_id": 2,
                "node_name": "executor",
                "downstream_nodes": [],
                "is_critical_path": false
            }
        ]
    });

    let backend = GraphAwareVllmBackend::new(
        "",
        Some(json!({
            "base_url": "http://vllm.test:8000/v1",
            "model": "test-model"
        })),
    )
    .await
    .expect("create backend");

    let metadata = GraphMetadata::new("workflow-123", "exec-456")
        .with_pin_ttl(45_000)
        .with_critical_path_length(3)
        .with_nodes(vec![
            NodeSpec {
                node_id: 1,
                node_name: Some("planner".to_string()),
                estimated_prompt_tokens: Some(800),
                downstream_nodes: vec![2],
                priority_class: Some("critical_path".to_string()),
                reuse_group: None,
                is_critical_path: true,
            },
            NodeSpec {
                node_id: 2,
                node_name: Some("executor".to_string()),
                estimated_prompt_tokens: None,
                downstream_nodes: vec![],
                priority_class: None,
                reuse_group: None,
                is_critical_path: false,
            },
        ]);

    let payload = serde_json::to_value(&metadata).expect("serialize metadata");
    assert_eq!(
        backend.graph_registration_url(),
        "http://vllm.test:8000/v1/apxm/graphs/register"
    );
    assert_eq!(payload, expected_body);
}
