//! tests for the outbound run-lifecycle webhook.
//!
//! These verify the fire-and-forget contract end-to-end:
//!   - The dispatcher POSTs to the configured URL.
//!   - Each delivered body matches the on-wire SSE envelope.
//!   - A dead receiver does NOT block / break execution.
//!
//! Implementation note: the tests spawn a bare-bones HTTP server on
//! localhost and dispatch directly against it.  We never depend on
//! external network access.

use super::*;
use crate::webhook::WebhookDispatcher;
use apxm_core::events::payload::{AgentSpawnedPayload, OperationStartPayload};
use apxm_core::events::{ApxmEvent, EventSource};
use axum::Json as AxumJson;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::net::TcpListener;

struct WebhookSink {
    received: Arc<Mutex<Vec<serde_json::Value>>>,
    addr: std::net::SocketAddr,
    _shutdown: tokio::task::JoinHandle<()>,
}

async fn spawn_webhook_sink() -> WebhookSink {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let received: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let received_clone = received.clone();

    use axum::Router;
    use axum::routing::post;
    let app = Router::new().route(
        "/notify",
        post(move |AxumJson(body): AxumJson<serde_json::Value>| {
            let bucket = received_clone.clone();
            async move {
                bucket.lock().expect("bucket").push(body);
                axum::http::StatusCode::OK
            }
        }),
    );

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    WebhookSink {
        received,
        addr,
        _shutdown: handle,
    }
}

fn make_agent_spawned_event(execution_id: &str) -> ApxmEvent {
    ApxmEvent::root(
        AgentSpawnedPayload {
            node_id: 1,
            agent_code: "module.crm".to_string(),
            parent_execution_id: execution_id.to_string(),
            profile: Some("acp-codex".to_string()),
            process_id: None,
            scope_policy: None,
        },
        EventSource::Runtime,
        execution_id,
    )
}

#[tokio::test]
async fn webhook_carries_same_envelope_as_sse() {
    let sink = spawn_webhook_sink().await;
    let url = format!("http://{}/notify", sink.addr);
    let dispatcher = Arc::new(WebhookDispatcher::new(url).expect("webhook dispatcher"));

    let event = make_agent_spawned_event("exec-2");
    let expected = serde_json::to_value(&event).expect("serialize event");
    dispatcher.dispatch(event);

    for _ in 0..20 {
        if !sink.received.lock().expect("bucket").is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let recv = sink.received.lock().expect("bucket").clone();
    assert_eq!(recv.len(), 1);
    // The webhook body must equal the SSE envelope verbatim — same
    // `{meta, payload}` shape, no extra wrapping.
    assert_eq!(recv[0]["meta"]["seq"], expected["meta"]["seq"]);
    assert_eq!(recv[0]["payload"], expected["payload"]);
}

#[tokio::test]
async fn webhook_failure_does_not_block_execution() {
    // Point at a port nobody is listening on; the dispatch must
    // return immediately and surface no error to the caller.
    let dispatcher =
        Arc::new(WebhookDispatcher::new("http://127.0.0.1:1/notify").expect("dispatcher"));

    let started = std::time::Instant::now();
    dispatcher.dispatch(make_agent_spawned_event("exec-3"));
    assert!(
        started.elapsed() < std::time::Duration::from_millis(100),
        "dispatch must not block — fire and forget"
    );
}

#[tokio::test]
async fn webhook_skips_non_lifecycle_events() {
    let sink = spawn_webhook_sink().await;
    let url = format!("http://{}/notify", sink.addr);
    let dispatcher = Arc::new(WebhookDispatcher::new(url).expect("webhook dispatcher"));

    // OperationStart is observability noise — the dispatcher must
    // filter it out so external sinks don't drown.
    dispatcher.dispatch(ApxmEvent::root(
        OperationStartPayload {
            node_id: 1,
            op_type: AISOperationType::Ask,
            context: None,
        },
        EventSource::Runtime,
        "exec-4",
    ));

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let recv = sink.received.lock().expect("bucket").clone();
    assert!(
        recv.is_empty(),
        "operation_start must not fan out to webhooks: {recv:?}"
    );
}
