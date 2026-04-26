//! HTTP-mock integration test for the vLLM graph lifecycle.
//!
//! This test exercises the live contract APXM should rely on when talking to
//! the external fork in `external/vllm`:
//!
//! - `POST /v1/apxm/graphs/register`  — graph registration
//! - `POST /v1/chat/completions`      — node inference with `vllm_xargs.apxm`
//! - `DELETE /v1/apxm/graphs/{id}`    — graph release
//!
//! The key behavioral assertion is that pinning intent travels in
//! `vllm_xargs.apxm.pin_policy`, not through a separate control-plane pin API.

use apxm_backends::llm::backends::vllm::{
    ApxmGraphHints, GraphAwareVllmBackend, GraphMetadata, GraphStatusResponse,
};
use apxm_backends::llm::backends::{LLMBackend, LLMRequest};
use apxm_core::constants::http::headers;
use apxm_core::constants::llm::{api_paths, apxm as apxm_llm, config_keys};
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
        apxm_llm::OBJECT: apxm_llm::OBJECT_GRAPH_REGISTRATION,
        apxm_llm::GRAPH_ID: graph_id,
        apxm_llm::EXECUTION_ID: exec_id,
        apxm_llm::REGISTERED_NODES: registered_nodes,
        apxm_llm::CRITICAL_PATH_LENGTH: Value::Null,
        apxm_llm::MAX_PARALLELISM: Value::Null,
        apxm_llm::DEFAULT_PIN_TTL_MS: Value::Null,
    })
}

fn graph_release_body(graph_id: &str) -> Value {
    json!({
        apxm_llm::OBJECT: apxm_llm::OBJECT_GRAPH_RELEASE,
        apxm_llm::GRAPH_ID: graph_id,
        apxm_llm::RELEASED_HANDLES: 0_u32,
        apxm_llm::RELEASED_BLOCKS: 0_u32,
        apxm_llm::REMAINING_HANDLES: 0_u32,
        apxm_llm::REMAINING_BLOCKS: 0_u32,
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

#[tokio::test]
async fn vllm_graph_control_requests_use_backend_transport_headers() {
    const API_KEY: &str = "graph-control-key";
    const HEADER_NAME: &str = "x-apxm-gateway";
    const HEADER_VALUE: &str = "tenant-a";

    let graph_id = "graph-auth-headers";
    let exec_id = "exec-auth-headers";
    let model = "Qwen/Qwen2.5-7B-Instruct";
    let server = start_mock_vllm(graph_id, exec_id, model).await;
    let backend = GraphAwareVllmBackend::new(
        API_KEY,
        Some(json!({
            "base_url": format!("{}/v1", server.uri()),
            "model": model,
            config_keys::EXTRA_HEADERS: {
                HEADER_NAME: HEADER_VALUE,
            },
        })),
    )
    .await
    .expect("construct GraphAwareVllmBackend");

    backend
        .register_graph(GraphMetadata::new(graph_id, exec_id))
        .await
        .expect("register_graph");
    GraphAwareVllmBackend::release_graph(&backend, graph_id)
        .await
        .expect("release_graph");

    let received = server.received_requests().await.unwrap_or_default();
    let control_requests: Vec<&Request> = received
        .iter()
        .filter(|request| {
            request
                .url
                .path()
                .starts_with(&versioned_path(api_paths::APXM_GRAPHS))
        })
        .collect();
    assert_eq!(control_requests.len(), 2);
    for request in control_requests {
        let expected_auth = format!("Bearer {API_KEY}");
        let auth = request
            .headers
            .get(headers::AUTHORIZATION)
            .and_then(|value| value.to_str().ok());
        assert_eq!(auth, Some(expected_auth.as_str()));
        let gateway = request
            .headers
            .get(HEADER_NAME)
            .and_then(|value| value.to_str().ok());
        assert_eq!(gateway, Some(HEADER_VALUE));
    }
}

fn apxm_payload_from_request(request: &Request) -> Option<Value> {
    if request.method != wiremock::http::Method::POST
        || request.url.path() != versioned_path(api_paths::CHAT_COMPLETIONS)
    {
        return None;
    }
    let body = serde_json::from_slice::<Value>(&request.body).ok()?;
    body.get(apxm_llm::VLLM_XARGS)
        .and_then(|xargs| xargs.get(apxm_llm::HINTS_FIELD))
        .cloned()
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
        "expected at least one {} request carrying vllm_xargs.apxm",
        versioned_path(api_paths::CHAT_COMPLETIONS)
    );

    let apxm = apxm_payload_from_request(with_apxm[0]).expect("apxm payload present");
    assert_eq!(apxm[apxm_llm::GRAPH_ID], graph_id);
    assert_eq!(apxm[apxm_llm::NODE_ID], 1);
    assert_eq!(
        apxm[apxm_llm::PRIORITY_CLASS],
        apxm_llm::PRIORITY_CRITICAL_PATH
    );
    assert_eq!(
        apxm[apxm_llm::PIN_POLICY][apxm_llm::PIN_POLICY_MODE],
        apxm_llm::PIN_MODE_PREFIX
    );
    assert_eq!(
        apxm[apxm_llm::PIN_POLICY][apxm_llm::PIN_POLICY_TTL_MS],
        30_000
    );

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
    payloads.sort_by_key(|payload| payload[apxm_llm::NODE_ID].as_u64().unwrap_or_default());

    assert_eq!(payloads[0][apxm_llm::NODE_ID], 10);
    assert_eq!(
        payloads[0][apxm_llm::PRIORITY_CLASS],
        apxm_llm::PRIORITY_CRITICAL_PATH
    );
    assert_eq!(
        payloads[0][apxm_llm::PIN_POLICY][apxm_llm::PIN_POLICY_MODE],
        apxm_llm::PIN_MODE_PREFIX
    );

    assert_eq!(payloads[1][apxm_llm::NODE_ID], 20);
    assert_eq!(
        payloads[1][apxm_llm::PRIORITY_CLASS],
        apxm_llm::PRIORITY_PARALLEL
    );
    assert_eq!(
        payloads[1][apxm_llm::PIN_POLICY][apxm_llm::PIN_POLICY_MODE],
        apxm_llm::PIN_MODE_NONE
    );
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

/// Verify `get_graph_status` GET fires before `release_graph` DELETE.
#[tokio::test]
async fn vllm_get_graph_status_fires_before_release() {
    let graph_id = "graph-status-before-release";
    let exec_id = "exec-status-0001";
    let model = "Qwen/Qwen2.5-7B-Instruct";

    let server = MockServer::start().await;
    let graph_release_pattern =
        format!(r"^{}{}[^/]+$", versioned_path(api_paths::APXM_GRAPHS), "/");

    // Register mock
    Mock::given(method("POST"))
        .and(path(versioned_path(api_paths::APXM_GRAPHS_REGISTER)))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(graph_register_body(graph_id, exec_id, 1)),
        )
        .mount(&server)
        .await;

    // Graph status GET mock
    let graph_status_path = format!(
        "{}{}/{}",
        api_paths::VERSION_PREFIX,
        api_paths::APXM_GRAPHS,
        graph_id
    );
    // Typed + trait method each fire one GET = 2 total
    let status_fixture = GraphStatusResponse {
        object: apxm_llm::OBJECT_GRAPH_STATUS.to_owned(),
        graph_id: graph_id.to_owned(),
        registered: true,
        pinned_handles: 4,
        pinned_blocks: 12,
        node_count: Some(3),
        critical_path_length: Some(2),
    };
    Mock::given(method("GET"))
        .and(path(&graph_status_path))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::to_value(&status_fixture).unwrap()),
        )
        .expect(2)
        .mount(&server)
        .await;

    // Release DELETE mock
    Mock::given(method("DELETE"))
        .and(path_regex(graph_release_pattern))
        .respond_with(ResponseTemplate::new(200).set_body_json(graph_release_body(graph_id)))
        .expect(1)
        .mount(&server)
        .await;

    let backend = make_backend(&server.uri(), model).await;

    // Register
    backend
        .register_graph(apxm_backends::llm::backends::vllm::GraphMetadata::new(
            graph_id, exec_id,
        ))
        .await
        .expect("register_graph");

    // Get status (typed) before release
    let status: GraphStatusResponse = backend
        .get_graph_status_typed(graph_id)
        .await
        .expect("get_graph_status_typed");
    assert!(status.registered);
    assert_eq!(status.pinned_blocks, 12);
    assert_eq!(status.pinned_handles, 4);

    // Also test the trait method
    let trait_status =
        apxm_backends::llm::backends::LLMBackend::get_graph_status(&backend, graph_id)
            .await
            .expect("trait get_graph_status");
    assert!(trait_status.is_some());
    let status_snapshot = trait_status.unwrap();
    assert_eq!(status_snapshot.pinned_blocks, 12);

    // Release
    apxm_backends::llm::backends::vllm::GraphAwareVllmBackend::release_graph(&backend, graph_id)
        .await
        .expect("release_graph");

    // Verify request ordering: first GET must come before DELETE
    let received = server.received_requests().await.unwrap_or_default();
    let mut first_get_idx = None;
    let mut delete_idx = None;
    for (i, req) in received.iter().enumerate() {
        if req.method == wiremock::http::Method::GET
            && req.url.path().contains(graph_id)
            && first_get_idx.is_none()
        {
            first_get_idx = Some(i);
        }
        if req.method == wiremock::http::Method::DELETE && req.url.path().contains(graph_id) {
            delete_idx = Some(i);
        }
    }
    let get_i = first_get_idx.expect("GET request for graph status must have been sent");
    let delete_i = delete_idx.expect("DELETE request for graph release must have been sent");
    assert!(
        get_i < delete_i,
        "GET /graphs/{graph_id} (index {get_i}) must fire before DELETE (index {delete_i})"
    );

    drop(server);
}

/// Stock-vLLM detection: a 404 from `/v1/apxm/graphs/__apxm_probe__` must hard-fail
/// `health_check` by default, so APXM never silently runs without graph-aware
/// scheduling on a server that drops `vllm_xargs.apxm`.
#[tokio::test]
async fn vllm_health_check_hard_fails_when_apxm_endpoints_missing() {
    let server = MockServer::start().await;

    // Inner OpenAI health check probes /v1/models — keep it happy.
    Mock::given(method("GET"))
        .and(path(versioned_path(api_paths::MODELS)))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [],
        })))
        .mount(&server)
        .await;

    // Stock vLLM returns 404 for the APXM probe path.
    let probe_path = format!(
        "{}{}/{}",
        api_paths::VERSION_PREFIX,
        api_paths::APXM_GRAPHS,
        apxm_core::constants::llm::vllm::APXM_PROBE_GRAPH_ID,
    );
    Mock::given(method("GET"))
        .and(path(probe_path))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let backend = make_backend(&server.uri(), "Qwen/Qwen2.5-7B-Instruct").await;

    LLMBackend::health_check(&backend)
        .await
        .expect_err("health_check must hard-fail on stock vLLM by default");

    drop(server);
}

/// `get_graph_status` returns pin telemetry from the fork before release.
/// Verifies the GET fires, returns the expected shape, and DELETE follows.
#[tokio::test]
async fn vllm_graph_status_collected_before_release() {
    let graph_id = "graph-pin-telemetry-001";
    let exec_id = "exec-pin-telemetry-001";
    let model = "Qwen/Qwen2.5-7B-Instruct";

    let server = MockServer::start().await;
    let graph_release_pattern =
        format!(r"^{}{}[^/]+$", versioned_path(api_paths::APXM_GRAPHS), "/");

    Mock::given(method("POST"))
        .and(path(versioned_path(api_paths::APXM_GRAPHS_REGISTER)))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(graph_register_body(graph_id, exec_id, 5)),
        )
        .expect(1)
        .mount(&server)
        .await;

    // GET /v1/apxm/graphs/{id} returns pin telemetry
    let status_path = format!(
        "{}{}/{}",
        api_paths::VERSION_PREFIX,
        api_paths::APXM_GRAPHS,
        graph_id,
    );
    let status_fixture = GraphStatusResponse {
        object: apxm_llm::OBJECT_GRAPH_STATUS.to_owned(),
        graph_id: graph_id.to_owned(),
        registered: true,
        pinned_blocks: 7,
        pinned_handles: 3,
        critical_path_length: Some(4),
        node_count: Some(5),
    };
    Mock::given(method("GET"))
        .and(path(&status_path))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::to_value(&status_fixture).unwrap()),
        )
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("DELETE"))
        .and(path_regex(graph_release_pattern))
        .respond_with(ResponseTemplate::new(200).set_body_json(graph_release_body(graph_id)))
        .expect(1)
        .mount(&server)
        .await;

    let backend = make_backend(&server.uri(), model).await;

    let metadata = GraphMetadata::new(graph_id, exec_id);
    backend
        .register_graph(metadata)
        .await
        .expect("register_graph");

    // Collect status before release
    let status = LLMBackend::get_graph_status(&backend, graph_id)
        .await
        .expect("get_graph_status should succeed");

    assert!(
        status.is_some(),
        "expected Some(snapshot) from get_graph_status"
    );
    let status_json = status.unwrap().to_metrics_json();
    use apxm_core::constants::session::metrics_keys::graph_status_keys as gsk;
    assert_eq!(status_json[gsk::PINNED_BLOCKS], 7);
    assert_eq!(status_json[gsk::PINNED_HANDLES], 3);
    assert_eq!(status_json[gsk::CRITICAL_PATH_LENGTH], 4);
    assert_eq!(status_json[gsk::NODE_COUNT], 5);
    assert_eq!(status_json[gsk::GRAPH_ID], graph_id);

    // Release after status collection
    GraphAwareVllmBackend::release_graph(&backend, graph_id)
        .await
        .expect("release_graph");

    // Verify request ordering: POST register, GET status, DELETE release
    let received = server.received_requests().await.unwrap_or_default();
    let methods: Vec<&str> = received.iter().map(|r| r.method.as_str()).collect();
    let post_idx = methods.iter().position(|&m| m == "POST").expect("POST");
    let get_idx = methods.iter().position(|&m| m == "GET").expect("GET");
    let delete_idx = methods.iter().position(|&m| m == "DELETE").expect("DELETE");
    assert!(
        post_idx < get_idx,
        "register (POST) must precede status (GET)"
    );
    assert!(
        get_idx < delete_idx,
        "status (GET) must precede release (DELETE)"
    );

    drop(server);
}
