//! Integration tests for the Python tool worker bridge.
//!
//! Gated behind the `python_tools_integration` feature so CI without a
//! conda env (and the `apxm` Python package on PYTHONPATH) does not fail.
//!
//! Run with:
//!
//!     dekk apxm test --features python_tools_integration python_tools

#![cfg(feature = "python_tools_integration")]

use apxm_runtime::python_tools::{
    CallRequest, PROTOCOL_VERSION, PythonToolRegistry, PythonToolWorker, ToolDescriptor,
    WorkerRequest, WorkerResponse,
};
use std::time::Duration;

// Strings used to invoke `python` from tests. Centralized here so the test
// surface stays in one place.
const PYTHON_CMD: &str = "python";
const PYTHON_C_FLAG: &str = "-c";
const PYTHONPATH_VAR: &str = "PYTHONPATH";

// ---------------------------------------------------------------------------
// Pure-unit tests (no subprocess)
// ---------------------------------------------------------------------------

#[test]
fn test_registry_resolve() {
    let json = r#"[
        {
            "handler_id": "sha256:abc",
            "module": "mytools",
            "qualname": "add",
            "name": "add",
            "schema": {"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}}}
        }
    ]"#;

    let registry = PythonToolRegistry::from_json(json).unwrap();
    assert!(registry.contains("add"));
    assert!(!registry.contains("subtract"));

    let desc = registry.resolve("add").unwrap();
    assert_eq!(desc.handler_id, "sha256:abc");
}

#[test]
fn test_protocol_roundtrip() {
    let req = WorkerRequest::Call(CallRequest {
        v: PROTOCOL_VERSION,
        req_id: "u-42".into(),
        tool_id: "sha256:test".into(),
        args: serde_json::json!({"input": "hello"}),
        deadline_ms: 5000,
    });

    let serialized = serde_json::to_string(&req).unwrap();
    assert!(serialized.contains("\"type\":\"call\""));
    assert!(serialized.contains("\"req_id\":\"u-42\""));

    let resp_json = r#"{"v":1,"type":"result","req_id":"u-42","ok":true,"value":"world"}"#;
    let resp: WorkerResponse = serde_json::from_str(resp_json).unwrap();
    match resp {
        WorkerResponse::Result(r) => {
            assert!(r.ok);
            assert_eq!(r.req_id, "u-42");
            assert_eq!(r.value, Some(serde_json::json!("world")));
        }
    }
}

// ---------------------------------------------------------------------------
// End-to-end tests (require Python + apxm package importable)
// ---------------------------------------------------------------------------

/// Test fixture: a Python module that registers two `@tool` functions on import.
const TOOL_FIXTURE_SOURCE: &str = r#"
"""Test fixture — registers two @tool functions on import."""
from apxm.tools import tool

@tool
def add(a: int, b: int) -> int:
    """Add two integers."""
    return a + b

@tool
def boom() -> None:
    """Always raises."""
    raise ValueError("intentional kaboom")
"#;

/// Run a `python -c <snippet>` subprocess with PYTHONPATH set, returning
/// (success, stdout, stderr). Centralizes argv plumbing for all helpers.
async fn run_python(snippet: &str, pythonpath: &str) -> (bool, String, String) {
    let out = tokio::process::Command::new(PYTHON_CMD)
        .args([PYTHON_C_FLAG, snippet])
        .env(PYTHONPATH_VAR, pythonpath)
        .output()
        .await
        .expect("python invocation failed");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Compute a tool's handler_id by asking Python — avoids duplicating the
/// sha256(module:qualname) recipe in Rust.
async fn compute_handler_id(module: &str, qualname: &str, pythonpath: &str) -> String {
    let snippet = format!(
        "import importlib; m = importlib.import_module('{module}'); \
         t = getattr(m, '{qualname}'); print(t.handler_id)"
    );
    let (ok, stdout, stderr) = run_python(&snippet, pythonpath).await;
    assert!(ok, "python failed: stderr={stderr}");
    stdout.trim().to_string()
}

/// Locate the `apxm` Python package so the worker can `import apxm.tool_worker`
/// and `import apxm.tools` without requiring a `pip install`.
///
/// CARGO_MANIFEST_DIR points at `crates/runtime/apxm-runtime`; the package
/// lives at `crates/compiler/apxm-frontend/python/apxm`, so the directory
/// to put on PYTHONPATH is `crates/compiler/apxm-frontend/python`.
fn apxm_python_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../compiler/apxm-frontend/python")
}

/// Build a PYTHONPATH that prepends the temp fixture dir AND the apxm
/// package root to whatever PYTHONPATH the caller already has.
fn build_pythonpath(extra: &std::path::Path) -> String {
    let mut entries: Vec<String> = vec![
        extra.display().to_string(),
        apxm_python_root().display().to_string(),
    ];
    if let Ok(prev) = std::env::var(PYTHONPATH_VAR) {
        if !prev.is_empty() {
            entries.push(prev);
        }
    }
    entries.join(":")
}

/// Skip the test gracefully if `python -c "import apxm.tools"` fails even
/// after PYTHONPATH is set up (covers the case where Python itself is missing).
async fn python_apxm_available(pythonpath: &str) -> bool {
    run_python("import apxm.tools", pythonpath).await.0
}

/// End-to-end: real subprocess, real Python tool function, real result.
///
/// Verifies the full Rust→Python tool dispatch path:
///   PythonToolWorker::spawn → manifest tempfile import → @tool registry
///   lookup → fn() execution → NDJSON result → worker.call() return.
///
/// Also exercises the error path (Python exception → structured RuntimeError).
#[tokio::test]
async fn test_worker_calls_real_python_tool() {
    let tmp = tempfile::tempdir().unwrap();
    let module_name = "apxm_e2e_fixture";
    let fixture_path = tmp.path().join(format!("{module_name}.py"));
    std::fs::write(&fixture_path, TOOL_FIXTURE_SOURCE).unwrap();

    let pythonpath = build_pythonpath(tmp.path());

    if !python_apxm_available(&pythonpath).await {
        eprintln!("Skipping: `import apxm.tools` failed in subprocess Python");
        return;
    }
    let (add_id, boom_id) = tokio::join!(
        compute_handler_id(module_name, "add", &pythonpath),
        compute_handler_id(module_name, "boom", &pythonpath),
    );

    let manifest = serde_json::to_string(&[
        serde_json::json!({
            "handler_id": add_id, "module": module_name, "qualname": "add",
            "name": "add", "schema": {"type": "object"},
        }),
        serde_json::json!({
            "handler_id": boom_id, "module": module_name, "qualname": "boom",
            "name": "boom", "schema": {"type": "object"},
        }),
    ])
    .unwrap();

    let worker = PythonToolWorker::spawn_with_env(&manifest, &[("PYTHONPATH", &pythonpath)])
        .await
        .expect("worker should spawn");

    // ----- happy path: add(7, 11) == 18 -----
    let value = worker
        .call(
            &add_id,
            serde_json::json!({"a": 7, "b": 11}),
            Duration::from_secs(15),
        )
        .await
        .expect("add should succeed");
    assert_eq!(value, serde_json::json!(18));

    // ----- error path: boom() returns structured Capability error -----
    let err = worker
        .call(&boom_id, serde_json::json!({}), Duration::from_secs(15))
        .await
        .expect_err("boom should propagate the Python exception");
    let msg = format!("{err}");
    assert!(msg.contains("ValueError"), "missing ValueError tag: {msg}");
    assert!(msg.contains("kaboom"), "missing exception message: {msg}");

    // ----- unknown handler_id is rejected by the worker -----
    let err = worker
        .call(
            "sha256:does_not_exist",
            serde_json::json!({}),
            Duration::from_secs(5),
        )
        .await
        .expect_err("unknown handler should fail");
    assert!(format!("{err}").contains("unknown_handler"));
}

/// Sanity check: the registry's manifest_json roundtrips through the full
/// ToolDescriptor type (catches accidental field renames).
#[test]
fn test_registry_manifest_roundtrip_all_fields() {
    let descriptors = vec![ToolDescriptor {
        handler_id: "sha256:full".into(),
        module: "m".into(),
        qualname: "fn".into(),
        name: "fn".into(),
        schema: serde_json::json!({"type": "object", "properties": {"x": {"type": "integer"}}}),
    }];
    let registry = PythonToolRegistry::from_descriptors(descriptors);
    let manifest = registry.manifest_json().unwrap();
    let parsed: Vec<ToolDescriptor> = serde_json::from_str(&manifest).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].handler_id, "sha256:full");
    assert_eq!(parsed[0].schema["properties"]["x"]["type"], "integer");
}
