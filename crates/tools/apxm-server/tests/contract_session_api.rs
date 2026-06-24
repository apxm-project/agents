//! Session API contract tests.

mod contract;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use apxm_backends::llm::backends::mock::MockLLMBackend;
use apxm_runtime::executor::session_ledger::{SessionLedger, seed};
use apxm_server::test_support;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn get_json(app: &Router, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn post_json(
    app: &Router,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

use futures::StreamExt;

async fn post_execute_stream(app: &Router, session_id: &str) -> StatusCode {
    let req = Request::builder()
        .method("POST")
        .uri("/v1/execute/stream")
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "air": test_support::single_ask_air(),
                "session_id": session_id,
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let _ = resp.into_body().collect().await.unwrap().to_bytes();
    status
}

async fn get_sse_prefix(app: &Router, path: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let mut stream = resp.into_body().into_data_stream();
    let chunk = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("timed out waiting for first SSE chunk")
        .expect("stream ended before first chunk")
        .expect("stream chunk");
    (status, String::from_utf8_lossy(&chunk).into_owned())
}

fn seed_session_ledger(session_id: &str, turn_cap: Option<usize>, grants: &[&str]) {
    let mut grant_set = HashSet::new();
    grant_set.extend(grants.iter().map(|s| s.to_string()));
    seed(
        session_id,
        SessionLedger::new(turn_cap, HashMap::new(), grant_set),
    );
}

#[tokio::test]
async fn unknown_session_status_returns_typed_not_found() {
    let app = test_support::contract_app_with_mock(MockLLMBackend::static_response("ok")).await;
    let (status, json) = get_json(&app, "/v1/sessions/unknown-session-xyz/status").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    contract::assert_typed_error_envelope(&json);
    contract::assert_fault_code(&json, "not_found");
}

#[tokio::test]
async fn session_status_matches_openapi_shape() {
    let session_id = "contract-status-shape";
    seed_session_ledger(session_id, Some(5), &["read_file"]);
    let app = test_support::contract_app_with_mock(MockLLMBackend::static_response("ok")).await;
    let (status, json) = get_json(&app, &format!("/v1/sessions/{session_id}/status")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["session_id"], session_id);
    assert!(json["turn_count"].is_number());
    assert!(json["ledger"].is_object());
    assert_eq!(json["ledger"]["turn_cap"], 5);
    assert!(
        json["ledger"]["grants"]
            .as_array()
            .expect("grants array")
            .contains(&serde_json::json!("read_file"))
    );
}

#[tokio::test]
async fn cancel_unknown_inflight_session_returns_conflict() {
    let session_id = "contract-cancel-conflict";
    seed_session_ledger(session_id, None, &[]);
    let app = test_support::contract_app_with_mock(MockLLMBackend::static_response("ok")).await;
    let (status, json) = post_json(
        &app,
        &format!("/v1/sessions/{session_id}/cancel"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    contract::assert_typed_error_envelope(&json);
    contract::assert_fault_code(&json, "conflict");
}

#[tokio::test]
async fn update_grants_add_and_remove_round_trip_in_status() {
    let session_id = "contract-grants-roundtrip";
    seed_session_ledger(session_id, None, &["alpha"]);
    let app = test_support::contract_app_with_mock(MockLLMBackend::static_response("ok")).await;

    let (status, _) = post_json(
        &app,
        &format!("/v1/sessions/{session_id}/grants"),
        serde_json::json!({ "add": ["beta"], "remove": ["alpha"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, json) = get_json(&app, &format!("/v1/sessions/{session_id}/status")).await;
    let grants = json["ledger"]["grants"].as_array().expect("grants");
    assert!(grants.contains(&serde_json::json!("beta")));
    assert!(!grants.iter().any(|g| g == "alpha"));
}

#[tokio::test]
async fn compact_session_returns_ok_for_known_session() {
    let session_id = "contract-compact-ok";
    seed_session_ledger(session_id, None, &[]);
    let app = test_support::contract_app_with_mock(MockLLMBackend::static_response("ok")).await;
    let (status, json) = post_json(
        &app,
        &format!("/v1/sessions/{session_id}/compact"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["ok"], true);
    assert_eq!(json["session_id"], session_id);
}

#[tokio::test]
async fn list_session_events_after_execute_stream() {
    let app = test_support::contract_app_with_mock(MockLLMBackend::static_response("hello")).await;
    let session_id = "contract-events-session";
    let status = post_execute_stream(&app, session_id).await;
    assert_eq!(status, StatusCode::OK);

    let (status, json) = get_json(&app, &format!("/v1/sessions/{session_id}/events?since=0")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["session_id"], session_id);
    assert!(!json["events"].as_array().expect("events").is_empty());
    assert!(json["next_seq"].is_number());
    assert!(json["done"].is_boolean());
}

#[tokio::test]
async fn stream_session_events_returns_sse() {
    let app = test_support::contract_app_with_mock(MockLLMBackend::static_response("hello")).await;
    let session_id = "contract-stream-session";
    let status = post_execute_stream(&app, session_id).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = get_sse_prefix(
        &app,
        &format!("/v1/sessions/{session_id}/events/stream?since=0"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("data:"),
        "expected SSE data frames, got: {body}"
    );
}

#[tokio::test]
async fn sc003_turn_cap_denies_conversation_message_without_client_counting() {
    let session_id = "contract-turn-cap";
    seed(
        session_id,
        SessionLedger::new(Some(1), HashMap::new(), HashSet::new()),
    );
    let ledger = apxm_runtime::executor::session_ledger::get(session_id).expect("ledger");
    assert_eq!(ledger.charge_turn().unwrap(), 1);

    let app = test_support::contract_app_with_mock(MockLLMBackend::static_response("ok")).await;
    let (status, json) = post_json(
        &app,
        &format!("/v1/conversations/{session_id}/message"),
        serde_json::json!({ "message": "turn two" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    contract::assert_typed_error_envelope(&json);
    contract::assert_fault_class(&json, "program_fault");
    contract::assert_fault_code(&json, "turn_cap_exceeded");
}

#[tokio::test]
async fn invalid_session_id_returns_bad_request() {
    let app = test_support::contract_app_with_mock(MockLLMBackend::static_response("ok")).await;
    let (status, json) = get_json(&app, "/v1/sessions/bad%20id/status").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    contract::assert_typed_error_envelope(&json);
    contract::assert_fault_code(&json, "bad_request");
}
