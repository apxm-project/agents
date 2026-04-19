//! HTTP-mock integration test for the vLLM graph lifecycle (PR 2 of the
//! APXM <-> vLLM integration plan).
//!
//! This test exercises the HTTP surface that `VllmGraphLifecycle`
//! (Step 4 of the plan, lives in `apxm-runtime`) drives end-to-end:
//!
//! - `POST /v1/apxm/graphs/register`  — graph registration
//! - `POST /v1/apxm/pins`             — KV-cache pin creation (fire-and-forget)
//! - `GET  /v1/apxm/pins/stats`       — health-check probe
//! - `POST /v1/chat/completions`      — node inference with `extra_body.apxm`
//! - `DELETE /v1/apxm/graphs/{id}`    — graph release
//!
//! ## Scope
//!
//! The cross-crate `VllmGraphLifecycle` wrapper that orchestrates these
//! calls (with a `Drop` panic-safety net) is implemented in
//! `crates/runtime/apxm-runtime/src/runtime/vllm_lifecycle.rs` (Step 4 of
//! the plan). At the time this test was written, Step 4 was being
//! implemented in parallel and the lifecycle module is not available from
//! `apxm-backends` (and `apxm-runtime` is not — and must not become — a
//! dev-dependency of `apxm-backends` since `apxm-runtime` depends on
//! `apxm-backends`, not the other way around).
//!
//! Therefore this test exercises the lifecycle's **HTTP surface** directly
//! against `GraphAwareVllmBackend`. The asserts mirror the spec at the
//! bottom of PR 2 in the plan:
//!
//! - (a) `register_graph` called exactly once
//! - (b) at least one `chat/completions` call carries
//!       `extra_body.apxm.{graph_id, node_id}` populated
//! - (c) `release_graph` called exactly once on the happy path
//!
//! Assertion (d) — `release_graph` fires when the execution future is
//! dropped mid-flight (Drop guard) — is exercised in
//! `apxm-runtime/tests/vllm_lifecycle_drop.rs` instead, since the Drop
//! guard is a property of `VllmGraphLifecycle` (defined in
//! `apxm-runtime`) and cannot be imported from this crate.

use apxm_backends::llm::backends::vllm::{ApxmGraphHints, GraphAwareVllmBackend, GraphMetadata};
use apxm_backends::llm::backends::{LLMBackend, LLMRequest};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

// ---------------------------------------------------------------------------
// Stub helpers
// ---------------------------------------------------------------------------

/// Minimal OpenAI-compatible chat completion body.
fn chat_completion_body(model: &str) -> Value {
    json!({
        "id": "chatcmpl-mock-0001",
        "object": "chat.completion",
        "created": 1_700_000_000_u64,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "ok"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 4, "completion_tokens": 1, "total_tokens": 5}
    })
}

fn pin_stats_body() -> Value {
    json!({
        "object": "apxm.pins.stats",
        "active_pins": 0_u64,
        "pinned_blocks": 0_u64,
        "pin_hits": 0_u64,
        "pin_misses": 0_u64,
        "pin_hit_ratio": 0.0_f64,
        "pin_expirations": 0_u64,
        "memory_pressure_releases": 0_u64,
        "total_lookups": 0_u64
    })
}

fn graph_register_body(graph_id: &str, exec_id: &str, registered_nodes: u32) -> Value {
    json!({
        "object": "apxm.graph.registration",
        "graph_id": graph_id,
        "execution_id": exec_id,
        "registered_nodes": registered_nodes,
    })
}

fn graph_release_body(graph_id: &str) -> Value {
    json!({
        "object": "apxm.graph.release",
        "graph_id": graph_id,
        "released_handles": 0_u32,
        "released_blocks": 0_u32,
    })
}

fn pin_create_body(graph_id: &str, node_id: u32, ttl_ms: u64) -> Value {
    json!({
        "object": "apxm.pin",
        "graph_id": graph_id,
        "node_id": node_id,
        "reuse_group": Value::Null,
        "request_id": "req-mock-0001",
        "ttl_ms": ttl_ms,
        "expiry_ts": 0.0_f64,
    })
}

/// Install all five vLLM stubs on a fresh `MockServer`. Returns the server.
async fn start_mock_vllm(graph_id: &str, exec_id: &str, model: &str) -> MockServer {
    let server = MockServer::start().await;

    // 1. Graph registration
    Mock::given(method("POST"))
        .and(path("/v1/apxm/graphs/register"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(graph_register_body(graph_id, exec_id, 1)),
        )
        .expect(1)
        .mount(&server)
        .await;

    // 2. Pin creation (fire-and-forget; may or may not be called — don't gate)
    Mock::given(method("POST"))
        .and(path("/v1/apxm/pins"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(pin_create_body(graph_id, 1, 30_000)),
        )
        .mount(&server)
        .await;

    // 3. Pin stats (health-check probe)
    Mock::given(method("GET"))
        .and(path("/v1/apxm/pins/stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pin_stats_body()))
        .mount(&server)
        .await;

    // 4. Chat completions
    //
    // Convention: `base_url` includes the `/v1` prefix (e.g.
    // `http://127.0.0.1:PORT/v1`). The inner `OpenAIBackend` appends
    // `/chat/completions` directly, so the wire path is
    // `/v1/chat/completions`. The outer extension methods append
    // `/apxm/...`, producing `/v1/apxm/...`.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion_body(model)))
        .mount(&server)
        .await;

    // 5. Graph release
    Mock::given(method("DELETE"))
        .and(path_regex(r"^/v1/apxm/graphs/[^/]+$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(graph_release_body(graph_id)),
        )
        .expect(1)
        .mount(&server)
        .await;

    server
}

/// Build a `GraphAwareVllmBackend` pointed at the given mock server URI.
///
/// Convention: endpoints are stored with the `/v1` prefix. Both the inner
/// `OpenAIBackend` and the outer extension methods share the same
/// `base_url` config key. The inner client appends `/chat/completions`;
/// the outer extension calls append `/apxm/...`. So the `base_url` passed
/// here should be `{server_root}/v1`.
async fn make_backend(server_uri: &str, model: &str) -> GraphAwareVllmBackend {
    let base_url = format!("{server_uri}/v1");
    GraphAwareVllmBackend::new(
        "test-key",
        Some(json!({
            "base_url": base_url,
            "model": model,
        })),
    )
    .await
    .expect("construct GraphAwareVllmBackend")
}

/// Locate every recorded `POST /v1/chat/completions` request whose JSON body's
/// `extra_body.apxm` object has both `graph_id` and `node_id` populated.
fn chat_completion_requests_with_apxm(reqs: &[Request]) -> Vec<&Request> {
    reqs.iter()
        .filter(|r| r.method == wiremock::http::Method::POST)
        .filter(|r| r.url.path() == "/v1/chat/completions")
        .filter(|r| {
            let Ok(body) = serde_json::from_slice::<Value>(&r.body) else {
                return false;
            };
            // Some clients merge extra_body into the top-level request body
            // (the OpenAI-compatible flattening). Look in both places.
            let apxm = body
                .get("extra_body")
                .and_then(|eb| eb.get("apxm"))
                .or_else(|| body.get("apxm"));
            let Some(apxm) = apxm else { return false };
            apxm.get("graph_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .is_some()
                && apxm.get("node_id").and_then(Value::as_u64).is_some()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// (a), (b), (c) — happy path: register -> generate -> release.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn vllm_lifecycle_happy_path_register_generate_release() {
    let graph_id = "graph-test-0001";
    let exec_id = "exec-test-0001";
    let model = "Qwen/Qwen2.5-7B-Instruct";

    let server = start_mock_vllm(graph_id, exec_id, model).await;
    let backend = make_backend(&server.uri(), model).await;

    // (a) register_graph called exactly once.
    let metadata = GraphMetadata::new(graph_id, exec_id);
    backend
        .register_graph(metadata)
        .await
        .expect("register_graph happy path");

    // (b) at least one chat/completions call carries extra_body.apxm.{graph_id,
    // node_id} populated.
    let hints = ApxmGraphHints::critical_path(
        graph_id,
        exec_id,
        1, // node_id
        "node-under-test",
        vec![2, 3],
        30_000,
    );
    let request = LLMRequest::new("hello").with_apxm_hints(hints);
    let _response = LLMBackend::generate(&backend, request)
        .await
        .expect("chat completion under mock");

    // (c) release_graph called exactly once on the happy path.
    GraphAwareVllmBackend::release_graph(&backend, graph_id)
        .await
        .expect("release_graph happy path");

    // Verify (b) by inspecting the recorded requests against the mock server.
    let received = server.received_requests().await.unwrap_or_default();
    let with_apxm = chat_completion_requests_with_apxm(&received);
    assert!(
        !with_apxm.is_empty(),
        "expected at least one /v1/chat/completions request carrying \
         extra_body.apxm.{{graph_id, node_id}}; saw {} chat-completion request(s) total",
        received
            .iter()
            .filter(|r| r.url.path() == "/v1/chat/completions")
            .count()
    );

    // (a) and (c) are enforced by `.expect(1)` on the mounted mocks; they
    // are checked when the `MockServer` is dropped at end-of-scope. Drop
    // explicitly here so a failure surfaces as a panic from this test
    // function rather than a dangling teardown.
    drop(server);
}

// ---------------------------------------------------------------------------
// (b) — per-node hint passthrough: two completions with distinct node_ids.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn vllm_lifecycle_per_node_hint_passthrough() {
    let graph_id = "graph-hint-pass-0001";
    let exec_id = "exec-hint-pass-0001";
    let model = "Qwen/Qwen2.5-7B-Instruct";

    // Stand up the mock without strict call-count on register/release since
    // this test focuses solely on the chat-completion payloads.
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/apxm/graphs/register"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(graph_register_body(graph_id, exec_id, 2)),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/apxm/pins"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(pin_create_body(graph_id, 1, 30_000)),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/apxm/pins/stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pin_stats_body()))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion_body(model)))
        .mount(&server)
        .await;

    Mock::given(method("DELETE"))
        .and(path_regex(r"^/v1/apxm/graphs/[^/]+$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(graph_release_body(graph_id)),
        )
        .mount(&server)
        .await;

    let backend = make_backend(&server.uri(), model).await;

    // Register.
    let metadata = GraphMetadata::new(graph_id, exec_id);
    backend
        .register_graph(metadata)
        .await
        .expect("register_graph");

    // Drive two completions with distinct node_ids and priority classes.
    let hints_a = ApxmGraphHints::critical_path(
        graph_id, exec_id, 10, "node-alpha", vec![11], 30_000,
    );
    let hints_b = ApxmGraphHints::parallel(graph_id, exec_id, 20, "node-beta");

    let req_a = LLMRequest::new("prompt-a").with_apxm_hints(hints_a);
    let req_b = LLMRequest::new("prompt-b").with_apxm_hints(hints_b);

    LLMBackend::generate(&backend, req_a)
        .await
        .expect("generate node-alpha");
    LLMBackend::generate(&backend, req_b)
        .await
        .expect("generate node-beta");

    // Release.
    GraphAwareVllmBackend::release_graph(&backend, graph_id)
        .await
        .expect("release_graph");

    // Inspect captured requests.
    let received = server.received_requests().await.unwrap_or_default();
    let with_apxm = chat_completion_requests_with_apxm(&received);

    assert_eq!(
        with_apxm.len(),
        2,
        "expected exactly 2 chat-completion requests carrying APXM hints, got {}",
        with_apxm.len()
    );

    // Extract the (node_id, priority_class) pairs and sort by node_id.
    let mut pairs: Vec<(u64, String)> = with_apxm
        .iter()
        .map(|r| {
            let body: Value = serde_json::from_slice(&r.body).unwrap();
            let apxm = body
                .get("extra_body")
                .and_then(|eb| eb.get("apxm"))
                .or_else(|| body.get("apxm"))
                .expect("apxm field present");
            let nid = apxm["node_id"].as_u64().expect("node_id is u64");
            let pclass = apxm["priority_class"]
                .as_str()
                .expect("priority_class is string")
                .to_string();
            (nid, pclass)
        })
        .collect();
    pairs.sort_by_key(|(nid, _)| *nid);

    assert_eq!(pairs[0], (10, "critical_path".to_string()));
    assert_eq!(pairs[1], (20, "parallel".to_string()));
}

// ---------------------------------------------------------------------------
// (c) — explicit-release happy path (focused).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn vllm_lifecycle_explicit_release() {
    let graph_id = "graph-release-0001";
    let exec_id = "exec-release-0001";
    let model = "Qwen/Qwen2.5-7B-Instruct";

    let server = start_mock_vllm(graph_id, exec_id, model).await;
    let backend = make_backend(&server.uri(), model).await;

    // Register.
    let metadata = GraphMetadata::new(graph_id, exec_id);
    backend
        .register_graph(metadata)
        .await
        .expect("register_graph");

    // One generate.
    let hints = ApxmGraphHints::critical_path(
        graph_id, exec_id, 1, "single-node", vec![], 30_000,
    );
    let request = LLMRequest::new("hello").with_apxm_hints(hints);
    LLMBackend::generate(&backend, request)
        .await
        .expect("generate");

    // Explicit release.
    GraphAwareVllmBackend::release_graph(&backend, graph_id)
        .await
        .expect("release_graph");

    // Assert exactly one DELETE to /v1/apxm/graphs/{id} was recorded.
    let received = server.received_requests().await.unwrap_or_default();
    let delete_count = received
        .iter()
        .filter(|r| r.method == wiremock::http::Method::DELETE)
        .filter(|r| r.url.path().starts_with("/v1/apxm/graphs/"))
        .count();
    assert_eq!(
        delete_count, 1,
        "expected exactly 1 DELETE /v1/apxm/graphs/*, got {delete_count}"
    );

    // The `.expect(1)` on start_mock_vllm's DELETE mount also validates on
    // drop, so drop explicitly.
    drop(server);
}

// ---------------------------------------------------------------------------
// (d) — drop-safety: Drop guard fires release without explicit call.
// ---------------------------------------------------------------------------

/// Minimal guard that mirrors `VllmGraphLifecycle`'s Drop contract: if the
/// graph was never explicitly released, the destructor spawns a best-effort
/// DELETE. This lives in the test because `VllmGraphLifecycle` is defined in
/// `apxm-runtime` and cannot be imported from `apxm-backends`.
struct DropGuard {
    backend: GraphAwareVllmBackend,
    graph_id: String,
    released: std::sync::atomic::AtomicBool,
}

impl DropGuard {
    fn new(backend: GraphAwareVllmBackend, graph_id: impl Into<String>) -> Self {
        Self {
            backend,
            graph_id: graph_id.into(),
            released: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl Drop for DropGuard {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        if !self.released.swap(true, Ordering::AcqRel) {
            // Mirror VllmGraphLifecycle: best-effort release via tokio::spawn.
            let client = reqwest::Client::new();
            let url = format!(
                "{}/apxm/graphs/{}",
                self.backend.graph_registration_url()
                    .trim_end_matches("/apxm/graphs/register"),
                self.graph_id
            );
            tokio::spawn(async move {
                let _ = client.delete(&url).send().await;
            });
        }
    }
}

#[tokio::test]
async fn vllm_lifecycle_drop_safety() {
    let graph_id = "graph-drop-0001";
    let exec_id = "exec-drop-0001";
    let model = "Qwen/Qwen2.5-7B-Instruct";

    // Set up mock — allow 1..= DELETEs (the drop guard should fire exactly 1).
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/apxm/graphs/register"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(graph_register_body(graph_id, exec_id, 1)),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/apxm/pins"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(pin_create_body(graph_id, 1, 30_000)),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/apxm/pins/stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pin_stats_body()))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion_body(model)))
        .mount(&server)
        .await;

    Mock::given(method("DELETE"))
        .and(path_regex(r"^/v1/apxm/graphs/[^/]+$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(graph_release_body(graph_id)),
        )
        .expect(1)
        .mount(&server)
        .await;

    let backend = make_backend(&server.uri(), model).await;

    // Register.
    let metadata = GraphMetadata::new(graph_id, exec_id);
    backend
        .register_graph(metadata)
        .await
        .expect("register_graph");

    // Generate once.
    let hints = ApxmGraphHints::critical_path(
        graph_id, exec_id, 1, "drop-test-node", vec![], 30_000,
    );
    let request = LLMRequest::new("hello").with_apxm_hints(hints);
    LLMBackend::generate(&backend, request)
        .await
        .expect("generate");

    // Wrap in DropGuard and drop WITHOUT calling release_graph.
    {
        let _guard = DropGuard::new(backend, graph_id);
        // _guard dropped here — Drop spawns the DELETE.
    }

    // Give the spawned task time to execute the DELETE.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify the mock server received at least one DELETE.
    let received = server.received_requests().await.unwrap_or_default();
    let delete_count = received
        .iter()
        .filter(|r| r.method == wiremock::http::Method::DELETE)
        .filter(|r| r.url.path().starts_with("/v1/apxm/graphs/"))
        .count();
    assert!(
        delete_count >= 1,
        "expected at least 1 DELETE /v1/apxm/graphs/* from drop guard, got {delete_count}"
    );

    // `.expect(1)` on the DELETE mock verifies exactly-once semantics.
    drop(server);
}
