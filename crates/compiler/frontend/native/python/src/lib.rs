//! In-process Python authoring bridge.
//!
//! The Python authoring frontend records a FrontendGraph and lowers it to
//! canonical AIR through this native extension — never through a CLI subprocess
//! or a network compile. The bridge is a thin wrapper over the shared
//! deterministic lowering in `apxm-program`.

// The `#[pymodule]`/`#[pyfunction]` macros expand to generated wrapper code that
// trips the Rust 2024 unsafe-op lint and a self-conversion on the error path;
// the generated boundary is owned by pyo3.
#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Lower a FrontendGraph JSON string to canonical AIR JSON.
#[pyfunction]
fn lower_frontend_graph(graph_json: &str) -> PyResult<String> {
    apxm_program::lower_frontend_graph_json(graph_json).map_err(PyValueError::new_err)
}

/// Return `None` if the FrontendGraph JSON verifies, otherwise a diagnostic
/// string describing why it was rejected.
#[pyfunction]
fn verify_frontend_graph(graph_json: &str) -> PyResult<Option<String>> {
    let value: serde_json::Value =
        serde_json::from_str(graph_json).map_err(|e| PyValueError::new_err(e.to_string()))?;
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

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(lower_frontend_graph, module)?)?;
    module.add_function(wrap_pyfunction!(verify_frontend_graph, module)?)?;
    Ok(())
}
