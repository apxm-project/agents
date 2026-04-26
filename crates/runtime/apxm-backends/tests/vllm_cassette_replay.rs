//! Cassette-replay test: verifies wire-format compatibility between APXM
//! and a vLLM-compatible OpenAI endpoint using a recorded request/response
//! fixture.
//!
//! The fixture at `tests/fixtures/vllm_qwen35_happy.json` contains a single
//! happy-path exchange: one `POST /v1/chat/completions` request with
//! `vllm_xargs.apxm` scheduling hints and the corresponding OpenAI-format
//! response from a Qwen/Qwen3.5-4B model.
//!
//! ## Re-recording
//!
//! When the wire format changes (e.g. new fields in `ApxmGraphHints` or a
//! different OpenAI response schema), re-record the fixture:
//!
//! ```bash
//! # TODO: implement automatic recording mode.
//! # For now, manually curl the vLLM endpoint and paste request + response
//! # into tests/fixtures/vllm_qwen35_happy.json.
//! #
//! # APXM_RECORD_FIXTURE=1 cargo test -p apxm-backends --test vllm_cassette_replay
//! ```

use apxm_backends::llm::backends::vllm::{ApxmGraphHints, GraphAwareVllmBackend};
use apxm_backends::llm::backends::{LLMBackend, LLMRequest};
use apxm_core::constants::llm::{apxm as apxm_llm, openai as openai_keys};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Load the cassette fixture from disk.
fn load_fixture() -> Value {
    let bytes = include_bytes!("fixtures/vllm_qwen35_happy.json");
    serde_json::from_slice(bytes).expect("parse cassette fixture JSON")
}

#[tokio::test]
async fn cassette_replay_qwen35_happy_path() {
    let fixture = load_fixture();
    let request_fixture = &fixture["request"];
    let response_fixture = &fixture["response"];

    // Extract expected values from fixture.
    let model = request_fixture["model"]
        .as_str()
        .expect("fixture request.model");
    let expected_content = response_fixture["choices"][0]["message"]["content"]
        .as_str()
        .expect("fixture response assistant content");

    // Stand up a wiremock server that replays the recorded response.
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_fixture.clone()))
        .expect(1)
        .mount(&server)
        .await;

    // Build the backend pointed at the mock server.
    // Convention: base_url includes the /v1 prefix.
    let base_url = format!("{}/v1", server.uri());
    let backend = GraphAwareVllmBackend::new(
        "test-key",
        Some(json!({
            "base_url": base_url,
            "model": model,
        })),
    )
    .await
    .expect("construct GraphAwareVllmBackend");

    // Build the same request shape as the fixture: prompt + hints.
    let prompt = request_fixture["messages"][0]["content"]
        .as_str()
        .expect("fixture request prompt");
    let apxm_hints =
        &request_fixture[openai_keys::EXTRA_BODY][apxm_llm::VLLM_XARGS][apxm_llm::HINTS_FIELD];

    let hints = ApxmGraphHints::critical_path(
        apxm_hints[apxm_llm::GRAPH_ID].as_str().unwrap(),
        apxm_hints[apxm_llm::EXECUTION_ID].as_str().unwrap(),
        apxm_hints[apxm_llm::NODE_ID].as_u64().unwrap() as u32,
        apxm_hints[apxm_llm::NODE_NAME].as_str().unwrap(),
        apxm_hints[apxm_llm::DOWNSTREAM_NODES]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u32)
            .collect(),
        apxm_hints[apxm_llm::PIN_POLICY][apxm_llm::PIN_POLICY_TTL_MS]
            .as_u64()
            .unwrap() as u32,
    );

    let request = LLMRequest::new(prompt)
        .with_temperature(0.1)
        .with_apxm_hints(hints);

    // Execute the request against the mock.
    let response = LLMBackend::generate(&backend, request)
        .await
        .expect("generate from cassette replay");

    // Assert the parsed response matches the fixture.
    assert_eq!(
        response.content, expected_content,
        "response content should match the cassette fixture"
    );
    assert_eq!(
        response.model, model,
        "response model should match the cassette fixture"
    );

    // Verify the mock received exactly one request.
    let received = server.received_requests().await.unwrap_or_default();
    let chat_reqs: Vec<_> = received
        .iter()
        .filter(|r| r.url.path().ends_with("/chat/completions"))
        .collect();
    assert_eq!(
        chat_reqs.len(),
        1,
        "expected exactly one chat/completions request"
    );

    // Verify the outgoing request carried vllm_xargs.apxm with the right graph_id.
    let sent_body: Value =
        serde_json::from_slice(&chat_reqs[0].body).expect("parse sent request body");
    let sent_apxm = sent_body
        .get(apxm_llm::VLLM_XARGS)
        .and_then(|xargs| xargs.get(apxm_llm::HINTS_FIELD));
    assert!(
        sent_apxm.is_some(),
        "outgoing request must carry vllm_xargs.apxm"
    );
    let sent_apxm = sent_apxm.unwrap();
    assert_eq!(
        sent_apxm[apxm_llm::GRAPH_ID].as_str(),
        Some("cassette-graph-001"),
        "sent graph_id must match fixture"
    );
    assert_eq!(
        sent_apxm[apxm_llm::NODE_ID].as_u64(),
        Some(1),
        "sent node_id must match fixture"
    );

    // Explicit drop so wiremock expect(1) fires inside this test fn.
    drop(server);
}
