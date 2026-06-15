use std::env;
use std::path::Path;

use apxm_core::constants::env as apxm_env;
use apxm_core::types::ApxmPathFormat;
use serde_json::Value as JsonValue;

const PYTHON_TOOLS_PREFIX: &str = "; __apxm_python_tools__ ";
/// Any APXM sidecar comment line (e.g. `; __apxm_hooks__ ...`). These are
/// `;`-prefixed metadata the MLIR parser cannot read, so they MUST be stripped
/// before compile. Only the python-tools sidecar is captured for the bridge; the
/// rest (hooks) travel inside the artifact as REGISTER_HOOK nodes + the tools
/// manifest, so stripping the comment is sufficient.
const SIDECAR_LINE_PREFIX: &str = "; __apxm_";

pub(crate) fn air_from_args(args: &JsonValue) -> Result<String, String> {
    let air = args
        .get("air")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty());
    let path = args
        .get("path")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty());
    air_from_parts(air, path)
}

pub(crate) fn air_from_parts(air: Option<&str>, path: Option<&str>) -> Result<String, String> {
    match (air, path) {
        (Some(_), Some(_)) => Err("pass either 'air' or 'path', not both".to_string()),
        (Some(air), None) => Ok(strip_python_tools_sidecar(air).0),
        (None, Some(path)) => air_from_path(Path::new(path)),
        (None, None) => Err("missing required argument: pass 'air' text or 'path'".to_string()),
    }
}

pub(crate) fn air_from_path(path: &Path) -> Result<String, String> {
    let format = ApxmPathFormat::from_path(path);
    if format.is_air_source() {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("failed to read AIR '{}': {error}", path.display()))?;
        return Ok(strip_python_tools_sidecar(&text).0);
    }
    if format.is_python_frontend() {
        return emit_air_from_python(path);
    }
    Err(format!(
        "unsupported workflow source '{}'; use .air or a Python frontend file",
        path.display()
    ))
}

pub(crate) fn strip_python_tools_sidecar(air: &str) -> (String, Option<Vec<u8>>) {
    let mut sidecar = None;
    let mut filtered = String::with_capacity(air.len());
    for line in air.lines() {
        if let Some(value) = line.strip_prefix(PYTHON_TOOLS_PREFIX) {
            sidecar = Some(value.as_bytes().to_vec());
        } else if line.starts_with(SIDECAR_LINE_PREFIX) {
            // Other APXM sidecar comment (e.g. __apxm_hooks__): strip so the
            // MLIR parser never sees a `;` line; nothing to capture here.
        } else {
            if !filtered.is_empty() {
                filtered.push('\n');
            }
            filtered.push_str(line);
        }
    }
    (filtered, sidecar)
}

fn emit_air_from_python(path: &Path) -> Result<String, String> {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let python_frontend = repo_root.join("crates/compiler/apxm-frontend/python");
    let mut pythonpath_entries = vec![python_frontend, repo_root];
    if let Some(parent) = path.parent() {
        pythonpath_entries.push(parent.to_path_buf());
    }
    if let Some(existing) = env::var_os(apxm_env::PYTHONPATH) {
        pythonpath_entries.extend(env::split_paths(&existing));
    }
    let pythonpath = env::join_paths(pythonpath_entries)
        .map_err(|error| format!("failed to build PYTHONPATH for APXM Python frontend: {error}"))?;

    for candidate in ["python3", "python"] {
        let output = match std::process::Command::new(candidate)
            .arg(path)
            .env(apxm_env::PYTHONPATH, &pythonpath)
            .env(apxm_env::APXM_EMIT_AIR, apxm_env::flag_values::ENABLED)
            .output()
        {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "failed to run Python workflow '{}' with {candidate}: {error}",
                    path.display()
                ));
            }
        };
        if !output.status.success() {
            return Err(format!(
                "Python workflow '{}' failed: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let air = String::from_utf8(output.stdout).map_err(|error| {
            format!(
                "Python workflow '{}' emitted non-UTF8 AIR: {error}",
                path.display()
            )
        })?;
        let (air, _sidecar) = strip_python_tools_sidecar(&air);
        if air.trim().is_empty() {
            return Err(format!(
                "Python workflow '{}' emitted no AIR",
                path.display()
            ));
        }
        return Ok(air);
    }

    Err("Python interpreter not found on PATH (tried python3, python)".to_string())
}
