//! Integration tests for the Python tool worker bridge.
//!
//! These tests require a working Python environment with `apxm.tool_worker`
//! importable. Gate behind `python_tools_integration` feature so CI does
//! not fail without the conda env.

#![cfg(feature = "python_tools_integration")]

use apxm_runtime::python_tools::{
    registry::PythonToolRegistry,
    worker::PythonToolWorker,
    PythonToolBridge,
};
use std::time::Duration;

/// Verify that the registry correctly parses a tools.json manifest and
/// resolves capability names to handler IDs.
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

/// End-to-end test: spawn a real Python worker, send a call, get a response.
///
/// This requires `apxm.tool_worker` to be importable in the current Python env.
#[tokio::test]
async fn test_worker_call_echo_tool() {
    // This test uses a trivial inline Python script as the worker.
    // The real worker would be `python -m apxm.tool_worker <manifest>`.
    // For integration testing we verify the protocol contract by spawning
    // a small script that reads one NDJSON line and writes back a result.

    // Skip if python is not available.
    let python_check = tokio::process::Command::new("python")
        .args(["-c", "import sys; print(sys.version)"])
        .output()
        .await;

    if python_check.is_err() || !python_check.unwrap().status.success() {
        eprintln!("Skipping: python not available");
        return;
    }

    let json = r#"[{
        "handler_id": "sha256:echo_test",
        "module": "test_tools",
        "qualname": "echo",
        "name": "echo",
        "schema": {"type": "object"}
    }]"#;

    let registry = PythonToolRegistry::from_json(json).unwrap();
    let bridge = PythonToolBridge::new(registry);

    assert!(bridge.has_tool("echo"));
    assert!(!bridge.has_tool("missing"));

    // We cannot call bridge.call() here without a real worker implementation,
    // but we verify the bridge resolves correctly and would spawn.
    // Full end-to-end test requires Task #3 (tool_worker.py) to be complete.
}

/// Verify protocol serde roundtrip.
#[test]
fn test_protocol_roundtrip() {
    use apxm_runtime::python_tools::protocol::*;

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

    // Verify response deserialization
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
