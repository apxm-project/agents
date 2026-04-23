//! Drop-guard integration test for `VllmGraphLifecycle` (Step 4 of the
//! APXM ↔ vLLM integration plan, assertion (d) of the PR2 HTTP-mock spec).
//!
//! This test exercises the property that when a `VllmGraphLifecycle` guard
//! is dropped without an explicit `release().await` (e.g. on panic or future
//! cancellation), the `Drop` impl detaches a best-effort `release_graph`
//! call so the vLLM server doesn't leak a pinned graph.
//!
//! It lives here (not in `apxm-backends/tests/`) because
//! `VllmGraphLifecycle` is defined in `apxm-runtime` and `apxm-backends`
//! cannot import it without inverting the dependency direction.

use std::sync::Arc;
use std::time::Duration;

use apxm_backends::llm::backends::traits::LLMBackend;
use apxm_backends::llm::backends::vllm::GraphAwareVllmBackend;
use apxm_core::constants::llm::{api_paths, apxm as apxm_llm};
use apxm_core::types::{AISOperationType, ExecutionDag, Node};
use apxm_runtime::vllm_lifecycle::VllmGraphLifecycle;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn versioned_path(route: &str) -> String {
    format!("{}{}", api_paths::VERSION_PREFIX, route)
}

// ---------------------------------------------------------------------------
// Stub helpers (kept self-contained; mirrors apxm-backends/tests/vllm_lifecycle_http.rs).
// ---------------------------------------------------------------------------

fn graph_register_body(graph_id: &str, exec_id: &str) -> Value {
    json!({
        "object": apxm_llm::OBJECT_GRAPH_REGISTRATION,
        "graph_id": graph_id,
        "execution_id": exec_id,
        "registered_nodes": 1,
    })
}

fn graph_release_body(graph_id: &str) -> Value {
    json!({
        "object": apxm_llm::OBJECT_GRAPH_RELEASE,
        "graph_id": graph_id,
        "released_handles": 0_u32,
        "released_blocks": 0_u32,
    })
}

async fn start_mock_vllm(graph_id: &str, exec_id: &str) -> MockServer {
    let server = MockServer::start().await;
    let graph_release_pattern =
        format!(r"^{}{}[^/]+$", versioned_path(api_paths::APXM_GRAPHS), "/");

    Mock::given(method("POST"))
        .and(path(versioned_path(api_paths::APXM_GRAPHS_REGISTER)))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(graph_register_body(graph_id, exec_id)),
        )
        .mount(&server)
        .await;

    Mock::given(method("DELETE"))
        .and(path_regex(graph_release_pattern))
        .respond_with(ResponseTemplate::new(200).set_body_json(graph_release_body(graph_id)))
        .mount(&server)
        .await;

    server
}

async fn make_backend(server_uri: &str) -> Arc<dyn LLMBackend> {
    // Convention: `base_url` includes the `/v1` prefix; the extension methods
    // append `/apxm/...` to it. Wiremock mounts the full `/v1/apxm/...` paths.
    let base_url = format!("{server_uri}/v1");
    let backend = GraphAwareVllmBackend::new(
        "test-key",
        Some(json!({
            "base_url": base_url,
            "model": "Qwen/Qwen2.5-7B-Instruct",
        })),
    )
    .await
    .expect("construct GraphAwareVllmBackend");
    Arc::new(backend)
}

fn single_node_dag(name: &str) -> ExecutionDag {
    let mut dag = ExecutionDag::new();
    dag.metadata.name = Some(name.to_string());
    dag.add_node(Node::new(0, AISOperationType::Ask))
        .expect("add node");
    dag.entry_nodes.push(0);
    dag.exit_nodes.push(0);
    dag
}

fn count_releases(received: &[wiremock::Request]) -> usize {
    received
        .iter()
        .filter(|r| r.method == wiremock::http::Method::DELETE)
        .filter(|r| {
            r.url
                .path()
                .starts_with(&versioned_path(api_paths::APXM_GRAPHS))
        })
        .count()
}

// ---------------------------------------------------------------------------
// Happy path: explicit release() => exactly one DELETE; no Drop fire.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn vllm_lifecycle_explicit_release_fires_once() {
    let graph_id = "graph-happy-0001";
    let exec_id = "exec-happy-0001";
    let server = start_mock_vllm(graph_id, exec_id).await;
    let backend = make_backend(&server.uri()).await;
    let dag = single_node_dag("happy");

    let lifecycle =
        VllmGraphLifecycle::register(backend, graph_id.to_string(), exec_id.to_string(), &dag)
            .await
            .expect("register lifecycle");

    lifecycle.release().await.expect("explicit release");

    // Even after Drop runs, the second release attempt should be a no-op
    // (the AtomicBool guard).
    drop(lifecycle);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let received = server.received_requests().await.unwrap_or_default();
    assert_eq!(
        count_releases(&received),
        1,
        "explicit release + drop must produce exactly one DELETE"
    );
}

// ---------------------------------------------------------------------------
// Drop guard: dropping without release() must still fire one DELETE.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn vllm_lifecycle_drop_guard_releases_when_dropped() {
    let graph_id = "graph-drop-0001";
    let exec_id = "exec-drop-0001";
    let server = start_mock_vllm(graph_id, exec_id).await;
    let backend = make_backend(&server.uri()).await;
    let dag = single_node_dag("drop");

    {
        let lifecycle =
            VllmGraphLifecycle::register(backend, graph_id.to_string(), exec_id.to_string(), &dag)
                .await
                .expect("register lifecycle");
        // Intentionally drop without calling release().
        drop(lifecycle);
    }

    // Give the detached `tokio::spawn` inside Drop a tick to flush.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let received = server.received_requests().await.unwrap_or_default();
    assert_eq!(
        count_releases(&received),
        1,
        "Drop guard must fire exactly one DELETE when release() was never called"
    );
}

// ---------------------------------------------------------------------------
// Drop guard on cancelled future: aborting a tokio task that owns the
// lifecycle must still fire DELETE.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn vllm_lifecycle_drop_guard_releases_on_task_cancel() {
    let graph_id = "graph-cancel-0001";
    let exec_id = "exec-cancel-0001";
    let server = start_mock_vllm(graph_id, exec_id).await;
    let backend = make_backend(&server.uri()).await;
    let dag = single_node_dag("cancel");

    let lifecycle =
        VllmGraphLifecycle::register(backend, graph_id.to_string(), exec_id.to_string(), &dag)
            .await
            .expect("register lifecycle");

    let handle = tokio::spawn(async move {
        let _lc = lifecycle;
        // Long sleep — we'll abort before it completes.
        tokio::time::sleep(Duration::from_secs(60)).await;
    });

    // Let the task start, then abort.
    tokio::time::sleep(Duration::from_millis(50)).await;
    handle.abort();
    let _ = handle.await; // join to ensure Drop has run

    // Give the detached release task a tick to flush.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let received = server.received_requests().await.unwrap_or_default();
    assert_eq!(
        count_releases(&received),
        1,
        "Drop guard must fire exactly one DELETE when the owning task is aborted"
    );
}
