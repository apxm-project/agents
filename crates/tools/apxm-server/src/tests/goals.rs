use super::*;
use crate::routes;
use apxm_core::events::payload::{ErrorPayload, ExecutionStartedPayload};
use apxm_core::events::{ApxmEvent, EventSource};

#[tokio::test]
async fn goal_routes_expose_status_events_list_and_cancel() {
    let state = test_state().await;
    let goal_id = format!("goal-route-{}", uuid::Uuid::new_v4());
    state
        .goal_runs
        .insert_running(goal_id.clone(), "ship the goal routes".to_string(), 3);
    state.goal_runs.set_current_pass(
        &goal_id,
        0,
        "exec-1".to_string(),
        "session-1".to_string(),
        Some("/tmp/session-1".to_string()),
        "/tmp/workflow.apxmw".to_string(),
        "/tmp/goal-bundle".to_string(),
        serde_json::json!({ "plan_json": "/tmp/goal-bundle/plan.json" }),
        serde_json::json!({
            "task": "ship the goal routes",
            "workers": []
        }),
        serde_json::json!({ "mode": "explicit", "generated": false }),
        None,
        serde_json::json!({ "status_tool": "goal_status" }),
    );
    state.run_event_bus.record(
        &goal_id,
        ApxmEvent::root(
            ExecutionStartedPayload {
                execution_id: goal_id.clone(),
            },
            EventSource::Server,
            &goal_id,
        ),
    );
    let app = build_app(state);

    let (status, body) = get_json(app.clone(), &routes::goal_detail_path(&goal_id)).await;
    assert_eq!(status, StatusCode::OK, "goal detail failed: {body}");
    assert_eq!(body["goal_id"], goal_id);
    assert_eq!(body["task"]["description"], "ship the goal routes");
    assert_eq!(body["task"]["plan"]["task"], "ship the goal routes");
    assert_eq!(body["task"]["planning"]["mode"], "explicit");
    assert_eq!(body["status"], "running");
    assert_eq!(body["latest_pass"]["execution_id"], "exec-1");
    assert_eq!(body["latest_pass"]["status"], "unknown");
    assert_eq!(body["totals"]["events"], 1);

    let list_path = format!("{}?status=running&limit=1", routes::GOALS);
    let (status, body) = get_json(app.clone(), &list_path).await;
    assert_eq!(status, StatusCode::OK, "goal list failed: {body}");
    assert_eq!(body["object"], "list");
    assert_eq!(body["data"][0]["goal_id"], goal_id);
    assert_eq!(
        body["data"][0]["task"]["description"],
        "ship the goal routes"
    );

    let events_path = format!("{}?since=0&limit=1", routes::goal_events_path(&goal_id));
    let (status, body) = get_json(app.clone(), &events_path).await;
    assert_eq!(status, StatusCode::OK, "goal events failed: {body}");
    assert_eq!(body["goal_id"], goal_id);
    assert_eq!(body["events"].as_array().expect("events").len(), 1);
    assert_eq!(body["next_seq"], 1);

    let (status, body) = post_json(
        app.clone(),
        &routes::goal_cancel_path(&goal_id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "goal cancel failed: {body}");
    assert_eq!(body["goal_id"], goal_id);
    assert_eq!(body["cancelled"], true);

    let (status, body) = get_json(app, &routes::goal_detail_path(&goal_id)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "goal detail after cancel failed: {body}"
    );
    assert_eq!(body["status"], "cancelled");
    assert_eq!(body["cancel_requested"], true);
}

#[tokio::test]
async fn goal_routes_return_404_for_unknown_goal() {
    let app = build_app(test_state().await);
    let (status, body) = get_json(app, &routes::goal_detail_path("missing-goal")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unexpected response: {body}");
}

#[tokio::test]
async fn goal_routes_start_goal_with_same_plan_contract_as_mcp() {
    let app = build_app(test_state().await);
    let session_id = format!("goal-rest-plan-{}", uuid::Uuid::new_v4());

    let (status, body) = post_json(
        app,
        routes::GOALS,
        serde_json::json!({
            "task": "investigate frontend plan wiring",
            "session_id": session_id,
            "dry_run": true,
            "planning": { "mode": "auto", "max_workers": 4 }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "goal REST start failed: {body}");
    assert_eq!(
        body["status"],
        apxm_core::types::OrchestrationStartStatus::Planned.as_str(),
        "dry-run start should only materialize the plan: {body}"
    );
    assert!(
        body["goal_id"].as_str().is_some(),
        "goal_id missing: {body}"
    );
    assert_eq!(body["plan"]["task"], "investigate frontend plan wiring");
    assert_eq!(body["planning"]["generated"], true);
    assert_eq!(body["control"]["status_tool"], "goal_status");

    let bundle_dir = std::path::PathBuf::from(body["bundle_dir"].as_str().expect("bundle_dir"));
    let _ = std::fs::remove_dir_all(bundle_dir);
}

#[tokio::test]
async fn goal_events_stream_replays_and_tails_live_events() {
    let state = test_state().await;
    let goal_id = format!("goal-stream-{}", uuid::Uuid::new_v4());
    state
        .goal_runs
        .insert_running(goal_id.clone(), "stream goal events".to_string(), 1);
    state.run_event_bus.record(
        &goal_id,
        ApxmEvent::root(
            ExecutionStartedPayload {
                execution_id: goal_id.clone(),
            },
            EventSource::Server,
            &goal_id,
        ),
    );
    let app = build_app(state.clone());

    let req = Request::builder()
        .method("GET")
        .uri(&routes::goal_events_stream_path(&goal_id))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    use http_body_util::BodyStream;
    let mut body_stream = BodyStream::new(resp.into_body());
    let mut buf = Vec::new();
    read_sse_bytes_until(&mut body_stream, &mut buf, "event: execution_started").await;

    state.run_event_bus.record(
        &goal_id,
        ApxmEvent::root(
            ErrorPayload {
                message: "wake the caller".to_string(),
                status: None,
                recoverable: false,
            },
            EventSource::Server,
            &goal_id,
        ),
    );
    read_sse_bytes_until(&mut body_stream, &mut buf, "event: error").await;

    let text = String::from_utf8_lossy(&buf);
    assert!(
        text.contains("id: 0\n") && text.contains("event: execution_started"),
        "stream should replay retained goal events: {text}"
    );
    assert!(
        text.contains("id: 1\n") && text.contains("event: error"),
        "stream should tail live goal events: {text}"
    );
}

async fn read_sse_bytes_until(
    body_stream: &mut http_body_util::BodyStream<Body>,
    buf: &mut Vec<u8>,
    needle: &str,
) {
    use futures::StreamExt as _;

    for _ in 0..20 {
        let next =
            tokio::time::timeout(std::time::Duration::from_millis(500), body_stream.next()).await;
        if let Ok(Some(Ok(frame))) = next
            && let Some(data) = frame.data_ref()
        {
            buf.extend_from_slice(data);
            if String::from_utf8_lossy(buf).contains(needle) {
                return;
            }
        }
    }
    panic!(
        "SSE stream did not contain {needle}: {}",
        String::from_utf8_lossy(buf)
    );
}
