//! In-process TypeScript authoring bridge (Node-API).
//!
//! The TypeScript authoring frontend records a FrontendGraph and lowers it to
//! canonical AIR through this Node-API addon — never through a CLI subprocess or
//! a network compile. It is a thin wrapper over the shared deterministic
//! lowering in `apxm-program`, so it produces AIR byte-identical to the Python
//! bridge for the same program.

// The Node-API FFI boundary is owned by napi; its generated registration code
// uses unsafe and trips the Rust 2024 unsafe-op lint.
#![allow(unsafe_code, unsafe_op_in_unsafe_fn)]

use napi::Error;
use napi_derive::napi;

/// Lower a FrontendGraph JSON string to canonical AIR JSON.
#[napi]
pub fn lower_frontend_graph(graph_json: String) -> napi::Result<String> {
    apxm_program::lower_frontend_graph_json(&graph_json).map_err(Error::from_reason)
}

/// Return `null` if the FrontendGraph JSON verifies, otherwise a diagnostic
/// string describing why it was rejected.
#[napi]
pub fn verify_frontend_graph(graph_json: String) -> napi::Result<Option<String>> {
    let value: serde_json::Value =
        serde_json::from_str(&graph_json).map_err(|e| Error::from_reason(e.to_string()))?;
    let verdict = apxm_program::verify_frontend_graph_json(&value);
    if verdict.is_accepted() {
        Ok(None)
    } else {
        let rendered: Vec<String> = verdict
            .into_diagnostics()
            .into_iter()
            .map(|d| format!("{}:{}", d.code.slug(), d.location))
            .collect();
        Ok(Some(rendered.join("; ")))
    }
}
