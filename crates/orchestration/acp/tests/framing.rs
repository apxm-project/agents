//! JSON-RPC 2.0 framing tests for the stdio transport `apxm-acp` uses to
//! talk to spawned agents: request/response encode+decode, error decode,
//! notification dispatch, and reverse-request encode/decode/response.
//!
//! These drive a real child process (a small Python fixture) over real
//! pipes rather than mocking the transport, so they exercise the exact
//! NDJson framing `StdioTransport` writes and reads in production.

mod support;

use std::process::Stdio;

use apxm_acp::AcpError;
use apxm_acp::protocol::StdioTransport;
use apxm_acp::reverse::ReverseHandler;
use async_trait::async_trait;
use tokio::process::Command;

fn spawn_framing_agent(dir: &std::path::Path) -> (tokio::process::Child, StdioTransport) {
    let script = support::write_fixture(dir, "framing_agent.py", support::FRAMING_AGENT);
    let mut child = Command::new("python3")
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn framing fixture");
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let transport = StdioTransport::new(stdin, stdout);
    (child, transport)
}

/// No-op reverse handler for tests that don't expect a reverse call.
struct NoOpHandler;

#[async_trait]
impl ReverseHandler for NoOpHandler {
    async fn handle(&self, method: &str, _params: serde_json::Value) -> Result<serde_json::Value, AcpError> {
        panic!("unexpected reverse request: {method}");
    }
}

#[tokio::test]
async fn request_response_round_trip_preserves_ids_and_result() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut transport) = spawn_framing_agent(dir.path());

    let id = transport
        .send_request("ping", Some(serde_json::json!({"a": 1})))
        .await
        .unwrap();
    assert_eq!(id, 1, "the first request on a fresh transport gets id 1");

    let result = transport
        .read_response(id, &NoOpHandler)
        .await
        .expect("ping should succeed");
    assert_eq!(result, serde_json::json!({"echo": {"a": 1}}));

    // IDs increment monotonically across the life of the transport.
    let id2 = transport.send_request("ping", None).await.unwrap();
    assert_eq!(id2, 2);
    let _ = transport.read_response(id2, &NoOpHandler).await.unwrap();

    drop(transport);
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[tokio::test]
async fn error_responses_decode_into_agent_error() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut transport) = spawn_framing_agent(dir.path());

    let id = transport.send_request("boom", None).await.unwrap();
    let err = transport
        .read_response(id, &NoOpHandler)
        .await
        .expect_err("boom should produce a JSON-RPC error");

    match err {
        AcpError::AgentError { code, message } => {
            assert_eq!(code, 123);
            assert_eq!(message, "nope");
        }
        other => panic!("expected AgentError, got {other:?}"),
    }

    drop(transport);
    let _ = child.kill().await;
    let _ = child.wait().await;
}

/// Captures notifications delivered while waiting for a response.
struct CapturingHandler {
    seen: std::sync::Mutex<Vec<(String, Option<serde_json::Value>)>>,
}

#[async_trait]
impl ReverseHandler for CapturingHandler {
    async fn handle(&self, method: &str, _params: serde_json::Value) -> Result<serde_json::Value, AcpError> {
        panic!("unexpected reverse request: {method}");
    }

    async fn on_notification(&self, method: &str, params: Option<&serde_json::Value>) {
        self.seen
            .lock()
            .unwrap()
            .push((method.to_string(), params.cloned()));
    }
}

#[tokio::test]
async fn notifications_are_dispatched_before_the_final_response() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut transport) = spawn_framing_agent(dir.path());

    let handler = CapturingHandler {
        seen: std::sync::Mutex::new(Vec::new()),
    };

    let id = transport.send_request("fire_notification", None).await.unwrap();
    let result = transport.read_response(id, &handler).await.unwrap();
    assert_eq!(result, serde_json::json!({"ok": true}));

    let seen = handler.seen.into_inner().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "note");
    assert_eq!(seen[0].1, Some(serde_json::json!({"x": 1})));

    drop(transport);
    let _ = child.kill().await;
    let _ = child.wait().await;
}

/// Answers a reverse request with a fixed result so the test can assert the
/// fixture observed exactly what we sent back.
struct ReversePongHandler;

#[async_trait]
impl ReverseHandler for ReversePongHandler {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, AcpError> {
        assert_eq!(method, "myreverse");
        assert_eq!(params, serde_json::json!({"foo": "bar"}));
        Ok(serde_json::json!({"pong": true}))
    }
}

#[tokio::test]
async fn reverse_requests_are_encoded_dispatched_and_answered() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut transport) = spawn_framing_agent(dir.path());

    let id = transport.send_request("ask_reverse", None).await.unwrap();
    let result = transport
        .read_response(id, &ReversePongHandler)
        .await
        .expect("the fixture should relay our reverse response back");

    // The fixture echoes back the id/result it received for the reverse
    // request it issued, proving our response was framed correctly on the
    // wire (matching id, JSON-RPC 2.0 shape) and decoded correctly on its end.
    assert_eq!(
        result,
        serde_json::json!({"reverse_id": 7, "reverse_result": {"pong": true}})
    );

    drop(transport);
    let _ = child.kill().await;
    let _ = child.wait().await;
}
