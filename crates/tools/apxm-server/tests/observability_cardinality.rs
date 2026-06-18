//! Metrics cardinality regression tests.
//!
//! Asserts that metric families do not expose label combinations beyond the
//! bounded sets documented in `docs/observability/metrics.md`. Each test
//! scrapes the `/metrics` endpoint on a fixture server and parses the Prometheus
//! text format, then verifies that every label value belongs to its declared set.

use apxm_backends::llm::backends::MockLLMBackend;
use apxm_driver::ServerConfig;
use apxm_server::test_support::test_app_with_config_and_mock;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Parse the Prometheus text exposition format into (metric_name, label_map) pairs.
/// Only handles the simple `metric_name{label="value",...} N` line form.
fn parse_metric_families(body: &str) -> Vec<(String, std::collections::HashMap<String, String>)> {
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        // metric_name{...} value [timestamp]
        let (name_and_labels, _rest) = match line.split_once(' ') {
            Some(pair) => pair,
            None => continue,
        };
        let (name, labels_str) = if let Some(idx) = name_and_labels.find('{') {
            let name = &name_and_labels[..idx];
            let labels_str = name_and_labels[idx + 1..].trim_end_matches('}');
            (name, labels_str)
        } else {
            (name_and_labels, "")
        };
        let mut labels = std::collections::HashMap::new();
        for kv in labels_str.split(',') {
            let kv = kv.trim();
            if kv.is_empty() {
                continue;
            }
            if let Some((k, v)) = kv.split_once('=') {
                let v = v.trim_matches('"');
                labels.insert(k.to_string(), v.to_string());
            }
        }
        out.push((name.to_string(), labels));
    }
    out
}

async fn scrape_metrics() -> String {
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
    let bytes = response.into_body().collect().await.expect("body").to_bytes();
    String::from_utf8(bytes.to_vec()).expect("utf8 metrics body")
}

/// `apxm_workflow_runs_total` must only carry `status` values from the bounded set.
#[tokio::test]
async fn workflow_runs_total_status_cardinality() {
    let allowed: std::collections::HashSet<&str> =
        ["success", "failed", "cancelled"].into_iter().collect();
    let body = scrape_metrics().await;
    let families = parse_metric_families(&body);
    for (name, labels) in &families {
        if name != "apxm_workflow_runs_total" {
            continue;
        }
        if let Some(status) = labels.get("status") {
            assert!(
                allowed.contains(status.as_str()),
                "apxm_workflow_runs_total has unexpected status label value: {status:?}"
            );
        }
    }
}

/// `apxm_webhook_verify_total` must only carry `result` values from the bounded set.
#[tokio::test]
async fn webhook_verify_total_result_cardinality() {
    let allowed: std::collections::HashSet<&str> = ["ok", "fail"].into_iter().collect();
    let body = scrape_metrics().await;
    let families = parse_metric_families(&body);
    for (name, labels) in &families {
        if name != "apxm_webhook_verify_total" {
            continue;
        }
        if let Some(result) = labels.get("result") {
            assert!(
                allowed.contains(result.as_str()),
                "apxm_webhook_verify_total has unexpected result label value: {result:?}"
            );
        }
    }
}

/// `apxm_queue_depth` must only carry `state` values from the bounded set.
#[tokio::test]
async fn queue_depth_state_cardinality() {
    let allowed: std::collections::HashSet<&str> =
        ["pending", "processing", "dead_lettered"].into_iter().collect();
    let body = scrape_metrics().await;
    let families = parse_metric_families(&body);
    for (name, labels) in &families {
        if name != "apxm_queue_depth" {
            continue;
        }
        if let Some(state) = labels.get("state") {
            assert!(
                allowed.contains(state.as_str()),
                "apxm_queue_depth has unexpected state label value: {state:?}"
            );
        }
    }
}

/// No metric family may carry an `email`, `user_id`, or `ip` label — these are
/// personally identifiable and must never appear in metric labels.
#[tokio::test]
async fn no_pii_in_metric_labels() {
    let banned_labels = ["email", "user_id", "ip", "token", "secret", "key"];
    let body = scrape_metrics().await;
    let families = parse_metric_families(&body);
    for (name, labels) in &families {
        for banned in &banned_labels {
            assert!(
                !labels.contains_key(*banned),
                "metric {name} carries banned PII/secret label: {banned}"
            );
        }
    }
}
