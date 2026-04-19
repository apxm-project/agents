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
//! - (d) `release_graph` fires when the execution future is dropped
//!       mid-flight (Drop guard) — **gated on Step 4** because the Drop
//!       guard is a property of `VllmGraphLifecycle`, not of the raw
//!       backend. This sub-test is `#[ignore]`'d with a TODO; the
//!       maintainer should un-ignore it once Step 4 lands and can be
//!       imported here (or, more likely, move it to
//!       `apxm-runtime/tests/`).

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
    // Note: the inner `OpenAIBackend` appends `/chat/completions` directly
    // to the configured `base_url`, while the outer `GraphAwareVllmBackend`
    // hardcodes `/v1/apxm/...` for the extension surface. Both share the
    // same `base_url` config value (see vllm/backend.rs `pub async fn new`),
    // so the canonical setup — and the one used by the existing
    // `vllm_integration.rs` test — is to point `base_url` at the server
    // root and let chat completions land at `{root}/chat/completions`.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
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

/// Build a `GraphAwareVllmBackend` pointed at the given `base_url`.
///
/// Both the inner `OpenAIBackend` and the outer extension methods read the
/// same `base_url` config key. The inner client appends `/chat/completions`
/// to it; the outer extension calls hardcode `/v1/apxm/...`. So `base_url`
/// must be the server root with no trailing path segment — matching the
/// shape used by `vllm_integration.rs::test_graph_registration_request_structure`.
async fn make_backend(base_url: &str, model: &str) -> GraphAwareVllmBackend {
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
        .filter(|r| r.url.path() == "/chat/completions")
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
            .filter(|r| r.url.path() == "/chat/completions")
            .count()
    );

    // (a) and (c) are enforced by `.expect(1)` on the mounted mocks; they
    // are checked when the `MockServer` is dropped at end-of-scope. Drop
    // explicitly here so a failure surfaces as a panic from this test
    // function rather than a dangling teardown.
    drop(server);
}

// ---------------------------------------------------------------------------
// (d) — Drop guard: release fires when the execution future is dropped
// mid-flight. This is a property of `VllmGraphLifecycle` (Step 4 of the
// plan), which is not yet available from this crate. Ignored until Step 4
// lands; the maintainer should un-ignore (and likely move to
// `apxm-runtime/tests/`) once the lifecycle type is importable.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "Step 4 (VllmGraphLifecycle Drop guard) not yet merged. \
            TODO: un-ignore and possibly relocate to apxm-runtime/tests/ \
            once VllmGraphLifecycle is importable. The Drop guard cannot \
            be exercised from apxm-backends in isolation because the \
            wrapper that owns it lives one crate up."]
async fn vllm_lifecycle_drop_guard_releases_on_cancel() {
    let graph_id = "graph-test-drop-0001";
    let exec_id = "exec-test-drop-0001";
    let model = "Qwen/Qwen2.5-7B-Instruct";

    let server = start_mock_vllm(graph_id, exec_id, model).await;
    let _backend = make_backend(&server.uri(), model).await;

    // Pseudocode for the eventual implementation (compiles against the
    // expected Step 4 API, kept here as a reference for the maintainer):
    //
    //     let backend_arc: Arc<dyn LLMBackend> = Arc::new(_backend);
    //     let lifecycle = VllmGraphLifecycle::register(
    //         backend_arc.clone(),
    //         graph_id.to_string(),
    //         exec_id.to_string(),
    //         &dag,
    //     )
    //     .await
    //     .expect("register lifecycle");
    //
    //     let handle = tokio::spawn(async move {
    //         // Hold the lifecycle inside a long-running future, then we'll
    //         // drop the JoinHandle (and abort) before it completes.
    //         let _lc = lifecycle;
    //         tokio::time::sleep(Duration::from_secs(60)).await;
    //     });
    //     tokio::time::sleep(Duration::from_millis(50)).await;
    //     handle.abort();
    //     // Give the Drop's tokio::spawn a tick to flush.
    //     tokio::time::sleep(Duration::from_millis(200)).await;
    //
    //     let received = server.received_requests().await.unwrap_or_default();
    //     let releases = received
    //         .iter()
    //         .filter(|r| r.method == wiremock::http::Method::DELETE)
    //         .filter(|r| r.url.path().starts_with("/v1/apxm/graphs/"))
    //         .count();
    //     assert_eq!(releases, 1, "Drop guard must fire exactly one DELETE");
    //
    //     drop(server);

    // Until Step 4 lands, fail loudly if someone un-ignores prematurely.
    panic!(
        "vllm_lifecycle_drop_guard_releases_on_cancel is gated on Step 4 \
         (VllmGraphLifecycle). Un-ignore once that lands."
    );
}
