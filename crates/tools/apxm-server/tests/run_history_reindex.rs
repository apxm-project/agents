//! Corrupt/partial artifact diagnostics tests for the reindex walk (spec 0013 T032).
//!
//! These tests exercise the `POST /v1/runs/reindex` HTTP endpoint with
//! `APXM_RUNS_ROOT` pointing at a temporary directory, asserting on the JSON
//! response counts and diagnostics rather than internal state.

use apxm_backends::llm::backends::mock::MockLLMBackend;
use apxm_core::events::payload::RedactedContent;
use apxm_core::types::NodeMetrics;
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

async fn test_app_with_run_history_db(path: &std::path::Path) -> Router {
    test_support::test_app_with_run_history_db_and_mock(path, MockLLMBackend::static_response("ok"))
        .await
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
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn get_json(app: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
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

    let (status, history) = get_json(&app, "/v1/workflows/wf-001/runs").await;
    assert_eq!(status, StatusCode::OK);
    let runs = history["runs"].as_array().expect("runs array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["run_id"], "exec-abc");
    assert_eq!(runs[0]["workflow_id"], "wf-001");
    assert_eq!(runs[0]["run_root"], run_dir.display().to_string());

    let (status, summary) = get_json(&app, "/v1/runs/exec-abc/summary").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(summary["workflow_id"], "wf-001");
    assert_eq!(summary["run_root"], run_dir.display().to_string());
}

#[tokio::test]
#[allow(unsafe_code)]
async fn reindex_hydrates_node_detail_and_artifact_refs() {
    let app = test_app().await;
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = make_run_dir(&tmp.path().to_path_buf(), "wf-nodes", "exec-nodes");
    let node_dir = run_dir.join("nodes").join("01_node");
    std::fs::create_dir_all(&node_dir).unwrap();
    std::fs::write(
        node_dir.join("node.json"),
        serde_json::to_vec(&serde_json::json!({
            "run_id": "exec-nodes",
            "workflow_id": "wf-nodes",
            "node_id": 1,
            "node_name": "node",
            "node_dir": "01_node"
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        node_dir.join("output.json"),
        serde_json::to_vec(&serde_json::json!({
            "node_id": 1,
            "node_name": "node",
            "observed_at_ms": 1200,
            "output": RedactedContent::from_text("node output")
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        node_dir.join("metrics.json"),
        serde_json::to_vec(&serde_json::json!({
            "node_id": 1,
            "node_name": "node",
            "observed_at_ms": 1300,
            "metrics": NodeMetrics::new(1)
        }))
        .unwrap(),
    )
    .unwrap();
    write_artifact(
        &run_dir,
        r#"{
          "run_id":"exec-nodes",
          "workflow_id":"wf-nodes",
          "skill_id":"wf-nodes",
          "skill_version":"raw-workflow",
          "status":"succeeded",
          "started_at_ms":1000,
          "finished_at_ms":2000,
          "duration_ms":1000,
          "nodes":[{
            "node_id":1,
            "node_dir":"nodes/01_node",
            "output_json":"nodes/01_node/output.json",
            "metrics_json":"nodes/01_node/metrics.json"
          }]
        }"#,
    );

    unsafe { std::env::set_var("APXM_RUNS_ROOT", tmp.path()) };
    let (status, body) = post_reindex(&app).await;
    unsafe { std::env::remove_var("APXM_RUNS_ROOT") };
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["artifacts_found"].as_u64(), Some(1));
    assert_eq!(body["records_loaded"].as_u64(), Some(1));

    let (status, node) = get_json(&app, "/v1/runs/exec-nodes/nodes/1").await;
    assert_eq!(status, StatusCode::OK, "node detail failed: {node}");
    assert_eq!(node["node_id"], 1);
    assert_eq!(node["outputs"].as_array().map(Vec::len), Some(1));
    assert_eq!(node["metrics"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        node["artifacts"]["output_json"],
        "nodes/01_node/output.json"
    );
    assert_eq!(
        node["artifacts"]["metrics_json"],
        "nodes/01_node/metrics.json"
    );

    let (status, artifact) = get_json(
        &app,
        "/v1/runs/exec-nodes/artifacts/nodes/01_node/output.json",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "artifact fetch failed: {artifact}");
    assert_eq!(artifact["node_id"], 1);
}

#[tokio::test]
#[allow(unsafe_code)]
async fn reindex_preserves_noncanonical_node_artifact_refs() {
    let app = test_app().await;
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = make_run_dir(
        &tmp.path().to_path_buf(),
        "wf-custom-node",
        "exec-custom-node",
    );
    let node_dir = run_dir.join("nodes").join("custom-step-output");
    std::fs::create_dir_all(&node_dir).unwrap();
    std::fs::write(
        node_dir.join("node.json"),
        serde_json::to_vec(&serde_json::json!({
            "run_id": "exec-custom-node",
            "workflow_id": "wf-custom-node",
            "node_id": 7,
            "node_name": "Original Name",
            "node_dir": "custom-step-output"
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        node_dir.join("value.out.json"),
        serde_json::to_vec(&serde_json::json!({
            "node_id": 7,
            "node_name": "Renamed Later",
            "observed_at_ms": 1200,
            "output": RedactedContent::from_text("custom output")
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        node_dir.join("value.metrics.json"),
        serde_json::to_vec(&serde_json::json!({
            "node_id": 7,
            "node_name": "Renamed Later",
            "observed_at_ms": 1300,
            "metrics": NodeMetrics::new(7)
        }))
        .unwrap(),
    )
    .unwrap();
    write_artifact(
        &run_dir,
        r#"{
          "run_id":"exec-custom-node",
          "workflow_id":"wf-custom-node",
          "skill_id":"wf-custom-node",
          "skill_version":"raw-workflow",
          "status":"succeeded",
          "started_at_ms":1000,
          "finished_at_ms":2000,
          "duration_ms":1000,
          "nodes":[{
            "node_id":7,
            "node_dir":"nodes/custom-step-output",
            "node_json":"nodes/custom-step-output/node.json",
            "output_json":"nodes/custom-step-output/value.out.json",
            "metrics_json":"nodes/custom-step-output/value.metrics.json"
          }]
        }"#,
    );

    unsafe { std::env::set_var("APXM_RUNS_ROOT", tmp.path()) };
    let (status, body) = post_reindex(&app).await;
    unsafe { std::env::remove_var("APXM_RUNS_ROOT") };
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["records_loaded"].as_u64(), Some(1));

    let (status, node) = get_json(&app, "/v1/runs/exec-custom-node/nodes/7").await;
    assert_eq!(status, StatusCode::OK, "node detail failed: {node}");
    assert_eq!(
        node["artifacts"]["node_dir"], "nodes/custom-step-output",
        "node detail must preserve durable node_dir instead of recomputing one"
    );
    assert_eq!(
        node["artifacts"]["output_json"],
        "nodes/custom-step-output/value.out.json"
    );
    assert_eq!(
        node["artifacts"]["metrics_json"],
        "nodes/custom-step-output/value.metrics.json"
    );
}

#[cfg(unix)]
#[tokio::test]
#[allow(unsafe_code)]
async fn reindex_skips_symlinked_node_artifact_escape() {
    use std::os::unix::fs::symlink;

    let app = test_app().await;
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = make_run_dir(&tmp.path().to_path_buf(), "wf-symlink", "exec-symlink");
    let node_dir = run_dir.join("nodes").join("01_node");
    std::fs::create_dir_all(&node_dir).unwrap();
    std::fs::write(
        node_dir.join("node.json"),
        serde_json::to_vec(&serde_json::json!({
            "run_id": "exec-symlink",
            "workflow_id": "wf-symlink",
            "node_id": 1,
            "node_name": "node",
            "node_dir": "01_node"
        }))
        .unwrap(),
    )
    .unwrap();
    let outside = tmp.path().join("outside-output.json");
    std::fs::write(
        &outside,
        serde_json::to_vec(&serde_json::json!({
            "node_id": 1,
            "node_name": "node",
            "observed_at_ms": 1200,
            "output": RedactedContent::from_text("outside")
        }))
        .unwrap(),
    )
    .unwrap();
    symlink(&outside, node_dir.join("output.json")).unwrap();
    std::fs::write(
        node_dir.join("metrics.json"),
        serde_json::to_vec(&serde_json::json!({
            "node_id": 1,
            "node_name": "node",
            "observed_at_ms": 1300,
            "metrics": NodeMetrics::new(1)
        }))
        .unwrap(),
    )
    .unwrap();
    write_artifact(
        &run_dir,
        r#"{
          "run_id":"exec-symlink",
          "workflow_id":"wf-symlink",
          "skill_id":"wf-symlink",
          "skill_version":"raw-workflow",
          "status":"succeeded",
          "started_at_ms":1000,
          "finished_at_ms":2000,
          "duration_ms":1000,
          "nodes":[{
            "node_id":1,
            "node_dir":"nodes/01_node",
            "output_json":"nodes/01_node/output.json",
            "metrics_json":"nodes/01_node/metrics.json"
          }]
        }"#,
    );

    unsafe { std::env::set_var("APXM_RUNS_ROOT", tmp.path()) };
    let (status, body) = post_reindex(&app).await;
    unsafe { std::env::remove_var("APXM_RUNS_ROOT") };
    assert_eq!(status, StatusCode::OK);
    let diagnostics = body["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .as_str()
            .is_some_and(|message| message.contains("escapes run root"))),
        "expected symlink escape diagnostic, got {diagnostics:?}"
    );

    let (status, node) = get_json(&app, "/v1/runs/exec-symlink/nodes/1").await;
    assert_eq!(status, StatusCode::OK, "node detail failed: {node}");
    assert_eq!(node["outputs"].as_array().map(Vec::len), None);
    assert_eq!(node["metrics"].as_array().map(Vec::len), Some(1));
    assert!(node["artifacts"]["output_json"].is_null());
    assert_eq!(
        node["artifacts"]["metrics_json"],
        "nodes/01_node/metrics.json"
    );
}

#[tokio::test]
#[allow(unsafe_code)]
async fn reindex_exposes_failed_run_results_artifact() {
    let app = test_app().await;
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = make_run_dir(&tmp.path().to_path_buf(), "wf-failed", "exec-failed");
    write_artifact(
        &run_dir,
        r#"{
          "run_id":"exec-failed",
          "workflow_id":"wf-failed",
          "skill_id":"wf-failed",
          "skill_version":"raw-workflow",
          "status":"failed",
          "started_at_ms":1000,
          "finished_at_ms":2000,
          "duration_ms":1000,
          "results_json":"results.json"
        }"#,
    );
    std::fs::write(
        run_dir.join("results.json"),
        serde_json::to_vec(&serde_json::json!({
            "object": "apxm.run.results",
            "artifact_schema_version": 1,
            "run_id": "exec-failed",
            "execution_id": "exec-failed",
            "workflow_id": "wf-failed",
            "status": "failed",
            "error": "backend down",
            "results": {}
        }))
        .unwrap(),
    )
    .unwrap();

    unsafe { std::env::set_var("APXM_RUNS_ROOT", tmp.path()) };
    let (status, body) = post_reindex(&app).await;
    unsafe { std::env::remove_var("APXM_RUNS_ROOT") };
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["records_loaded"].as_u64(), Some(1));

    let (status, artifacts) = get_json(&app, "/v1/runs/exec-failed/artifacts").await;
    assert_eq!(status, StatusCode::OK, "artifact list failed: {artifacts}");
    assert!(
        artifacts["artifacts"]
            .as_array()
            .expect("artifacts array")
            .iter()
            .any(|artifact| artifact["path"] == "results.json"),
        "reindexed failed run should list results.json: {artifacts}"
    );
    let (status, results) = get_json(&app, "/v1/runs/exec-failed/artifacts/results.json").await;
    assert_eq!(status, StatusCode::OK, "results fetch failed: {results}");
    assert_eq!(results["status"], "failed");
    assert_eq!(results["error"], "backend down");
}

// ── Durable SQLite run-history index ─────────────────────────────────────────

#[tokio::test]
#[allow(unsafe_code)]
async fn reindex_persists_workflow_rows_and_token_columns_in_sqlite() {
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");
    let db_path = tmp.path().join("sessions").join("runs.sqlite");
    let run_dir = make_run_dir(&runs_root, "wf-db", "exec-db");
    write_artifact(
        &run_dir,
        r#"{
          "run_id":"exec-db",
          "workflow_id":"wf-db",
          "skill_id":"wf-db",
          "skill_version":"raw-workflow",
          "session_id":"session-db",
          "session_dir":"/tmp/session-db",
          "trace_id":"trace-db",
          "status":"succeeded",
          "started_at_ms":1000,
          "finished_at_ms":2500,
          "duration_ms":1500,
          "input_tokens":12,
          "output_tokens":8,
          "total_tokens":20
        }"#,
    );

    let app = test_app_with_run_history_db(&db_path).await;
    // SAFETY: this integration test file is run serially in CI and by the
    // documented command; mutation is scoped to the test process.
    unsafe { std::env::set_var("APXM_RUNS_ROOT", &runs_root) };
    let (status, body) = post_reindex(&app).await;
    unsafe { std::env::remove_var("APXM_RUNS_ROOT") };
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["artifacts_found"].as_u64(), Some(1));
    assert_eq!(body["records_loaded"].as_u64(), Some(1));
    assert!(db_path.is_file(), "runs.sqlite should be created");

    // A new router with an empty hot ExecutionStore but the same SQLite index
    // must answer workflow history without APXM_RUNS_ROOT or another reindex.
    let app = test_app_with_run_history_db(&db_path).await;
    let (status, history) = get_json(&app, "/v1/workflows/wf-db/runs").await;
    assert_eq!(status, StatusCode::OK);
    let runs = history["runs"].as_array().expect("runs array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["run_id"], "exec-db");
    assert_eq!(runs[0]["trace_id"], "trace-db");
    assert_eq!(runs[0]["input_tokens"], 12);
    assert_eq!(runs[0]["output_tokens"], 8);
    assert_eq!(runs[0]["total_tokens"], 20);
    assert_eq!(runs[0]["retention_class"], "standard");
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
    assert!(
        !diags.is_empty(),
        "corrupt artifact must produce a diagnostic"
    );
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
    assert_eq!(
        diags.len(),
        1,
        "exactly one diagnostic for the corrupt file"
    );
}
