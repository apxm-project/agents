//! End-to-end test: the calculator demo runs via `dekk apxm execute` with mock
//! backend and produces a result containing "42".
//!
//! Gated behind `--features python_tools_integration` because it needs Python
//! and the `apxm` frontend package importable.
//!
//! Run with:
//!
//!     dekk apxm test -p apxm-runtime --features python_tools_integration calculator_demo

#![cfg(feature = "python_tools_integration")]

use std::path::PathBuf;
use std::process::Command;

/// Locate the demo script relative to CARGO_MANIFEST_DIR.
fn demo_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples/python/getting-started/calculator_agent.py")
}

/// Locate the apxm Python package root for PYTHONPATH.
fn apxm_python_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../compiler/apxm-frontend/python")
}

#[test]
fn test_calculator_demo_mock_execution() {
    let demo = demo_path();
    assert!(
        demo.exists(),
        "calculator_agent.py not found at {}",
        demo.display()
    );

    let python_root = apxm_python_root();
    let pythonpath = format!(
        "{}:{}",
        python_root.display(),
        std::env::var("PYTHONPATH").unwrap_or_default()
    );

    let output = Command::new("python")
        .arg(demo.to_str().unwrap())
        .env("APXM_MOCK_BACKEND", "1")
        .env("PYTHONPATH", &pythonpath)
        .output()
        .expect("failed to run calculator_agent.py");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "calculator_agent.py failed (exit {:?}):\nstdout: {}\nstderr: {}",
        output.status.code(),
        stdout,
        stderr,
    );

    assert!(
        stdout.contains("42"),
        "Expected output to contain '42', got:\nstdout: {}\nstderr: {}",
        stdout,
        stderr,
    );
}
