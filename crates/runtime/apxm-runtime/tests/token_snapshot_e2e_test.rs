//! Integration test: TokenAccountant snapshot is populated in ExecutionResult
//!
//! Verifies the full chain: MockBackend returns non-zero tokens → LLM handler
//! records them in TokenAccountant → ExecutorEngine.execute_dag() calls
//! snapshot() → ExecutionResult.token_snapshot contains the accumulated data.

use std::collections::HashMap;

use apxm_backends::llm::backends::mock::MockLLMBackend;
use apxm_core::types::execution::NodeMetadata;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use apxm_core::types::{ExecutionDag, Node};
use apxm_runtime::{Runtime, RuntimeConfig};

/// Minimal Ask node that exercises the LLM path with MockLLMBackend.
fn single_ask_dag() -> ExecutionDag {
    let mut ask_attrs = HashMap::new();
    ask_attrs.insert(
        "prompt".to_string(),
        Value::String("test prompt".to_string()),
    );
    // MockLLMBackend requires model attr; any string will do.
    ask_attrs.insert("model".to_string(), Value::String("mock-model".to_string()));

    let ask = Node {
        id: 1,
        op_type: AISOperationType::Ask,
        attributes: ask_attrs,
        input_tokens: vec![],
        output_tokens: vec![10],
        metadata: NodeMetadata::default(),
    };

    let mut dag = ExecutionDag::new();
    dag.add_node(ask).unwrap();
    dag.entry_nodes = dag.find_entry_nodes();
    dag.exit_nodes = dag.find_exit_nodes();
    dag
}

#[tokio::test]
async fn token_snapshot_e2e() {
    // 1. Set up runtime with MockLLMBackend
    let config = RuntimeConfig::in_memory();
    let runtime = Runtime::new(config).await.expect("runtime init");

    // Register MockLLMBackend so the Ask node resolves
    let mock = MockLLMBackend::static_response("mock response text");
    runtime
        .llm_registry()
        .register("mock-backend", mock)
        .expect("register backend");
    runtime
        .llm_registry()
        .set_default("mock-backend")
        .expect("set default");

    let dag = single_ask_dag();

    // 2. Execute the DAG
    let result = runtime
        .execute(dag)
        .await
        .expect("execution should succeed");

    // 3. Assert token_snapshot is populated with non-zero values
    let snapshot = &result.token_snapshot;

    // MockLLMBackend::static_response() uses MockResponse::new(), which returns
    // 10 input + 20 output tokens (see mock.rs:73-79)
    assert_eq!(
        snapshot.total.total_tokens, 30,
        "Expected 30 total tokens from MockLLMBackend (10 input + 20 output)"
    );
    assert_eq!(snapshot.total.input_tokens, 10);
    assert_eq!(snapshot.total.output_tokens, 20);
    assert_eq!(snapshot.total.call_count, 1);

    // 4. Assert per-node tracking captured the Ask node
    assert!(
        snapshot.per_node.contains_key(&1),
        "per_node should track node ID 1 (the Ask node)"
    );
    let node_1_usage = snapshot.per_node.get(&1).unwrap();
    assert_eq!(node_1_usage.total_tokens, 30);
    assert_eq!(node_1_usage.input_tokens, 10);
    assert_eq!(node_1_usage.output_tokens, 20);
}
