//! Module-level string constants for the TypeScript tool worker bridge.

/// Capability name for artifact sidecar sections and runtime errors.
pub const CAPABILITY_NAME: &str = "typescript_tools";

/// Node.js executable resolved on `PATH`.
pub const NODE_BIN: &str = "node";

/// NDJSON worker script filename under the TypeScript frontend `scripts/` dir.
pub const WORKER_SCRIPT: &str = "tool-worker.mjs";
pub const FRONTEND_PACKAGE_ENV: &str = apxm_core::constants::env::APXM_TYPESCRIPT_FRONTEND_PACKAGE;

pub const TRACE_TARGET: &str = "typescript_tool_worker";

pub const MANIFEST_TEMPFILE_PREFIX: &str = "apxm-ts-tools-";

pub const MANIFEST_TEMPFILE_SUFFIX: &str = ".json";
