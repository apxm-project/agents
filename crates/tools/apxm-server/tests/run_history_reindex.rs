//! Corrupt/partial artifact diagnostics tests for the reindex walk (spec 0013 T032).
//!
//! These tests exercise the `POST /v1/runs/reindex` HTTP endpoint with
//! `APXM_RUNS_ROOT` pointing at a temporary directory, asserting on the JSON
//! response counts and diagnostics rather than internal state.

use apxm_backends::llm::backends::mock::MockLLMBackend;
use apxm_server::test_support;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use std::path::PathBuf;
use tower::ServiceExt;

async fn test_app() -> Router {
    test_support::test_app_with_mock(MockLLMBackend::static_response("ok")).await
}

async fn post_reindex(app: &Router) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/v1/runs/reindex")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn make_run_dir(root: &PathBuf, workflow_id: &str, execution_id: &str) -> PathBuf {
    let dir = root.join(workflow_id).join(execution_id);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_artifact(dir: &PathBuf, content: &str) {
    std::fs::write(dir.join("run.json"), content).unwrap();
}

// ── APXM_RUNS_ROOT unset ─────────────────────────────────────────────────────

#[tokio::test]
#[allow(unsafe_code)]
async fn reindex_skips_gracefully_when_runs_root_unset() {
    let app = test_app().await;
    // SAFETY: tests run with --test-threads=1 or isolated; env mutation contained.
    unsafe { std::env::remove_var("APXM_RUNS_ROOT") };
    let (status, body) = post_reindex(&app).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["artifacts_found"].as_u64(), Some(0));
    assert_eq!(body["records_loaded"].as_u64(), Some(0));
    let diags = body["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags
            .iter()
            .any(|d| d.as_str().map_or(false, |s| s.contains("APXM_RUNS_ROOT"))),
        "diagnostics must mention APXM_RUNS_ROOT, got: {diags:?}"
    );
}

// ── Empty runs root ───────────────────────────────────────────────────────────

#[tokio::test]
#[allow(unsafe_code)]
async fn reindex_returns_zero_counts_for_empty_root() {
    let app = test_app().await;
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: tests run with --test-threads=1 or isolated; env mutation contained.
    unsafe { std::env::set_var("APXM_RUNS_ROOT", tmp.path()) };
    let (status, body) = post_reindex(&app).await;
    unsafe { std::env::remove_var("APXM_RUNS_ROOT") };
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["artifacts_found"].as_u64(), Some(0));
    assert_eq!(body["records_loaded"].as_u64(), Some(0));
}

// ── Valid artifact ────────────────────────────────────────────────────────────

#[tokio::test]
#[allow(unsafe_code)]
async fn reindex_counts_valid_artifact() {
    let app = test_app().await;
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = make_run_dir(&tmp.path().to_path_buf(), "wf-001", "exec-abc");
    write_artifact(
        &run_dir,
        r#"{"run_id":"exec-abc","workflow_id":"wf-001","status":"succeeded","started_at_ms":1000,"finished_at_ms":2000,"duration_ms":1000}"#,
    );
    // SAFETY: tests run with --test-threads=1 or isolated; env mutation contained.
    unsafe { std::env::set_var("APXM_RUNS_ROOT", tmp.path()) };
    let (status, body) = post_reindex(&app).await;
    unsafe { std::env::remove_var("APXM_RUNS_ROOT") };
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["artifacts_found"].as_u64(), Some(1));
    assert_eq!(body["records_loaded"].as_u64(), Some(1));
}

// ── Corrupt artifact ──────────────────────────────────────────────────────────

#[tokio::test]
#[allow(unsafe_code)]
async fn reindex_records_diagnostic_for_corrupt_artifact() {
    let app = test_app().await;
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = make_run_dir(&tmp.path().to_path_buf(), "wf-bad", "exec-bad");
    write_artifact(&run_dir, "{ not valid json }");
    // SAFETY: tests run with --test-threads=1 or isolated; env mutation contained.
    unsafe { std::env::set_var("APXM_RUNS_ROOT", tmp.path()) };
    let (status, body) = post_reindex(&app).await;
    unsafe { std::env::remove_var("APXM_RUNS_ROOT") };
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["artifacts_found"].as_u64(), Some(1));
    assert_eq!(body["records_loaded"].as_u64(), Some(0));
    let diags = body["diagnostics"].as_array().expect("diagnostics array");
    assert!(!diags.is_empty(), "corrupt artifact must produce a diagnostic");
}

// ── Partial artifact (missing optional fields) ────────────────────────────────

#[tokio::test]
#[allow(unsafe_code)]
async fn reindex_handles_partial_artifact_missing_optional_fields() {
    let app = test_app().await;
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = make_run_dir(&tmp.path().to_path_buf(), "wf-partial", "exec-partial");
    // finished_at_ms and duration_ms are optional.
    write_artifact(
        &run_dir,
        r#"{"run_id":"exec-partial","workflow_id":"wf-partial","status":"running","started_at_ms":5000}"#,
    );
    // SAFETY: tests run with --test-threads=1 or isolated; env mutation contained.
    unsafe { std::env::set_var("APXM_RUNS_ROOT", tmp.path()) };
    let (status, body) = post_reindex(&app).await;
    unsafe { std::env::remove_var("APXM_RUNS_ROOT") };
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["artifacts_found"].as_u64(), Some(1));
    assert_eq!(body["records_loaded"].as_u64(), Some(1));
    let diags = body["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags.is_empty(),
        "valid partial artifact must not produce diagnostics"
    );
}

// ── Mixed valid and corrupt ───────────────────────────────────────────────────

#[tokio::test]
#[allow(unsafe_code)]
async fn reindex_mixes_valid_and_corrupt_artifacts() {
    let app = test_app().await;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    let good = make_run_dir(&root, "wf-mix", "exec-good");
    write_artifact(
        &good,
        r#"{"run_id":"exec-good","workflow_id":"wf-mix","status":"succeeded","started_at_ms":1}"#,
    );
    let bad = make_run_dir(&root, "wf-mix", "exec-bad");
    write_artifact(&bad, "GARBAGE");

    // SAFETY: tests run with --test-threads=1 or isolated; env mutation contained.
    unsafe { std::env::set_var("APXM_RUNS_ROOT", tmp.path()) };
    let (status, body) = post_reindex(&app).await;
    unsafe { std::env::remove_var("APXM_RUNS_ROOT") };

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["artifacts_found"].as_u64(), Some(2));
    assert_eq!(body["records_loaded"].as_u64(), Some(1));
    let diags = body["diagnostics"].as_array().expect("diagnostics array");
    assert_eq!(diags.len(), 1, "exactly one diagnostic for the corrupt file");
}
