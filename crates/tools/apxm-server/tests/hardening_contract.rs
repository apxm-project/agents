//! Contract tests for spec 0006 observability hardening.

use apxm_backends::llm::backends::MockLLMBackend;
use apxm_driver::{ServerConfig, ServerSafetyConfig};
use apxm_server::test_support::test_app_with_config_and_mock;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn safety_config(rps: u32, max_body: usize) -> ServerConfig {
    ServerConfig {
        safety: ServerSafetyConfig {
            rate_limit_rps: Some(rps),
            rate_limit_burst: Some(rps),
            max_body_bytes: Some(max_body),
        },
        ..ServerConfig::default()
    }
}

#[tokio::test]
async fn metrics_endpoint_returns_prometheus_text() {
    let app = test_app_with_config_and_mock(ServerConfig::default(), MockLLMBackend::new()).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.expect("body").to_bytes();
    let text = String::from_utf8(body.to_vec()).expect("utf8");
    assert!(text.contains("apxm_server_uptime_seconds"));
    assert!(text.contains("apxm_server_http_requests_total"));
}

#[tokio::test]
async fn rate_limit_returns_429_with_stable_code() {
    let app = test_app_with_config_and_mock(safety_config(1, 1024), MockLLMBackend::new()).await;

    for _ in 0..2 {
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("first");
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("limited");

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = response.into_body().collect().await.expect("body").to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["code"], "rate_limit_exceeded");
}

#[tokio::test]
async fn body_limit_rejects_oversized_payload() {
    let app = test_app_with_config_and_mock(safety_config(0, 32), MockLLMBackend::new()).await;
    let oversized = vec![b'x'; 64];
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/execute")
                .header("content-type", "application/json")
                .body(Body::from(oversized))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn health_stays_available_under_default_config() {
    let app = test_app_with_config_and_mock(ServerConfig::default(), MockLLMBackend::new()).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
}
