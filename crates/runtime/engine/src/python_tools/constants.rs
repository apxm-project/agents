//! Module-level string constants for the Python tool worker bridge.
//!
//! Centralized so the worker, registry, runtime wiring, and tests share a
//! single source of truth — eliminating drift if any of these names change
//! (e.g. renaming the Python entry-point module or the capability label).

/// Capability name attached to every `RuntimeError::Capability` raised from
/// this module. Matches the section name written into the artifact sidecar
/// and the section read out by `Runtime::extract_python_tools_section`.
pub const CAPABILITY_NAME: &str = "python_tools";

/// Python interpreter executable resolved on `PATH`. The conda env activation
/// performed by `dekk agents <cmd>` ensures this points at the project's env.
pub const PYTHON_BIN: &str = "python";

/// `python -m` flag — runs a module as a script.
pub const PYTHON_MODULE_FLAG: &str = "-m";

/// Python module that implements the NDJSON RPC worker entry point.
/// Invoked as `python -m {WORKER_MODULE} <manifest.json>`.
pub const WORKER_MODULE: &str = "apxm.tool_worker";

/// `tracing` target for all log lines emitted by the worker bridge — keeps
/// stderr/demuxer/error logs filterable as a single channel.
pub const TRACE_TARGET: &str = "python_tool_worker";

/// Filename prefix for the manifest tempfile passed to the worker via argv.
pub const MANIFEST_TEMPFILE_PREFIX: &str = "apxm-tools-";

/// Filename suffix for the manifest tempfile (selected so editors and
/// `file(1)` recognize the JSON content).
pub const MANIFEST_TEMPFILE_SUFFIX: &str = ".json";

/// Unbuffered Python env var so worker stderr/stdout arrives promptly.
pub const PYTHONUNBUFFERED: &str = "PYTHONUNBUFFERED";
