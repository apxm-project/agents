use std::path::Path;

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
        return Err(format!(
            "Python frontend paths are not accepted by the server compile API; precompile '{}' to AIR and pass 'air' or a .air path",
            path.display()
        ));
    }
    Err(format!(
        "unsupported workflow source '{}'; use .air or inline AIR text",
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

#[cfg(test)]
mod tests {
    use super::{air_from_path, strip_python_tools_sidecar};

    #[test]
    fn air_paths_are_read_without_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workflow.air");
        std::fs::write(
            &path,
            "module { }\n; __apxm_python_tools__ {\"unsafe\":\"sidecar\"}\n",
        )
        .unwrap();

        let air = air_from_path(&path).unwrap();
        assert_eq!(air, "module { }");
    }

    #[test]
    fn python_frontend_paths_are_rejected_by_server_compile_api() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workflow.py");
        std::fs::write(&path, "print('executed')\n").unwrap();

        let error = air_from_path(&path).unwrap_err();
        assert!(error.contains("Python frontend paths are not accepted"));
    }

    #[test]
    fn inline_air_sidecars_are_stripped() {
        let (air, sidecar) = strip_python_tools_sidecar(
            "module { }\n; __apxm_hooks__ {}\n; __apxm_python_tools__ {}\n",
        );
        assert_eq!(air, "module { }");
        assert_eq!(sidecar, Some(b"{}".to_vec()));
    }
}
