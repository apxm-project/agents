//! HTTP-mock integration test for the vLLM graph lifecycle.
//!
//! This test exercises the live contract APXM should rely on when talking to
//! the external fork in `external/vllm`:
//!
//! - `POST /v1/apxm/graphs/register`  — graph registration
//! - `POST /v1/chat/completions`      — node inference with `extra_body.apxm`
//! - `DELETE /v1/apxm/graphs/{id}`    — graph release
//!
//! The key behavioral assertion is that pinning intent travels in
//! `extra_body.apxm.pin_policy`, not through a separate control-plane pin API.

use apxm_backends::llm::backends::vllm::{ApxmGraphHints, GraphAwareVllmBackend, GraphMetadata};
use apxm_backends::llm::backends::{LLMBackend, LLMRequest};
use apxm_core::constants::llm::{api_paths, apxm as apxm_llm};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn versioned_path(route: &str) -> String {
    format!("{}{}", api_paths::VERSION_PREFIX, route)
}

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

fn graph_register_body(graph_id: &str, exec_id: &str, registered_nodes: u32) -> Value {
    json!({
        "object": apxm_llm::OBJECT_GRAPH_REGISTRATION,
        "graph_id": graph_id,
        "execution_id": exec_id,
        "registered_nodes": registered_nodes,
        "critical_path_length": Value::Null,
        "max_parallelism": Value::Null,
        "default_pin_ttl_ms": Value::Null,
    })
}

fn graph_release_body(graph_id: &str) -> Value {
    json!({
        "object": apxm_llm::OBJECT_GRAPH_RELEASE,
        "graph_id": graph_id,
        "released_handles": 0_u32,
        "released_blocks": 0_u32,
        "remaining_handles": 0_u32,
        "remaining_blocks": 0_u32,
    })
}

async fn start_mock_vllm(graph_id: &str, exec_id: &str, model: &str) -> MockServer {
    let server = MockServer::start().await;
    let graph_release_pattern =
        format!(r"^{}{}[^/]+$", versioned_path(api_paths::APXM_GRAPHS), "/");

    Mock::given(method("POST"))
        .and(path(versioned_path(api_paths::APXM_GRAPHS_REGISTER)))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(graph_register_body(graph_id, exec_id, 1)),
        )
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path(versioned_path(api_paths::CHAT_COMPLETIONS)))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion_body(model)))
        .mount(&server)
        .await;

    Mock::given(method("DELETE"))
        .and(path_regex(graph_release_pattern))
        .respond_with(ResponseTemplate::new(200).set_body_json(graph_release_body(graph_id)))
        .expect(1)
        .mount(&server)
        .await;

    server
}

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

fn apxm_payload_from_request(request: &Request) -> Option<Value> {
    if request.method != wiremock::http::Method::POST
        || request.url.path() != versioned_path(api_paths::CHAT_COMPLETIONS)
    {
        return None;
    }
    let body = serde_json::from_slice::<Value>(&request.body).ok()?;
    body.get("extra_body")
        .and_then(|eb| eb.get("apxm"))
        .cloned()
        .or_else(|| body.get("apxm").cloned())
}

fn chat_completion_requests_with_apxm(reqs: &[Request]) -> Vec<&Request> {
    reqs.iter()
        .filter(|r| apxm_payload_from_request(r).is_some())
        .collect()
}

#[tokio::test]
async fn vllm_lifecycle_happy_path_register_generate_release() {
    let graph_id = "graph-test-0001";
    let exec_id = "exec-test-0001";
    let model = "Qwen/Qwen2.5-7B-Instruct";

    let server = start_mock_vllm(graph_id, exec_id, model).await;
    let backend = make_backend(&server.uri(), model).await;

    let metadata = GraphMetadata::new(graph_id, exec_id);
    backend
        .register_graph(metadata)
        .await
        .expect("register_graph happy path");

    let hints =
        ApxmGraphHints::critical_path(graph_id, exec_id, 1, "node-under-test", vec![2, 3], 30_000);
    let request = LLMRequest::new("hello").with_apxm_hints(hints);
    let _response = LLMBackend::generate(&backend, request)
        .await
        .expect("chat completion under mock");

    GraphAwareVllmBackend::release_graph(&backend, graph_id)
        .await
        .expect("release_graph happy path");

    let received = server.received_requests().await.unwrap_or_default();
    let with_apxm = chat_completion_requests_with_apxm(&received);
    assert!(
        !with_apxm.is_empty(),
        "expected at least one {} request carrying extra_body.apxm",
        versioned_path(api_paths::CHAT_COMPLETIONS)
    );

    let apxm = apxm_payload_from_request(with_apxm[0]).expect("apxm payload present");
    assert_eq!(apxm["graph_id"], graph_id);
    assert_eq!(apxm["node_id"], 1);
    assert_eq!(apxm["priority_class"], apxm_llm::PRIORITY_CRITICAL_PATH);
    assert_eq!(apxm["pin_policy"]["mode"], apxm_llm::PIN_MODE_PREFIX);
    assert_eq!(apxm["pin_policy"]["ttl_ms"], 30_000);

    drop(server);
}

#[tokio::test]
async fn vllm_lifecycle_per_node_hint_passthrough() {
    let graph_id = "graph-hint-pass-0001";
    let exec_id = "exec-hint-pass-0001";
    let model = "Qwen/Qwen2.5-7B-Instruct";

    let server = MockServer::start().await;
    let graph_release_pattern =
        format!(r"^{}{}[^/]+$", versioned_path(api_paths::APXM_GRAPHS), "/");

    Mock::given(method("POST"))
        .and(path(versioned_path(api_paths::APXM_GRAPHS_REGISTER)))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(graph_register_body(graph_id, exec_id, 2)),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path(versioned_path(api_paths::CHAT_COMPLETIONS)))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion_body(model)))
        .mount(&server)
        .await;

    Mock::given(method("DELETE"))
        .and(path_regex(graph_release_pattern))
        .respond_with(ResponseTemplate::new(200).set_body_json(graph_release_body(graph_id)))
        .mount(&server)
        .await;

    let backend = make_backend(&server.uri(), model).await;

    backend
        .register_graph(GraphMetadata::new(graph_id, exec_id))
        .await
        .expect("register_graph");

    let hints_a =
        ApxmGraphHints::critical_path(graph_id, exec_id, 10, "node-alpha", vec![11], 30_000);
    let hints_b = ApxmGraphHints::parallel(graph_id, exec_id, 20, "node-beta");

    LLMBackend::generate(
        &backend,
        LLMRequest::new("prompt-a").with_apxm_hints(hints_a),
    )
    .await
    .expect("generate node-alpha");
    LLMBackend::generate(
        &backend,
        LLMRequest::new("prompt-b").with_apxm_hints(hints_b),
    )
    .await
    .expect("generate node-beta");

    GraphAwareVllmBackend::release_graph(&backend, graph_id)
        .await
        .expect("release_graph");

    let received = server.received_requests().await.unwrap_or_default();
    let mut payloads: Vec<Value> = chat_completion_requests_with_apxm(&received)
        .into_iter()
        .filter_map(apxm_payload_from_request)
        .collect();

    assert_eq!(payloads.len(), 2);
    payloads.sort_by_key(|payload| payload["node_id"].as_u64().unwrap_or_default());

    assert_eq!(payloads[0]["node_id"], 10);
    assert_eq!(
        payloads[0]["priority_class"],
        apxm_llm::PRIORITY_CRITICAL_PATH
    );
    assert_eq!(payloads[0]["pin_policy"]["mode"], apxm_llm::PIN_MODE_PREFIX);

    assert_eq!(payloads[1]["node_id"], 20);
    assert_eq!(payloads[1]["priority_class"], apxm_llm::PRIORITY_PARALLEL);
    assert_eq!(payloads[1]["pin_policy"]["mode"], apxm_llm::PIN_MODE_NONE);
}

#[tokio::test]
async fn vllm_lifecycle_explicit_release() {
    let graph_id = "graph-release-0001";
    let exec_id = "exec-release-0001";
    let model = "Qwen/Qwen2.5-7B-Instruct";

    let server = start_mock_vllm(graph_id, exec_id, model).await;
    let backend = make_backend(&server.uri(), model).await;

    backend
        .register_graph(GraphMetadata::new(graph_id, exec_id))
        .await
        .expect("register_graph");

    let hints = ApxmGraphHints::critical_path(graph_id, exec_id, 1, "single-node", vec![], 30_000);
    LLMBackend::generate(&backend, LLMRequest::new("hello").with_apxm_hints(hints))
        .await
        .expect("generate");

    GraphAwareVllmBackend::release_graph(&backend, graph_id)
        .await
        .expect("release_graph");

    let received = server.received_requests().await.unwrap_or_default();
    let delete_count = received
        .iter()
        .filter(|r| r.method == wiremock::http::Method::DELETE)
        .filter(|r| {
            r.url
                .path()
                .starts_with(&versioned_path(api_paths::APXM_GRAPHS))
        })
        .count();
    assert_eq!(delete_count, 1);

    drop(server);
}
