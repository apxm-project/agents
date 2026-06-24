//! SSE backpressure integration tests.

use std::time::Duration;

use apxm_backends::llm::backends::mock::{MockLLMBackend, MockResponse};
use apxm_driver::ServerConfig;
use apxm_server::test_support;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures::StreamExt;
use http_body_util::BodyExt;
use tower::ServiceExt;

const EXECUTE_STREAM: &str = "/v1/execute/stream";

#[derive(Debug, Clone)]
struct SseFrame {
    id: Option<u64>,
    event: Option<String>,
    data: String,
}

fn parse_sse_frames(input: &str) -> Vec<SseFrame> {
    let mut frames = Vec::new();
    for block in input.split("\n\n") {
        if block.trim().is_empty() {
            continue;
        }
        let mut id = None;
        let mut event = None;
        let mut data_lines = Vec::new();
        for line in block.lines() {
            if let Some(value) = line.strip_prefix("id:") {
                id = value.trim().parse().ok();
            } else if let Some(value) = line.strip_prefix("event:") {
                event = Some(value.trim().to_string());
            } else if let Some(value) = line.strip_prefix("data:") {
                data_lines.push(value.trim_start());
            }
        }
        if !data_lines.is_empty() {
            frames.push(SseFrame {
                id,
                event,
                data: data_lines.join("\n"),
            });
        }
    }
    frames
}

async fn post_execute_stream(app: Router, air: &str, session_id: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri(EXECUTE_STREAM)
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "air": air,
                "session_id": session_id,
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn post_execute_stream_slow(
    app: Router,
    air: &str,
    session_id: &str,
    pause_every_bytes: usize,
    pause_ms: u64,
) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri(EXECUTE_STREAM)
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "air": air,
                "session_id": session_id,
            })
            .to_string(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let mut stream = resp.into_body().into_data_stream();
    let mut body = String::new();
    let mut bytes_read = 0usize;
    let mut paused_once = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("stream chunk");
        bytes_read += chunk.len();
        body.push_str(&String::from_utf8_lossy(&chunk));
        if paused_once && bytes_read >= pause_every_bytes {
            bytes_read = 0;
            tokio::time::sleep(Duration::from_millis(pause_ms)).await;
        } else if !paused_once {
            paused_once = true;
        }
    }
    (status, body)
}

async fn get_run_events_bulk_since(
    app: Router,
    execution_id: &str,
    since: u64,
) -> (StatusCode, serde_json::Value) {
    let path = format!("/v1/runs/{execution_id}/events?since={since}");
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn event_kind(json: &serde_json::Value) -> Option<&str> {
    json.get("kind")
        .and_then(|v| v.as_str())
        .or_else(|| json.pointer("/payload/kind").and_then(|v| v.as_str()))
}

fn execution_id_from_sse(frames: &[SseFrame]) -> Option<String> {
    for frame in frames {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&frame.data) else {
            continue;
        };
        if event_kind(&json) == Some("execution_started") {
            return json
                .pointer("/payload/execution_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
    }
    None
}

fn event_seqs(frames: &[SseFrame]) -> Vec<u64> {
    frames.iter().filter_map(|frame| frame.id).collect()
}

fn is_lag_frame(frame: &SseFrame) -> bool {
    if frame.event.as_deref() == Some("error")
        || frame.data.contains("stream_lag")
        || frame.data.contains("lagged")
        || frame.data.contains("execute_stream_lag")
    {
        return true;
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&frame.data)
        && event_kind(&json) == Some("warning")
    {
        return json.pointer("/payload/code").and_then(|v| v.as_str())
            == Some("execute_stream_lag");
    }
    false
}

fn is_data_event_frame(frame: &SseFrame) -> bool {
    !frame.data.is_empty() && !is_lag_frame(frame)
}

fn bulk_event_seqs(events: &serde_json::Value) -> Vec<u64> {
    events
        .get("events")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|event| event.pointer("/meta/seq").and_then(|v| v.as_u64()))
                .collect()
        })
        .unwrap_or_default()
}

fn token_events_in_bulk(events: &serde_json::Value) -> usize {
    events
        .get("events")
        .and_then(|v| v.as_array())
        .map_or(0, |items| {
            items
                .iter()
                .filter(|event| event_kind(event) == Some("token"))
                .count()
        })
}

fn token_event_count(frames: &[SseFrame]) -> usize {
    frames
        .iter()
        .filter(|frame| {
            serde_json::from_str::<serde_json::Value>(&frame.data)
                .ok()
                .and_then(|json| event_kind(&json).map(str::to_string))
                == Some("token".to_string())
        })
        .count()
}

fn high_volume_mock(output_tokens: usize, tokens_per_second: u64) -> MockLLMBackend {
    MockLLMBackend::new()
        .default(MockResponse::new("tok ").with_tokens(10, output_tokens))
        .with_tokens_per_second(tokens_per_second)
}

/// Under deliberate consumer backpressure, no silent drops and overflow
/// surfaces an explicit lag/error frame.
#[tokio::test]
async fn sse_backpressure_sc001_no_silent_drops_and_lag_signal() {
    let mut config = ServerConfig::default();
    config.run_events = test_support::backpressure_run_events_config();
    config.execution_stream.channel_capacity = 4;
    let harness =
        test_support::test_harness_with_config_and_mock(config, high_volume_mock(100, 50_000))
            .await;
    let app = harness.router.clone();
    let bus_execution_id = harness.first_execution_id();

    let session_id = "sc001-backpressure";
    let (status, body) = post_execute_stream_slow(
        app.clone(),
        &test_support::single_ask_air(),
        session_id,
        32,
        40,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "execute stream failed: {body}");

    // Allow the background run task to finish recording on the observer bus.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let frames = parse_sse_frames(&body);
    let lag_frames: Vec<_> = frames.iter().filter(|f| is_lag_frame(f)).collect();
    assert!(
        !lag_frames.is_empty(),
        "slow consumer must receive explicit lag/overflow signal, body: {}",
        &body[..body.len().min(400)]
    );
    for lag in &lag_frames {
        assert!(
            lag.data.contains("lagged")
                || lag.data.contains("stream_lag")
                || lag.data.contains("execute_stream_lag"),
            "lag frame must be client-visible: {lag:?}"
        );
    }

    let data_frames: Vec<SseFrame> = frames
        .iter()
        .filter(|f| is_data_event_frame(f))
        .cloned()
        .collect();
    if !data_frames.is_empty() {
        let seqs = event_seqs(&data_frames);
        assert!(
            seqs.windows(2).all(|pair| pair[0] < pair[1]),
            "live data frames must be strictly ordered by seq: {seqs:?}"
        );
        assert!(
            data_frames.iter().all(|frame| frame.id.is_some()),
            "data frames must carry Last-Event-ID seq"
        );
    }

    let execution_id = execution_id_from_sse(&frames)
        .or(bus_execution_id)
        .expect("execution id must be discoverable from SSE or observer bus");

    let (status, bulk) = get_run_events_bulk_since(app, &execution_id, 0).await;
    assert_eq!(status, StatusCode::OK, "bulk events failed: {bulk}");
    let bus_seqs = bulk_event_seqs(&bulk);
    assert!(
        bus_seqs.len() >= 10,
        "observer bus must retain events for replay: {bus_seqs:?}"
    );
    assert!(
        bus_seqs.windows(2).all(|w| w[1] == w[0] + 1),
        "observer bus seq must be contiguous (0 silent drops): {bus_seqs:?}"
    );
}

/// Reconnect resume delivers post-ack events in order across simulated disconnects.
#[tokio::test]
async fn sse_backpressure_sc002_reconnect_resume_preserves_order() {
    let harness = test_support::test_harness_with_config_and_mock(
        ServerConfig::default(),
        high_volume_mock(40, 50_000),
    )
    .await;
    let app = harness.router.clone();
    let bus_execution_id = harness.first_execution_id();

    let session_id = "sc002-resume";
    let (status, body) =
        post_execute_stream(app.clone(), &test_support::single_ask_air(), session_id).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "baseline execute stream failed: {body}"
    );

    let frames = parse_sse_frames(&body);
    let execution_id = execution_id_from_sse(&frames)
        .or(bus_execution_id)
        .expect("execution_started must expose execution_id");

    let (status, bulk) = get_run_events_bulk_since(app.clone(), &execution_id, 0).await;
    assert_eq!(status, StatusCode::OK, "bulk events failed: {bulk}");
    let all_seqs = bulk_event_seqs(&bulk);
    assert!(
        all_seqs.len() >= 10,
        "need enough retained events to resume: {all_seqs:?}, stream prefix: {}",
        &body[..body.len().min(200)]
    );
    assert!(
        all_seqs.windows(2).all(|w| w[1] == w[0] + 1),
        "bulk seq must be contiguous: {all_seqs:?}"
    );

    for disconnect in 0..10 {
        let ack_idx = (disconnect + 1) * (all_seqs.len() / 11);
        let ack_seq = all_seqs[ack_idx.min(all_seqs.len().saturating_sub(1))];
        let expected_tail: Vec<u64> = all_seqs
            .iter()
            .copied()
            .filter(|seq| *seq > ack_seq)
            .collect();

        let (status, resumed) =
            get_run_events_bulk_since(app.clone(), &execution_id, ack_seq + 1).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "resume bulk failed on disconnect {disconnect}: {resumed}"
        );
        let resumed_seqs = bulk_event_seqs(&resumed);

        assert_eq!(
            resumed_seqs, expected_tail,
            "disconnect {disconnect}: post-ack resume must deliver full ordered tail (ack={ack_seq})"
        );
    }
}
