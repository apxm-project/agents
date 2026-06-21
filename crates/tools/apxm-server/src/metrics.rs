//! Prometheus text exposition for process-level server metrics.

use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

static HTTP_REQUESTS_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Increment the process-wide HTTP request counter (called from middleware).
pub(crate) fn record_http_request() {
    HTTP_REQUESTS_TOTAL.fetch_add(1, Ordering::Relaxed);
}

/// Increment HTTP request metrics for every served request.
pub(crate) async fn record_request_middleware(req: Request<Body>, next: Next) -> Response {
    record_http_request();
    next.run(req).await
}

/// `GET /metrics` — Prometheus text scrape endpoint.
pub(crate) async fn scrape_metrics(state: axum::extract::State<AppState>) -> Response {
    if !state.server_config.observability.metrics_enabled {
        return (StatusCode::NOT_FOUND, "metrics disabled").into_response();
    }

    let uptime = state
        .start_time
        .elapsed()
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0);
    let requests = HTTP_REQUESTS_TOTAL.load(Ordering::Relaxed);
    let body = format!(
        "# HELP apxm_server_uptime_seconds Process uptime in seconds.\n\
         # TYPE apxm_server_uptime_seconds gauge\n\
         apxm_server_uptime_seconds {uptime:.3}\n\
         # HELP apxm_server_http_requests_total Total HTTP requests served.\n\
         # TYPE apxm_server_http_requests_total counter\n\
         apxm_server_http_requests_total {requests}\n"
    );

    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; version=0.0.4"),
        )],
        body,
    )
        .into_response()
}

