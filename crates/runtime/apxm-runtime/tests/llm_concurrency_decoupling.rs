//! Step 5 verification: LLM concurrency is decoupled from compute concurrency.
//!
//! These tests cover LLM backend concurrency isolation: with both semaphores
//! set to 1, an LLM node must release its permit before the
//! downstream non-LLM child node tries to acquire one. If the worker held the
//! LLM permit across child scheduling, the child could still run because the
//! child takes the *compute* semaphore — but historically a single-semaphore
//! design would lock both sides together and deadlock under bursty fan-out.
//! This test guards the new branch in `worker::worker_loop` that picks the
//! right semaphore per op type.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use apxm_core::types::execution::NodeMetadata;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use apxm_core::types::{DependencyType, Edge, ExecutionDag, Node};
use apxm_runtime::{Runtime, RuntimeConfig, SchedulerConfig};

/// Build an Ask → Identity DAG.
///
/// - Node 1 (Ask): would normally hit the LLM, but we attach a `fallback`
///   attribute so the scheduler treats the failure as a successful publish
///   instead of aborting. This lets the downstream Identity node actually run,
///   which is the whole point of the deadlock check.
/// - Node 2 (Identity): pure passthrough; takes the compute semaphore.
fn ask_then_identity_dag() -> ExecutionDag {
    let mut ask_attrs = HashMap::new();
    ask_attrs.insert(
        "fallback".to_string(),
        Value::String("fallback-value".to_string()),
    );
    // Minimal prompt so the handler doesn't reject before reaching the
    // backend lookup; the LLM call will still fail under an empty registry.
    ask_attrs.insert("prompt".to_string(), Value::String("hello".to_string()));

    let ask = Node {
        id: 1,
        op_type: AISOperationType::Ask,
        attributes: ask_attrs,
        input_tokens: vec![],
        output_tokens: vec![10],
        metadata: NodeMetadata::default(),
    };
    let identity = Node {
        id: 2,
        op_type: AISOperationType::Identity,
        attributes: HashMap::new(),
        input_tokens: vec![10],
        output_tokens: vec![20],
        metadata: NodeMetadata::default(),
    };

    let mut dag = ExecutionDag::new();
    dag.add_node(ask).unwrap();
    dag.add_node(identity).unwrap();
    dag.add_edge(Edge::new(1, 2, 10, DependencyType::Data))
        .unwrap();
    dag.entry_nodes = dag.find_entry_nodes();
    dag.exit_nodes = dag.find_exit_nodes();
    dag
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_deadlock_when_both_semaphores_are_one() {
    // Worst case for R6: both semaphores set to 1. If the worker held the LLM
    // permit across the child's submission instead of dropping it between
    // nodes, the child would never be reached. Either way, this must not hang.
    let scheduler_config = SchedulerConfig::new()
        .with_max_concurrency(1)
        .with_max_inflight(1)
        .with_llm_inflight(1)
        // Generous deadlock timeout so the watchdog doesn't fire spuriously
        // under a slow CI; we want the *real* tokio::time::timeout below to
        // be the assertion.
        .with_deadlock_timeout(60_000);

    let mut config = RuntimeConfig::in_memory();
    config.scheduler_config = scheduler_config;
    let runtime = Arc::new(Runtime::new(config).await.expect("runtime init"));

    let dag = ask_then_identity_dag();

    // 10s ceiling: the actual happy path should finish in well under a
    // second. Anything close to this bound implies a hang.
    let result = tokio::time::timeout(Duration::from_secs(10), runtime.execute(dag)).await;

    // The contract under test is "doesn't hang", not "succeeds": with no LLM
    // backend registered the Ask node will exhaust retries, so an Err here
    // is fine. A timeout is NOT — that means the LLM permit was held across
    // the child's submission and the worker pool deadlocked.
    //
    // We assert on the timeout result, then deliberately ignore the inner
    // execution result (success or LLM backend error are both acceptable).
    match result {
        Ok(_) => {}
        Err(_) => panic!("scheduler hung — LLM permit likely held across child scheduling"),
    }
}
