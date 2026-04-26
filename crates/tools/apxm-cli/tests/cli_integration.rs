//! Integration tests for the APXM CLI.
//!
//! Tests the validate, analyze, ops, and template subcommands by invoking the
//! binary and checking stdout/stderr/exit-code.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::Command;

fn apxm() -> Command {
    let binary = std::path::PathBuf::from(env!("CARGO_BIN_EXE_apxm"));
    let mut cmd = Command::new(&binary);
    // Prevent color codes in test output
    cmd.env("NO_COLOR", "1");
    if let Some(lib_dir) = compiler_library_dir(&binary) {
        #[cfg(target_os = "macos")]
        let lib_var = "DYLD_LIBRARY_PATH";
        #[cfg(not(target_os = "macos"))]
        let lib_var = "LD_LIBRARY_PATH";

        let existing = std::env::var_os(lib_var);
        let joined = match existing {
            Some(existing) if !existing.is_empty() => {
                let mut paths = vec![lib_dir];
                paths.extend(std::env::split_paths(&existing));
                std::env::join_paths(paths).unwrap()
            }
            _ => std::env::join_paths([lib_dir]).unwrap(),
        };
        cmd.env(lib_var, joined);
    }
    cmd
}

fn compiler_library_dir(binary: &Path) -> Option<std::path::PathBuf> {
    let profile_dir = binary.parent()?;
    let installed_lib_dir = profile_dir.join("lib");
    if installed_lib_dir.is_dir() {
        return Some(installed_lib_dir);
    }

    let build_dir = profile_dir.join("build");
    let library_name = compiler_library_name();
    std::fs::read_dir(build_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("out").join("build").join("lib"))
        .find(|candidate| candidate.join(library_name).is_file())
}

fn compiler_library_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "libapxm_compiler_c.dylib"
    }
    #[cfg(target_os = "windows")]
    {
        "apxm_compiler_c.dll"
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        "libapxm_compiler_c.so"
    }
}

fn write_tmp_graph(content: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(content.as_bytes()).unwrap();
    f.flush().unwrap();
    f
}

fn write_tmp_file_named(name: &str, content: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::Builder::new().suffix(name).tempfile().unwrap();
    f.write_all(content.as_bytes()).unwrap();
    f.flush().unwrap();
    f
}

fn write_json_file(path: &Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn read_generated_snapshot(dir: &Path) -> BTreeMap<String, String> {
    let mut snapshot = BTreeMap::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_file() {
            snapshot.insert(
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(path).unwrap(),
            );
        }
    }
    snapshot
}

fn write_session_manifest(
    sessions_root: &Path,
    execution_id: &str,
    timestamp: &str,
) -> std::path::PathBuf {
    use apxm_core::constants;

    let session_dir = sessions_root.join(execution_id);
    std::fs::create_dir_all(&session_dir).unwrap();
    let manifest = serde_json::json!({
        "execution_id": execution_id,
        "graph_name": "test-graph",
        "timestamp": timestamp,
        "status": "completed",
        "duration_ms": 1234,
        "node_count": 1,
        "success": true
    });
    std::fs::write(
        session_dir.join(constants::session::files::MANIFEST),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    session_dir
}

fn write_tmp_driver_config(hook_log: &Path) -> tempfile::NamedTempFile {
    use apxm_driver::config::{HookConfig, HookEvent, MiddlewareConfig};

    fn render_scalar(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::String(value) => format!("{value:?}"),
            serde_json::Value::Bool(value) => value.to_string(),
            serde_json::Value::Number(value) => value.to_string(),
            other => panic!("unsupported config scalar: {other}"),
        }
    }

    fn render_table(header: &str, value: serde_json::Value, body: &mut String) {
        let serde_json::Value::Object(map) = value else {
            panic!("expected TOML table object");
        };
        body.push_str(header);
        body.push('\n');
        for (key, value) in map {
            if value.is_null() {
                continue;
            }
            body.push_str(&format!("{key} = {}\n", render_scalar(&value)));
        }
        body.push('\n');
    }

    let hook = HookConfig {
        event: HookEvent::NodeComplete,
        command: format!(
            "printf '%s|%s|%s\\n' {{{{node_id}}}} {{{{op_type}}}} {{{{success}}}} >> {}",
            hook_log.display()
        ),
        shell: None,
    };
    let middlewares = [
        MiddlewareConfig::Timeout {
            default_timeout_ms: Some(5000),
        },
        MiddlewareConfig::LoopGuard { max_repeats: 2 },
    ];

    let mut body = String::new();
    render_table("[[hooks]]", serde_json::to_value(&hook).unwrap(), &mut body);

    for middleware in middlewares {
        render_table(
            "[[middlewares]]",
            serde_json::to_value(&middleware).unwrap(),
            &mut body,
        );
    }

    write_tmp_file_named(".toml", &body)
}

// ─── Valid graphs ───────────────────────────────────────────────────────────

const VALID_ASK: &str = r#"{
  "name": "test-ask",
  "nodes": [{"id": 1, "name": "q", "op": "ASK", "attributes": {"template_str": "Hello"}}],
  "edges": [],
  "parameters": [],
  "metadata": {}
}"#;

const VALID_PIPELINE: &str = r#"{
  "name": "test-pipeline",
  "nodes": [
    {"id": 1, "name": "a", "op": "ASK", "attributes": {"template_str": "step 1"}},
    {"id": 2, "name": "b", "op": "ASK", "attributes": {"template_str": "step 2: {a}", "input_names": ["a"]}}
  ],
  "edges": [{"from": 1, "to": 2, "dependency": "Data"}],
  "parameters": [],
  "metadata": {}
}"#;

const VALID_PARALLEL: &str = r#"{
  "name": "test-parallel",
  "nodes": [
    {"id": 1, "name": "a", "op": "ASK", "attributes": {"template_str": "task a"}},
    {"id": 2, "name": "b", "op": "ASK", "attributes": {"template_str": "task b"}},
    {"id": 3, "name": "sync", "op": "WAIT_ALL", "attributes": {}}
  ],
  "edges": [
    {"from": 1, "to": 3, "dependency": "Data"},
    {"from": 2, "to": 3, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}"#;

const CONST_GRAPH: &str = r#"{
  "name": "const-graph",
  "nodes": [
    {"id": 1, "name": "value", "op": "CONST_STR", "attributes": {"value": "ok"}},
    {"id": 2, "name": "done", "op": "RETURN", "attributes": {}}
  ],
  "edges": [{"from": 1, "to": 2, "dependency": "Data"}],
  "parameters": [],
  "metadata": {}
}"#;

// ─── validate: valid graphs ─────────────────────────────────────────────────

#[test]
fn validate_valid_ask_json() {
    let f = write_tmp_graph(VALID_ASK);
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], true);
    assert!(v["errors"].as_array().unwrap().is_empty());
}

#[test]
fn validate_valid_pipeline_json() {
    let f = write_tmp_graph(VALID_PIPELINE);
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], true);
}

#[test]
fn run_json_errors_are_emitted_as_json() {
    let missing = tempfile::tempdir().unwrap().path().join("missing.apxmobj");
    let out = apxm()
        .args(["--json", "run", missing.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).trim().is_empty(),
        "expected clean stderr, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Failed to read artifact")
    );
}

#[test]
fn execute_json_errors_are_emitted_as_json() {
    let missing_config = tempfile::tempdir().unwrap().path().join("missing.toml");
    let out = apxm()
        .args([
            "--json",
            "--config",
            missing_config.to_str().unwrap(),
            "execute",
            "placeholder.air",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).trim().is_empty(),
        "expected clean stderr, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Failed to load configuration")
    );
}

#[test]
fn execute_json_with_local_controls_succeeds_end_to_end() {
    let graph = write_tmp_file_named(".json", CONST_GRAPH);
    let temp = tempfile::tempdir().unwrap();
    let hook_log = temp.path().join("hook.log");
    let sessions_root = temp.path().join("sessions");
    let config = write_tmp_driver_config(&hook_log);

    let out = apxm()
        .env("APXM_MOCK_BACKEND", "1")
        .args([
            "--json",
            "--config",
            config.path().to_str().unwrap(),
            "execute",
            graph.path().to_str().unwrap(),
            "--emit-session",
            sessions_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let session_dir = std::path::PathBuf::from(body["session_dir"].as_str().unwrap());
    assert!(session_dir.is_dir());
    assert_eq!(session_dir.parent().unwrap(), sessions_root);
    assert_eq!(body["content"], "ok");

    let hook_output = std::fs::read_to_string(&hook_log).unwrap();
    let hook_rows = hook_output
        .lines()
        .map(|line| line.split('|').map(str::to_string).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(hook_rows.len(), 2);
    assert!(hook_rows.iter().all(|row| row.len() == 3));
    assert!(hook_rows.iter().all(|row| row[2] == "true"));
}

// ─── validate: error cases ──────────────────────────────────────────────────

#[test]
fn validate_empty_name() {
    let f = write_tmp_graph(
        r#"{"name":"","nodes":[{"id":1,"name":"n","op":"ASK","attributes":{"template_str":"x"}}],"edges":[],"parameters":[]}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], false);
    assert!(
        v["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("name must not be empty"))
    );
}

#[test]
fn validate_rejects_node_id_placeholder_syntax() {
    let f = write_tmp_graph(
        r#"{"name":"node_id_placeholder","nodes":[{"id":1,"name":"a","op":"ASK","attributes":{"template_str":"step 1"}},{"id":2,"name":"b","op":"ASK","attributes":{"template_str":"step 2: {{node_1}}"}}],"edges":[{"from":1,"to":2,"dependency":"Data"}],"parameters":[],"metadata":{}}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], false);
    assert!(
        v["errors"].as_array().unwrap().iter().any(|e| {
            let msg = e.as_str().unwrap();
            msg.contains("node-id placeholder syntax")
                || msg.contains("references no known input or parameter")
        }),
        "expected node-id placeholder error, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn validate_unknown_op() {
    let f = write_tmp_graph(
        r#"{"name":"t","nodes":[{"id":1,"name":"n","op":"FAKE_OP","attributes":{}}],"edges":[],"parameters":[]}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("unknown op"))
    );
}

#[test]
fn validate_missing_required_attribute() {
    let f = write_tmp_graph(
        r#"{"name":"t","nodes":[{"id":1,"name":"n","op":"ASK","attributes":{}}],"edges":[],"parameters":[]}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("template_str"))
    );
}

#[test]
fn validate_duplicate_node_id() {
    let f = write_tmp_graph(
        r#"{"name":"t","nodes":[{"id":1,"name":"a","op":"ASK","attributes":{"template_str":"x"}},{"id":1,"name":"b","op":"ASK","attributes":{"template_str":"y"}}],"edges":[],"parameters":[]}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("duplicate node id"))
    );
}

#[test]
fn validate_self_loop() {
    let f = write_tmp_graph(
        r#"{"name":"t","nodes":[{"id":1,"name":"n","op":"ASK","attributes":{"template_str":"x"}}],"edges":[{"from":1,"to":1,"dependency":"Data"}],"parameters":[]}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("self-loop"))
    );
}

#[test]
fn validate_invalid_dependency_type() {
    let f = write_tmp_graph(
        r#"{"name":"t","nodes":[{"id":1,"name":"a","op":"ASK","attributes":{"template_str":"x"}},{"id":2,"name":"b","op":"ASK","attributes":{"template_str":"y"}}],"edges":[{"from":1,"to":2,"dependency":"Invalid"}],"parameters":[]}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("invalid dependency type"))
    );
}

#[test]
fn validate_edge_nonexistent_target() {
    let f = write_tmp_graph(
        r#"{"name":"t","nodes":[{"id":1,"name":"n","op":"ASK","attributes":{"template_str":"x"}}],"edges":[{"from":1,"to":99,"dependency":"Data"}],"parameters":[]}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("target node"))
    );
}

#[test]
fn validate_cycle_detection() {
    let f = write_tmp_graph(
        r#"{"name":"t","nodes":[{"id":1,"name":"a","op":"ASK","attributes":{"template_str":"x"}},{"id":2,"name":"b","op":"ASK","attributes":{"template_str":"y"}}],"edges":[{"from":1,"to":2,"dependency":"Data"},{"from":2,"to":1,"dependency":"Data"}],"parameters":[]}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("cycle"))
    );
}

#[test]
fn validate_file_not_found() {
    let out = apxm()
        .args(["validate", "/nonexistent/path.json"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn validate_invalid_json() {
    let f = write_tmp_graph("{not valid json");
    let out = apxm()
        .args(["validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn codegen_frontend_writes_generated_python_files() {
    let dir = tempfile::tempdir().unwrap();
    let out = apxm()
        .args([
            "--json",
            "codegen",
            "frontend",
            "--output-dir",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(out.status.success());

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["target"], "frontend");

    let files = v["files"].as_array().unwrap();
    assert!(files.iter().any(|item| item == "__init__.py"));
    assert!(files.iter().any(|item| item == "constants.py"));
    assert!(files.iter().any(|item| item == "operations.py"));
    assert!(files.iter().any(|item| item == "agents.py"));

    let constants = std::fs::read_to_string(dir.path().join("constants.py")).unwrap();
    let operations = std::fs::read_to_string(dir.path().join("operations.py")).unwrap();
    let agents = std::fs::read_to_string(dir.path().join("agents.py")).unwrap();

    assert!(constants.contains("MODEL: Final[str] = \"model\""));
    assert!(operations.contains("SPAWN_AGENT: Final = OpSpec("));
    assert!(agents.contains("claude: Final = AgentRef("));
}

// ─── analyze ────────────────────────────────────────────────────────────────

#[test]
fn analyze_parallel_graph() {
    let f = write_tmp_graph(VALID_PARALLEL);
    let out = apxm()
        .args(["--json", "analyze", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["max_parallelism"], 2);
    assert_eq!(v["depth"], 2);
    assert_eq!(v["node_count"], 3);
    assert_eq!(v["edge_count"], 2);
}

#[test]
fn analyze_single_node() {
    let f = write_tmp_graph(VALID_ASK);
    let out = apxm()
        .args(["--json", "analyze", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["max_parallelism"], 1);
    assert_eq!(v["depth"], 1);
    assert_eq!(v["speedup"]["estimated_speedup"], "1.00x");
}

#[test]
fn analyze_pipeline_graph() {
    let f = write_tmp_graph(VALID_PIPELINE);
    let out = apxm()
        .args(["--json", "analyze", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["max_parallelism"], 1);
    assert_eq!(v["depth"], 2);
}

// ─── ops ────────────────────────────────────────────────────────────────────

#[test]
fn ops_list_json() {
    let out = apxm().args(["--json", "ops", "list"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ops = v.as_array().unwrap();
    assert!(
        ops.len() >= 30,
        "expected at least 30 ops, got {}",
        ops.len()
    );
    // Check every op has required fields
    for op in ops {
        assert!(op["op"].is_string());
        assert!(op["category"].is_string());
        assert!(op["description"].is_string());
    }
}

#[test]
fn ops_list_category_filter() {
    let out = apxm()
        .args(["--json", "ops", "list", "--category", "reasoning"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ops = v.as_array().unwrap();
    for op in ops {
        assert_eq!(op["category"], "reasoning");
    }
    assert!(ops.len() >= 4, "expected at least 4 reasoning ops");
}

#[test]
fn ops_show_ask_json() {
    let out = apxm()
        .args(["--json", "ops", "show", "ASK"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["op"], "ASK");
    assert_eq!(v["category"], "reasoning");
    assert!(
        v["required_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "template_str")
    );
}

#[test]
fn ops_show_unknown() {
    let out = apxm()
        .args(["ops", "show", "NONEXISTENT"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

// ─── template ───────────────────────────────────────────────────────────────

#[test]
fn template_list_json() {
    let out = apxm()
        .args(["--json", "template", "list"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let templates = v.as_array().unwrap();
    assert!(templates.len() >= 6);
    for t in templates {
        assert!(t["name"].is_string());
        assert!(t["description"].is_string());
    }
}

#[test]
fn template_show_ask_json() {
    let out = apxm()
        .args(["--json", "template", "show", "ask"])
        .output()
        .unwrap();
    assert!(out.status.success());
    // Output should be valid JSON graph
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["name"].is_string());
    assert!(v["nodes"].is_array());
    assert!(v["edges"].is_array());
}

#[test]
fn template_show_unknown() {
    let out = apxm()
        .args(["template", "show", "nonexistent"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn template_roundtrip_validate() {
    // Get template JSON and feed it to validate
    let show_out = apxm()
        .args(["--json", "template", "show", "map-reduce"])
        .output()
        .unwrap();
    assert!(show_out.status.success());

    let f = write_tmp_graph(std::str::from_utf8(&show_out.stdout).unwrap());
    let val_out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(val_out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&val_out.stdout).unwrap();
    assert_eq!(v["valid"], true);
}

// ─── validate: edge cases ───────────────────────────────────────────────────

#[test]
fn validate_no_nodes() {
    let f = write_tmp_graph(r#"{"name":"t","nodes":[],"edges":[],"parameters":[]}"#);
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], false);
}

#[test]
fn validate_edge_nonexistent_source() {
    let f = write_tmp_graph(
        r#"{"name":"t","nodes":[{"id":1,"name":"n","op":"ASK","attributes":{"template_str":"x"}}],"edges":[{"from":99,"to":1,"dependency":"Data"}],"parameters":[]}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("source"))
    );
}

#[test]
fn validate_duplicate_parameter_name() {
    let f = write_tmp_graph(
        r#"{"name":"t","nodes":[{"id":1,"name":"n","op":"ASK","attributes":{"template_str":"x"}}],"edges":[],"parameters":[{"name":"p","type_name":"str"},{"name":"p","type_name":"int"}]}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    // Should have warning or error about duplicate parameter
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let has_dup = v["errors"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .chain(v["warnings"].as_array().unwrap_or(&vec![]).iter())
        .any(|e| e.as_str().unwrap_or("").contains("duplicate"));
    assert!(has_dup);
}

#[test]
fn validate_disconnected_graph() {
    // Two nodes with no edges — valid but might get a warning
    let f = write_tmp_graph(
        r#"{"name":"t","nodes":[{"id":1,"name":"a","op":"ASK","attributes":{"template_str":"x"}},{"id":2,"name":"b","op":"ASK","attributes":{"template_str":"y"}}],"edges":[],"parameters":[]}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], true);
}

#[test]
fn validate_parameter_invalid_type() {
    let f = write_tmp_graph(
        r#"{"name":"t","nodes":[{"id":1,"name":"n","op":"ASK","attributes":{"template_str":"x"}}],"edges":[],"parameters":[{"name":"p","type_name":"invalid_type"}]}"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    // Should succeed but with warning about non-standard type
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("non-standard"))
    );
}

// ─── analyze: edge cases ────────────────────────────────────────────────────

#[test]
fn analyze_disconnected_components() {
    // Two independent nodes — both should be in phase 1
    let f = write_tmp_graph(
        r#"{"name":"t","nodes":[{"id":1,"name":"a","op":"ASK","attributes":{"template_str":"x"}},{"id":2,"name":"b","op":"ASK","attributes":{"template_str":"y"}}],"edges":[],"parameters":[]}"#,
    );
    let out = apxm()
        .args(["--json", "analyze", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["max_parallelism"], 2);
    assert_eq!(v["depth"], 1);
}

#[test]
fn analyze_includes_entry_exit_nodes() {
    let f = write_tmp_graph(VALID_PIPELINE);
    let out = apxm()
        .args(["--json", "analyze", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["entry_nodes"].is_array());
    assert!(v["exit_nodes"].is_array());
}

// ─── ops: edge cases ────────────────────────────────────────────────────────

#[test]
fn ops_list_has_coordination_ops() {
    let out = apxm().args(["--json", "ops", "list"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ops: Vec<String> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["op"].as_str().unwrap().to_string())
        .collect();
    assert!(ops.contains(&"UPDATE_GOAL".to_string()));
    assert!(ops.contains(&"GUARD".to_string()));
    assert!(ops.contains(&"CLAIM".to_string()));
    assert!(ops.contains(&"PAUSE".to_string()));
    assert!(ops.contains(&"RESUME".to_string()));
}

#[test]
fn ops_show_has_long_description() {
    let out = apxm()
        .args(["--json", "ops", "show", "THINK"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["long_description"].is_string());
    assert!(!v["long_description"].as_str().unwrap().is_empty());
}

// ─── explain ───────────────────────────────────────────────────────────────

#[test]
fn explain_single_node_json() {
    let f = write_tmp_graph(VALID_ASK);
    let out = apxm()
        .args(["--json", "explain", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["graph_name"], "test-ask");
    assert_eq!(v["node_count"], 1);
    assert_eq!(v["edge_count"], 0);
    assert_eq!(v["depth"], 1);
    let flow = v["execution_flow"].as_array().unwrap();
    assert_eq!(flow.len(), 1);
    assert_eq!(flow[0]["phase"], 1);
    let nodes = flow[0]["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["op"], "ASK");
}

#[test]
fn explain_pipeline_json() {
    let f = write_tmp_graph(VALID_PIPELINE);
    let out = apxm()
        .args(["--json", "explain", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["depth"], 2);
    let flow = v["execution_flow"].as_array().unwrap();
    assert_eq!(flow.len(), 2);
    // Phase 1: single node, Phase 2: single node
    assert_eq!(flow[0]["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(flow[1]["nodes"].as_array().unwrap().len(), 1);
}

#[test]
fn explain_parallel_json() {
    let f = write_tmp_graph(VALID_PARALLEL);
    let out = apxm()
        .args(["--json", "explain", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let flow = v["execution_flow"].as_array().unwrap();
    // Phase 1 should have 2 parallel nodes
    assert!(flow[0]["parallel"].as_bool().unwrap());
    assert_eq!(flow[0]["nodes"].as_array().unwrap().len(), 2);
    assert!(v["summary"]["max_parallelism"].as_u64().unwrap() >= 2);
}

#[test]
fn explain_human_readable() {
    let f = write_tmp_graph(VALID_PIPELINE);
    let out = apxm()
        .args(["explain", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Phase 1"));
    assert!(stdout.contains("Phase 2"));
}

#[test]
fn explain_file_not_found() {
    let out = apxm()
        .args(["explain", "/nonexistent/file.json"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn explain_node_metadata() {
    let f = write_tmp_graph(VALID_ASK);
    let out = apxm()
        .args(["--json", "explain", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let node = &v["execution_flow"][0]["nodes"][0];
    assert!(node["category"].is_string());
    assert!(node["description"].is_string());
    assert!(node["latency"].is_string());
    assert!(node["latency_ms"].is_number());
}

// ─── codegen ───────────────────────────────────────────────────────────────

#[test]
fn codegen_frontend_is_idempotent() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output_dir = temp_dir.path().join("_generated");
    let output_dir_str = output_dir.to_str().unwrap();

    let first = apxm()
        .args([
            "--json",
            "codegen",
            "frontend",
            "--output-dir",
            output_dir_str,
        ])
        .output()
        .unwrap();
    assert!(first.status.success());
    let first_json: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first_json["target"], "frontend");
    assert_eq!(first_json["output_dir"], output_dir_str);

    let first_snapshot = read_generated_snapshot(&output_dir);
    assert_eq!(first_snapshot.len(), 7);
    assert!(first_snapshot.contains_key("__init__.py"));
    assert!(first_snapshot.contains_key("agents.py"));
    assert!(first_snapshot.contains_key("constants.py"));
    assert!(first_snapshot.contains_key("emission.py"));
    assert!(first_snapshot.contains_key("models.py"));
    assert!(first_snapshot.contains_key("operations.py"));
    assert!(first_snapshot.contains_key("providers.py"));

    let second = apxm()
        .args([
            "--json",
            "codegen",
            "frontend",
            "--output-dir",
            output_dir_str,
        ])
        .output()
        .unwrap();
    assert!(second.status.success());
    let second_json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    let second_snapshot = read_generated_snapshot(&output_dir);

    assert_eq!(first_json, second_json);
    assert_eq!(first_snapshot, second_snapshot);
}

// ─── tool ──────────────────────────────────────────────────────────────────

#[test]
fn tool_list_empty_json() {
    // Use a temp HOME to avoid reading real ~/.apxm/tools.json
    let tmp_home = tempfile::tempdir().unwrap();
    let out = apxm()
        .env("HOME", tmp_home.path())
        .args(["--json", "tool", "list"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.as_array().unwrap().is_empty());
}

#[test]
fn tool_add_and_list() {
    let tmp_home = tempfile::tempdir().unwrap();
    // Add a tool
    let out = apxm()
        .env("HOME", tmp_home.path())
        .args([
            "tool",
            "add",
            "test-search",
            "--description",
            "Test search tool",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    // List tools, should contain the added tool
    let out = apxm()
        .env("HOME", tmp_home.path())
        .args(["--json", "tool", "list"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let tools = v.as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "test-search");
    assert_eq!(tools[0]["description"], "Test search tool");
}

#[test]
fn tool_add_duplicate_fails() {
    let tmp_home = tempfile::tempdir().unwrap();
    // Add once
    apxm()
        .env("HOME", tmp_home.path())
        .args(["tool", "add", "dup-tool", "--description", "First"])
        .output()
        .unwrap();
    // Add same name again
    let out = apxm()
        .env("HOME", tmp_home.path())
        .args(["tool", "add", "dup-tool", "--description", "Second"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn tool_remove() {
    let tmp_home = tempfile::tempdir().unwrap();
    // Add then remove
    apxm()
        .env("HOME", tmp_home.path())
        .args(["tool", "add", "rm-tool", "--description", "To be removed"])
        .output()
        .unwrap();
    let out = apxm()
        .env("HOME", tmp_home.path())
        .args(["tool", "remove", "rm-tool"])
        .output()
        .unwrap();
    assert!(out.status.success());

    // Verify it's gone
    let out = apxm()
        .env("HOME", tmp_home.path())
        .args(["--json", "tool", "list"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.as_array().unwrap().is_empty());
}

#[test]
fn tool_remove_nonexistent_fails() {
    let tmp_home = tempfile::tempdir().unwrap();
    let out = apxm()
        .env("HOME", tmp_home.path())
        .args(["tool", "remove", "no-such-tool"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

// ─── session ───────────────────────────────────────────────────────────────

#[test]
fn session_list_prefers_local_root_over_global_home() {
    let workspace = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();

    write_session_manifest(
        &workspace.path().join(".apxm").join("sessions"),
        "local-run",
        "2026-04-23T12:00:00Z",
    );
    write_session_manifest(
        &home.path().join(".apxm").join("sessions"),
        "global-run",
        "2026-04-22T12:00:00Z",
    );

    let out = apxm()
        .current_dir(workspace.path())
        .env("HOME", home.path())
        .args(["--json", "session", "list", "--limit", "10"])
        .output()
        .unwrap();
    assert!(out.status.success());

    let sessions: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let sessions = sessions.as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["execution_id"], "local-run");
}

#[test]
fn session_inspect_falls_back_to_global_when_not_found_locally() {
    let workspace = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();

    write_session_manifest(
        &workspace.path().join(".apxm").join("sessions"),
        "local-run",
        "2026-04-23T12:00:00Z",
    );
    let global_dir = write_session_manifest(
        &home.path().join(".apxm").join("sessions"),
        "global-run",
        "2026-04-22T12:00:00Z",
    );

    let out = apxm()
        .current_dir(workspace.path())
        .env("HOME", home.path())
        .args(["--json", "session", "inspect", "global-run"])
        .output()
        .unwrap();
    assert!(out.status.success());

    let session: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(session["manifest"]["execution_id"], "global-run");
    assert_eq!(session["path"], global_dir.display().to_string());
}

#[test]
fn session_list_uses_explicit_session_root() {
    let workspace = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let explicit_root = tempfile::tempdir().unwrap();

    write_session_manifest(
        &workspace.path().join(".apxm").join("sessions"),
        "local-run",
        "2026-04-23T12:00:00Z",
    );
    write_session_manifest(
        &home.path().join(".apxm").join("sessions"),
        "global-run",
        "2026-04-22T12:00:00Z",
    );
    write_session_manifest(explicit_root.path(), "explicit-run", "2026-04-21T12:00:00Z");

    let out = apxm()
        .current_dir(workspace.path())
        .env("HOME", home.path())
        .args([
            "--json",
            "session",
            "list",
            "--limit",
            "10",
            "--session-root",
            explicit_root.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    let sessions: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let sessions = sessions.as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["execution_id"], "explicit-run");
}

#[test]
fn session_clean_uses_explicit_session_root() {
    let workspace = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let explicit_root = tempfile::tempdir().unwrap();

    write_session_manifest(
        &workspace.path().join(".apxm").join("sessions"),
        "local-run",
        "2026-04-23T12:00:00Z",
    );
    write_session_manifest(
        &home.path().join(".apxm").join("sessions"),
        "global-run",
        "2026-04-22T12:00:00Z",
    );
    let explicit_dir =
        write_session_manifest(explicit_root.path(), "explicit-run", "2026-04-01T12:00:00Z");

    let out = apxm()
        .current_dir(workspace.path())
        .env("HOME", home.path())
        .args([
            "session",
            "clean",
            "--all",
            "--session-root",
            explicit_root.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!explicit_dir.exists());
    assert!(
        workspace
            .path()
            .join(".apxm")
            .join("sessions")
            .join("local-run")
            .exists()
    );
    assert!(
        home.path()
            .join(".apxm")
            .join("sessions")
            .join("global-run")
            .exists()
    );
}

#[test]
fn workflow_run_nested_workflow_uses_explicit_root_for_parent_and_child() {
    let temp = tempfile::tempdir().unwrap();
    let workflow_root = temp.path();
    let session_root = workflow_root.join("workflow-sessions");
    let graph_path = workflow_root.join("graph.json");
    let child_workflow_path = workflow_root.join("child.apxmw");
    let parent_workflow_path = workflow_root.join("parent.apxmw");

    write_json_file(
        &graph_path,
        &serde_json::json!({
            "name": "const-graph",
            "nodes": [
                {"id": 1, "name": "value", "op": "CONST_STR", "attributes": {"value": "ok"}},
                {"id": 2, "name": "done", "op": "RETURN", "attributes": {}}
            ],
            "edges": [{"from": 1, "to": 2, "dependency": "Data"}],
            "parameters": [],
            "metadata": {}
        }),
    );
    write_json_file(
        &child_workflow_path,
        &serde_json::json!({
            "name": "child",
            "graphs": [
                {"id": "child_step", "path": "graph.json"}
            ],
            "output": "{{child_step.output}}"
        }),
    );
    write_json_file(
        &parent_workflow_path,
        &serde_json::json!({
            "name": "parent",
            "graphs": [
                {"id": "child_workflow", "path": "child.apxmw"}
            ],
            "output": "{{child_workflow.output}}"
        }),
    );

    let out = apxm()
        .current_dir(workflow_root)
        .env("APXM_MOCK_BACKEND", "1")
        .args([
            "--json",
            "workflow",
            "run",
            parent_workflow_path.to_str().unwrap(),
            "--session-root",
            session_root.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    let parent_session_dir = Path::new(body["session_dir"].as_str().unwrap());
    assert!(parent_session_dir.is_dir());
    assert_eq!(parent_session_dir.parent().unwrap(), session_root);
    assert_eq!(body["output"], "ok");

    let child_session_dir = Path::new(
        body["step_results"]["child_workflow"]["session_dir"]
            .as_str()
            .unwrap(),
    );
    assert!(child_session_dir.is_dir());
    assert_eq!(child_session_dir.parent().unwrap(), parent_session_dir);
}

// ─── doctor ────────────────────────────────────────────────────────────────

#[test]
fn doctor_json_output() {
    let out = apxm().args(["--json", "doctor"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["mlir"].is_object());
    assert!(v["mlir"]["available"].is_boolean());
    assert!(v["backends"].is_object());
    assert!(v["backends"]["count"].is_number());
    assert!(v["environment"].is_object());
}

#[test]
fn doctor_human_readable() {
    let out = apxm().args(["doctor"]).output().unwrap();
    // doctor may exit non-zero when MLIR is missing — that's expected in CI/test
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("MLIR") || stdout.contains("mlir"));
}
