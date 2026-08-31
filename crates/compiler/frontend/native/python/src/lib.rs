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

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

/// State that must not be writable by evaluated Agent source.
///
/// The Python frontend evaluates an authored module in a restricted process.
/// Python objects and module globals in that process remain reflective, so a
/// graph kept in a Python dict (or in function defaults) is not an integrity
/// boundary. These registries live in Rust instead. A source-port process
/// captures one source bundle and exits, so process-local state is sufficient
/// and avoids introducing a persistent storage lifecycle.
const MAX_GRAPH_SNAPSHOTS: usize = 4096;

struct GraphSnapshot {
    // Keep the owner alive for as long as its pointer is in the registry. This
    // prevents CPython from reusing an object address for a later definition.
    _owner: Py<PyAny>,
    json: String,
}

static GRAPH_SNAPSHOTS: OnceLock<Mutex<HashMap<usize, GraphSnapshot>>> = OnceLock::new();
static AUTHORED_SOURCE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn graph_snapshots() -> &'static Mutex<HashMap<usize, GraphSnapshot>> {
    GRAPH_SNAPSHOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn authored_source_slot() -> &'static Mutex<Option<String>> {
    AUTHORED_SOURCE.get_or_init(|| Mutex::new(None))
}

/// Seal one graph against the identity of its Python AgentDefinition.
///
/// A second seal for the same object is rejected rather than replacing the
/// first value. This is deliberately a private frontend implementation hook;
/// callers retrieve a fresh Python value through [`frontend_graph`].
#[pyfunction]
fn seal_frontend_graph(owner: &Bound<'_, PyAny>, graph_json: &str) -> PyResult<()> {
    let key = owner.as_ptr() as usize;
    let mut snapshots = graph_snapshots()
        .lock()
        .map_err(|_| PyRuntimeError::new_err("frontend graph registry is poisoned"))?;
    if snapshots.contains_key(&key) {
        return Err(PyValueError::new_err(
            "the AgentDefinition already has a sealed frontend graph",
        ));
    }
    if snapshots.len() >= MAX_GRAPH_SNAPSHOTS {
        return Err(PyRuntimeError::new_err(
            "the frontend graph registry reached its capture limit",
        ));
    }
    snapshots.insert(
        key,
        GraphSnapshot {
            _owner: owner.clone().unbind(),
            json: graph_json.to_owned(),
        },
    );
    Ok(())
}

/// Return the graph sealed for one AgentDefinition as fresh Python data.
///
/// The JSON is parsed in Rust and recursively copied into a new Python value;
/// no Python JSON decoder, module global, function default, or source-visible
/// graph storage participates in this read.
#[pyfunction]
fn frontend_graph<'py>(py: Python<'py>, owner: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let key = owner.as_ptr() as usize;
    let graph_json = {
        let snapshots = graph_snapshots()
            .lock()
            .map_err(|_| PyRuntimeError::new_err("frontend graph registry is poisoned"))?;
        snapshots
            .get(&key)
            .map(|snapshot| snapshot.json.clone())
            .ok_or_else(|| PyKeyError::new_err("AgentDefinition has no sealed frontend graph"))?
    };
    let graph: serde_json::Value = serde_json::from_str(&graph_json).map_err(|error| {
        PyRuntimeError::new_err(format!("sealed frontend graph is invalid: {error}"))
    })?;
    json_to_python(py, &graph)
}

/// Install the exact submitted source for the current capture process.
///
/// The source-port harness calls this before evaluating submitted code. The
/// operation is one-shot: authored code cannot replace the source later with a
/// linecache or module-global forgery.
#[pyfunction]
fn set_authored_source(source: &str) -> PyResult<()> {
    let mut slot = authored_source_slot()
        .lock()
        .map_err(|_| PyRuntimeError::new_err("authored source registry is poisoned"))?;
    if slot.is_some() {
        return Err(PyValueError::new_err(
            "the capture source has already been sealed",
        ));
    }
    *slot = Some(source.to_owned());
    Ok(())
}

/// Read the source sealed by the source-port harness.
#[pyfunction]
fn authored_source() -> PyResult<String> {
    authored_source_slot()
        .lock()
        .map_err(|_| PyRuntimeError::new_err("authored source registry is poisoned"))?
        .clone()
        .ok_or_else(|| PyRuntimeError::new_err("no source was sealed for this capture"))
}

fn json_to_python<'py>(py: Python<'py>, value: &serde_json::Value) -> PyResult<Bound<'py, PyAny>> {
    use pyo3::IntoPyObject;

    match value {
        serde_json::Value::Null => Ok(py.None().into_bound(py)),
        serde_json::Value::Bool(value) => Ok(value.into_pyobject(py)?.to_owned().into_any()),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(value.into_pyobject(py)?.into_any())
            } else if let Some(value) = value.as_u64() {
                Ok(value.into_pyobject(py)?.into_any())
            } else if let Some(value) = value.as_f64() {
                Ok(value.into_pyobject(py)?.into_any())
            } else {
                Err(PyRuntimeError::new_err(
                    "sealed graph contains an invalid number",
                ))
            }
        }
        serde_json::Value::String(value) => Ok(value.into_pyobject(py)?.into_any()),
        serde_json::Value::Array(values) => {
            let converted = values
                .iter()
                .map(|value| json_to_python(py, value))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyList::new(py, converted)?.into_any())
        }
        serde_json::Value::Object(values) => {
            let dict = PyDict::new(py);
            for (key, value) in values {
                dict.set_item(key, json_to_python(py, value)?)?;
            }
            Ok(dict.into_any())
        }
    }
}

/// Lower a FrontendGraph JSON string to canonical AIR JSON.
#[pyfunction]
fn lower_frontend_graph(graph_json: &str) -> PyResult<String> {
    apxm_program::lower_frontend_graph_json(graph_json).map_err(PyValueError::new_err)
}

/// Compile a FrontendGraph JSON string into a complete executable artifact.
#[pyfunction]
fn compile_frontend_graph_artifact(graph_json: &str) -> PyResult<String> {
    apxm_program::compile_frontend_graph_artifact_json(graph_json).map_err(PyValueError::new_err)
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
    module.add_function(wrap_pyfunction!(compile_frontend_graph_artifact, module)?)?;
    module.add_function(wrap_pyfunction!(lower_frontend_graph, module)?)?;
    module.add_function(wrap_pyfunction!(verify_frontend_graph, module)?)?;
    module.add_function(wrap_pyfunction!(authored_source, module)?)?;
    module.add_function(wrap_pyfunction!(frontend_graph, module)?)?;
    module.add_function(wrap_pyfunction!(seal_frontend_graph, module)?)?;
    module.add_function(wrap_pyfunction!(set_authored_source, module)?)?;
    Ok(())
}
