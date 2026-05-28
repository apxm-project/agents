use apxm_backends::llm::backends::{
    AnthropicBackend, LLMBackend, LLMRequest, OllamaBackend, OpenAIBackend, StreamChunk,
};
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn openai_truncated_stream_errors_and_requests_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n"),
        )
        .mount(&server)
        .await;

    let backend = OpenAIBackend::new("test-key", Some(json!({ "base_url": server.uri() })))
        .await
        .expect("backend");
    let mut stream = backend.generate_stream(LLMRequest::new("hello"));

    match stream.next().await.expect("token").expect("token chunk") {
        StreamChunk::Token(token) => assert_eq!(token, "hi"),
        other => panic!("expected token, got {other:?}"),
    }
    let err = stream
        .next()
        .await
        .expect("terminal error")
        .expect_err("truncated stream must error");
    assert!(err.to_string().contains("[DONE]"));
    drop(stream);

    let received = server.received_requests().await.unwrap_or_default();
    let body: Value = serde_json::from_slice(&received[0].body).expect("request body");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
}

#[tokio::test]
async fn openai_stream_options_from_extra_body_are_preserved() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("data: [DONE]\n\n"),
        )
        .mount(&server)
        .await;

    let backend = OpenAIBackend::new("test-key", Some(json!({ "base_url": server.uri() })))
        .await
        .expect("backend");
    let request = LLMRequest::new("hello").with_extra_body(json!({
        "stream_options": { "include_usage": false }
    }));
    let mut stream = backend.generate_stream(request);

    match stream.next().await.expect("done").expect("done chunk") {
        StreamChunk::Done(_) => {}
        other => panic!("expected done, got {other:?}"),
    }
    drop(stream);

    let received = server.received_requests().await.unwrap_or_default();
    let body: Value = serde_json::from_slice(&received[0].body).expect("request body");
    assert_eq!(body["stream_options"]["include_usage"], false);
}

#[tokio::test]
async fn anthropic_truncated_stream_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    "event: content_block_delta\n\
                     data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n",
                ),
        )
        .mount(&server)
        .await;

    let backend = AnthropicBackend::new("test-key", Some(json!({ "base_url": server.uri() })))
        .await
        .expect("backend");
    let mut stream = backend.generate_stream(LLMRequest::new("hello"));

    match stream.next().await.expect("token").expect("token chunk") {
        StreamChunk::Token(token) => assert_eq!(token, "hi"),
        other => panic!("expected token, got {other:?}"),
    }
    let err = stream
        .next()
        .await
        .expect("terminal error")
        .expect_err("truncated stream must error");
    assert!(err.to_string().contains("message_stop"));
}

#[tokio::test]
async fn ollama_truncated_stream_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .set_body_string("{\"message\":{\"content\":\"hi\"},\"done\":false}\n"),
        )
        .mount(&server)
        .await;

    let backend = OllamaBackend::new("", Some(json!({ "base_url": server.uri() })))
        .await
        .expect("backend");
    let mut stream = backend.generate_stream(LLMRequest::new("hello"));

    match stream.next().await.expect("token").expect("token chunk") {
        StreamChunk::Token(token) => assert_eq!(token, "hi"),
        other => panic!("expected token, got {other:?}"),
    }
    let err = stream
        .next()
        .await
        .expect("terminal error")
        .expect_err("truncated stream must error");
    assert!(err.to_string().contains("done=true"));
}
