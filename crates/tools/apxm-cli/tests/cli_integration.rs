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
    let binary_dir = binary.parent()?;
    let profile_dirs = if binary_dir.file_name().is_some_and(|name| name == "deps") {
        vec![binary_dir, binary_dir.parent()?]
    } else {
        vec![binary_dir]
    };

    let library_name = compiler_library_name();
    for profile_dir in profile_dirs {
        let installed_lib_dir = profile_dir.join("lib");
        if installed_lib_dir.join(library_name).is_file() {
            return Some(installed_lib_dir);
        }

        let build_dir = profile_dir.join("build");
        if let Ok(entries) = std::fs::read_dir(build_dir) {
            let found = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("out").join("build").join("lib"))
                .find(|candidate| candidate.join(library_name).is_file());
            if found.is_some() {
                return found;
            }
        }
    }

    None
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
    let mut f = tempfile::Builder::new().suffix(".air").tempfile().unwrap();
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

#[test]
fn tokenize_json_counts_text_with_apxm_tokenizer() {
    let out = apxm()
        .args(["--json", "tokenize", "hello world"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "tokenize failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["tokens"], 2);
    assert_eq!(v["tokenizer"], "o200k_base");
    assert_eq!(v["chars"], 11);
    assert_eq!(v["bytes"], 11);
}

#[test]
fn tokenize_json_global_flag_works_after_subcommand_for_dekk() {
    let input = write_tmp_file_named(".txt", "hello world");
    let out = apxm()
        .args([
            "tokenize",
            "--json",
            "--file",
            input.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "tokenize failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["tokens"], 2);
    assert_eq!(v["tokenizer"], "o200k_base");
}

#[cfg(feature = "driver")]
fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(feature = "driver")]
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

#[cfg(feature = "driver")]
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

#[cfg(feature = "driver")]
#[test]
fn compile_python_air_preserves_tool_data_edges() {
    if !python3_available() {
        return;
    }

    let source = r##"print(r'''; __apxm_python_tools__ []
module {
  func.func @tool_data_edge() -> !ais.token attributes {ais.entry} {
    %reg = ais.register_capability "fixture_tool" {description = "fixture tool"} : !ais.token
    %tool = ais.inv_tool "fixture_tool" ("{}") : !ais.token
    %ask = ais.ask "Use {artifact}" [%tool : !ais.token] {input_names = ["artifact"]} : !ais.token
    func.return %ask : !ais.token
  }
}
''')"##;
    let graph = write_tmp_file_named(".py", source);
    let tmp = tempfile::tempdir().unwrap();
    let artifact = tmp.path().join("tool_data_edge.apxmobj");

    let compile = apxm()
        .args([
            "compile",
            graph.path().to_str().unwrap(),
            "--opt-level",
            "0",
            "-o",
            artifact.to_str().unwrap(),
        ])
        .output()
        .expect("apxm compile must run");
    assert!(
        compile.status.success(),
        "compile failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr)
    );

    let decompile = apxm()
        .args(["decompile", artifact.to_str().unwrap()])
        .output()
        .expect("apxm decompile must run");
    assert!(
        decompile.status.success(),
        "decompile failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&decompile.stdout),
        String::from_utf8_lossy(&decompile.stderr)
    );

    let air = String::from_utf8_lossy(&decompile.stdout);
    assert!(air.contains("%n2 = ais.inv_tool"));
    assert!(air.contains("%n3 = ais.ask"));
    assert!(air.contains("[%n2 : !ais.token]"));
    assert!(air.contains("\"input_names\" = [\"artifact\"]"));
}

#[cfg(feature = "driver")]
#[test]
fn compile_saved_python_air_sidecar_preserves_tool_data_edges() {
    let source = r##"; __apxm_python_tools__ []
module {
  func.func @tool_data_edge() -> !ais.token attributes {ais.entry} {
    %reg = ais.register_capability "fixture_tool" {description = "fixture tool"} : !ais.token
    %tool = ais.inv_tool "fixture_tool" ("{}") : !ais.token
    %ask = ais.ask "Use {artifact}" [%tool : !ais.token] {input_names = ["artifact"]} : !ais.token
    func.return %ask : !ais.token
  }
}
"##;
    let graph = write_tmp_file_named(".air", source);
    let tmp = tempfile::tempdir().unwrap();
    let artifact = tmp.path().join("tool_data_edge.apxmobj");

    let compile = apxm()
        .args([
            "compile",
            graph.path().to_str().unwrap(),
            "--opt-level",
            "0",
            "-o",
            artifact.to_str().unwrap(),
        ])
        .output()
        .expect("apxm compile must run");
    assert!(
        compile.status.success(),
        "compile failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr)
    );

    let decompile = apxm()
        .args(["decompile", artifact.to_str().unwrap()])
        .output()
        .expect("apxm decompile must run");
    assert!(
        decompile.status.success(),
        "decompile failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&decompile.stdout),
        String::from_utf8_lossy(&decompile.stderr)
    );

    let air = String::from_utf8_lossy(&decompile.stdout);
    assert!(air.contains("%n2 = ais.inv_tool"));
    assert!(air.contains("%n3 = ais.ask"));
    assert!(air.contains("[%n2 : !ais.token]"));
    assert!(air.contains("\"input_names\" = [\"artifact\"]"));
}

#[cfg(feature = "driver")]
#[test]
fn compile_python_comprehension_preserves_outer_context_edges() {
    if !python3_available() {
        return;
    }

    let source = r##"
from apxm import GraphRecorder, compile, run, tool

@tool
def fixture_tool() -> str:
    return "fixture"

@compile()
def comprehension_flow(g: GraphRecorder):
    artifact = g.invoke_tool(fixture_tool, name="Fixture")
    extracts = [
        g.ask(name=f"Extract_{index}", prompt="Use {artifact}", memoizable=False)
        for index in range(2)
    ]
    g.done(extracts[0])

if __name__ == "__main__":
    run(comprehension_flow())
"##;
    let graph = write_tmp_file_named(".py", source);
    let tmp = tempfile::tempdir().unwrap();
    let artifact = tmp.path().join("comprehension_flow.apxmobj");

    let compile = apxm()
        .args([
            "compile",
            graph.path().to_str().unwrap(),
            "--opt-level",
            "1",
            "--pass-list",
            "normalize",
            "-o",
            artifact.to_str().unwrap(),
        ])
        .output()
        .expect("apxm compile must run");
    assert!(
        compile.status.success(),
        "compile failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr)
    );

    let decompile = apxm()
        .args(["decompile", artifact.to_str().unwrap()])
        .output()
        .expect("apxm decompile must run");
    assert!(
        decompile.status.success(),
        "decompile failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&decompile.stdout),
        String::from_utf8_lossy(&decompile.stderr)
    );

    let air = String::from_utf8_lossy(&decompile.stdout);
    assert!(air.contains("ais.inv_tool"));
    assert_eq!(air.matches("ais.ask").count(), 2);
    assert_eq!(air.matches("\"input_names\" = [\"artifact\"]").count(), 2);
    assert_eq!(air.matches(": !ais.token]").count(), 2);
}

// ─── Valid graphs ───────────────────────────────────────────────────────────

const VALID_ASK: &str = r#"module {
  func.func @test_ask() -> !ais.token attributes {ais.entry} {
    %q = ais.ask "Hello" : !ais.token
    func.return %q : !ais.token
  }
}
"#;

#[cfg(feature = "driver")]
const VALID_PIPELINE: &str = r#"module {
  func.func @test_pipeline() -> !ais.token attributes {ais.entry} {
    %a = ais.ask "step 1" : !ais.token
    %b = ais.ask "step 2: {a}" [%a : !ais.token] {input_names = ["a"]} : !ais.token
    func.return %b : !ais.token
  }
}
"#;

#[cfg(feature = "driver")]
const VALID_PARALLEL: &str = r#"module {
  func.func @test_parallel() -> !ais.token attributes {ais.entry} {
    %a = ais.ask "task a" : !ais.token
    %b = ais.ask "task b" : !ais.token
    %sync = ais.wait_all %a, %b : !ais.token, !ais.token -> !ais.token
    func.return %sync : !ais.token
  }
}
"#;

#[cfg(feature = "driver")]
const CONST_GRAPH: &str = r#"module {
  func.func @const_graph() -> !ais.token attributes {ais.entry} {
    %value = ais.const_str "ok" : !ais.token
    func.return %value : !ais.token
  }
}
"#;

// ─── validate: valid graphs ─────────────────────────────────────────────────

#[cfg(feature = "driver")]
#[test]
fn validate_valid_ask_air() {
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

#[cfg(feature = "driver")]
#[test]
fn validate_valid_pipeline_air() {
    let f = write_tmp_graph(VALID_PIPELINE);
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], true);
}

#[cfg(feature = "driver")]
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

#[cfg(feature = "driver")]
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

#[cfg(feature = "driver")]
#[test]
fn execute_air_with_local_controls_succeeds_end_to_end() {
    let graph = write_tmp_file_named(".air", CONST_GRAPH);
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
fn validate_rejects_graph_json_source() {
    let f = write_tmp_file_named(".json", VALID_ASK);
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
            .any(|e| e.as_str().unwrap().contains("canonical .air"))
    );
}

#[test]
fn validate_file_not_found() {
    let out = apxm()
        .args(["validate", "/nonexistent/path.air"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn validate_invalid_air() {
    let f = write_tmp_graph("not valid AIR");
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], false);
    assert!(!v["errors"].as_array().unwrap().is_empty());
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

#[cfg(feature = "driver")]
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
    assert_eq!(v["depth"], 3);
    assert_eq!(v["node_count"], 4);
    assert_eq!(v["edge_count"], 3);
}

#[cfg(feature = "driver")]
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
    assert_eq!(v["depth"], 2);
    assert_eq!(v["speedup"]["estimated_speedup"], "1.00x");
}

#[cfg(feature = "driver")]
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
    assert_eq!(v["depth"], 3);
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
    let names: Vec<&str> = templates
        .iter()
        .filter_map(|template| template["name"].as_str())
        .collect();
    for expected in ["ask", "pipeline", "fan-out", "map-reduce", "verify"] {
        assert!(names.contains(&expected));
    }
    for t in templates {
        assert!(t["name"].is_string());
        assert!(t["description"].is_string());
    }
}

#[test]
fn template_show_ask_air() {
    let out = apxm().args(["template", "show", "ask"]).output().unwrap();
    assert!(out.status.success());
    let air = String::from_utf8_lossy(&out.stdout);
    assert!(air.contains("module {"));
    assert!(air.contains("func.func @simple_ask"));
}

#[test]
fn template_show_unknown() {
    let out = apxm()
        .args(["template", "show", "nonexistent"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[cfg(feature = "driver")]
#[test]
fn template_roundtrip_validate() {
    // Get canonical template AIR and feed it to validate.
    let show_out = apxm()
        .args(["--json", "template", "show", "map-reduce"])
        .output()
        .unwrap();
    assert!(show_out.status.success());
    let template: serde_json::Value = serde_json::from_slice(&show_out.stdout).unwrap();
    let air = template["air"].as_str().unwrap();

    let f = write_tmp_graph(air);
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
    let f = write_tmp_graph("module { }");
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
        "module { func.func @broken() -> !ais.token { func.return %missing : !ais.token } }",
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(!v["errors"].as_array().unwrap().is_empty());
}

#[test]
fn validate_duplicate_parameter_name() {
    let f = write_tmp_graph(
        "module { func.func @dup(%arg0: !ais.token {ais.param_name = \"p\", ais.param_type = \"str\"}, %arg1: !ais.token {ais.param_name = \"p\", ais.param_type = \"int\"}) -> !ais.token attributes {ais.entry} { func.return %arg0 : !ais.token } }",
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], false);
}

#[cfg(feature = "driver")]
#[test]
fn validate_disconnected_graph() {
    let f = write_tmp_graph(
        r#"module {
  func.func @disconnected() -> !ais.token attributes {ais.entry} {
    %a = ais.ask "x" : !ais.token
    %b = ais.ask "y" : !ais.token
    %merged = ais.merge %a, %b : !ais.token, !ais.token -> !ais.token
    func.return %merged : !ais.token
  }
}
"#,
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], true);
}

#[cfg(feature = "driver")]
#[test]
fn validate_parameter_invalid_type() {
    let f = write_tmp_graph(
        "module { func.func @param_type(%arg0: !ais.token {ais.param_name = \"p\", ais.param_type = \"invalid_type\"}) -> !ais.token attributes {ais.entry} { func.return %arg0 : !ais.token } }",
    );
    let out = apxm()
        .args(["--json", "validate", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], true);
}

// ─── analyze: edge cases ────────────────────────────────────────────────────

#[cfg(feature = "driver")]
#[test]
fn analyze_disconnected_components() {
    let f = write_tmp_graph(
        r#"module {
  func.func @parallel_components() -> !ais.token attributes {ais.entry} {
    %a = ais.ask "x" : !ais.token
    %b = ais.ask "y" : !ais.token
    %merged = ais.merge %a, %b : !ais.token, !ais.token -> !ais.token
    func.return %merged : !ais.token
  }
}
"#,
    );
    let out = apxm()
        .args(["--json", "analyze", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["max_parallelism"], 2);
    assert_eq!(v["depth"], 3);
}

#[cfg(feature = "driver")]
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

#[cfg(feature = "driver")]
#[test]
fn explain_single_node_json() {
    let f = write_tmp_graph(VALID_ASK);
    let out = apxm()
        .args(["--json", "explain", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["graph_name"], "test_ask");
    assert_eq!(v["node_count"], 2);
    assert_eq!(v["edge_count"], 1);
    assert_eq!(v["depth"], 2);
    let flow = v["execution_flow"].as_array().unwrap();
    assert_eq!(flow.len(), 2);
    assert_eq!(flow[0]["phase"], 1);
    let nodes = flow[0]["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["op"], "ASK");
    assert_eq!(flow[1]["nodes"][0]["op"], "RETURN");
}

#[cfg(feature = "driver")]
#[test]
fn explain_pipeline_json() {
    let f = write_tmp_graph(VALID_PIPELINE);
    let out = apxm()
        .args(["--json", "explain", f.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["depth"], 3);
    let flow = v["execution_flow"].as_array().unwrap();
    assert_eq!(flow.len(), 3);
    assert_eq!(flow[0]["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(flow[1]["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(flow[2]["nodes"][0]["op"], "RETURN");
}

#[cfg(feature = "driver")]
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
    assert!(flow[0]["parallel"].as_bool().unwrap());
    assert_eq!(flow[0]["nodes"].as_array().unwrap().len(), 2);
    assert!(v["summary"]["max_parallelism"].as_u64().unwrap() >= 2);
}

#[cfg(feature = "driver")]
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
        .args(["explain", "/nonexistent/file.air"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[cfg(feature = "driver")]
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
    assert_eq!(first_json["check"], false);

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

    let check = apxm()
        .args([
            "--json",
            "codegen",
            "frontend",
            "--output-dir",
            output_dir_str,
            "--check",
        ])
        .output()
        .unwrap();
    assert!(check.status.success());
    let check_json: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(check_json["target"], "frontend");
    assert_eq!(check_json["output_dir"], output_dir_str);
    assert_eq!(check_json["check"], true);
}

#[test]
fn codegen_typescript_is_idempotent_and_checkable() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output_path = temp_dir.path().join("generated.ts");
    let output_path_str = output_path.to_str().unwrap();

    let first = apxm()
        .args([
            "--json",
            "codegen",
            "typescript",
            "--output",
            output_path_str,
        ])
        .output()
        .unwrap();
    assert!(first.status.success());
    let first_json: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first_json["target"], "typescript");
    assert_eq!(first_json["output"], output_path_str);
    assert_eq!(first_json["check"], false);
    let first_content = std::fs::read_to_string(&output_path).unwrap();

    let second = apxm()
        .args([
            "--json",
            "codegen",
            "typescript",
            "--output",
            output_path_str,
        ])
        .output()
        .unwrap();
    assert!(second.status.success());
    let second_json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    let second_content = std::fs::read_to_string(&output_path).unwrap();
    assert_eq!(first_json, second_json);
    assert_eq!(first_content, second_content);

    let check = apxm()
        .args([
            "--json",
            "codegen",
            "typescript",
            "--output",
            output_path_str,
            "--check",
        ])
        .output()
        .unwrap();
    assert!(check.status.success());
    let check_json: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(check_json["target"], "typescript");
    assert_eq!(check_json["output"], output_path_str);
    assert_eq!(check_json["check"], true);
}

#[test]
fn codegen_event_kinds_is_idempotent_and_checkable() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output_path = temp_dir.path().join("core-event-kinds.ts");
    let output_path_str = output_path.to_str().unwrap();

    let first = apxm()
        .args([
            "--json",
            "codegen",
            "event-kinds",
            "--output",
            output_path_str,
        ])
        .output()
        .unwrap();
    assert!(first.status.success());
    let first_json: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first_json["target"], "event-kinds");
    assert_eq!(first_json["output"], output_path_str);
    assert_eq!(first_json["check"], false);
    assert!(first_json["count"].as_u64().unwrap() > 0);
    let first_content = std::fs::read_to_string(&output_path).unwrap();

    let second = apxm()
        .args([
            "--json",
            "codegen",
            "event-kinds",
            "--output",
            output_path_str,
        ])
        .output()
        .unwrap();
    assert!(second.status.success());
    let second_json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    let second_content = std::fs::read_to_string(&output_path).unwrap();
    assert_eq!(first_json, second_json);
    assert_eq!(first_content, second_content);

    let check = apxm()
        .args([
            "--json",
            "codegen",
            "event-kinds",
            "--output",
            output_path_str,
            "--check",
        ])
        .output()
        .unwrap();
    assert!(check.status.success());
    let check_json: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(check_json["target"], "event-kinds");
    assert_eq!(check_json["output"], output_path_str);
    assert_eq!(check_json["check"], true);
    assert_eq!(check_json["count"], first_json["count"]);
}

#[test]
fn codegen_typescript_check_fails_when_file_is_stale() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output_path = temp_dir.path().join("generated.ts");
    let output_path_str = output_path.to_str().unwrap();
    let stale_content = "// stale generated file\n";
    std::fs::write(&output_path, stale_content).unwrap();

    let check = apxm()
        .args([
            "--json",
            "codegen",
            "typescript",
            "--output",
            output_path_str,
            "--check",
        ])
        .output()
        .unwrap();

    assert!(!check.status.success());
    let check_json: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert!(check_json["error"].as_str().unwrap().contains("stale"));
    assert_eq!(
        std::fs::read_to_string(&output_path).unwrap(),
        stale_content
    );
}

#[test]
fn codegen_frontend_check_fails_when_file_is_stale() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output_dir = temp_dir.path().join("_generated");
    let output_dir_str = output_dir.to_str().unwrap();

    let generate = apxm()
        .args([
            "--json",
            "codegen",
            "frontend",
            "--output-dir",
            output_dir_str,
        ])
        .output()
        .unwrap();
    assert!(generate.status.success());

    let constants_path = output_dir.join("constants.py");
    let stale_content = "# stale generated frontend file\n";
    std::fs::write(&constants_path, stale_content).unwrap();

    let check = apxm()
        .args([
            "--json",
            "codegen",
            "frontend",
            "--output-dir",
            output_dir_str,
            "--check",
        ])
        .output()
        .unwrap();

    assert!(!check.status.success());
    let check_json: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert!(check_json["error"].as_str().unwrap().contains("stale"));
    assert_eq!(
        std::fs::read_to_string(&constants_path).unwrap(),
        stale_content
    );
}

#[test]
fn codegen_event_kinds_check_fails_when_file_is_stale() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output_path = temp_dir.path().join("core-event-kinds.ts");
    let output_path_str = output_path.to_str().unwrap();
    let stale_content = "// stale generated event kinds\n";
    std::fs::write(&output_path, stale_content).unwrap();

    let check = apxm()
        .args([
            "--json",
            "codegen",
            "event-kinds",
            "--output",
            output_path_str,
            "--check",
        ])
        .output()
        .unwrap();

    assert!(!check.status.success());
    let check_json: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert!(check_json["error"].as_str().unwrap().contains("stale"));
    assert_eq!(
        std::fs::read_to_string(&output_path).unwrap(),
        stale_content
    );
}

#[test]
fn codegen_default_generated_files_are_current() {
    let frontend = apxm()
        .args(["--json", "codegen", "frontend", "--check"])
        .output()
        .unwrap();
    assert!(
        frontend.status.success(),
        "frontend codegen drift: {}",
        String::from_utf8_lossy(&frontend.stderr)
    );

    let typescript = apxm()
        .args(["--json", "codegen", "typescript", "--check"])
        .output()
        .unwrap();
    assert!(
        typescript.status.success(),
        "typescript codegen drift: {}",
        String::from_utf8_lossy(&typescript.stderr)
    );

    let event_kinds = apxm()
        .args(["--json", "codegen", "event-kinds", "--check"])
        .output()
        .unwrap();
    assert!(
        event_kinds.status.success(),
        "event-kind codegen drift: {}",
        String::from_utf8_lossy(&event_kinds.stderr)
    );
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

#[cfg(feature = "driver")]
#[test]
fn workflow_run_nested_workflow_uses_explicit_root_for_parent_and_child() {
    let temp = tempfile::tempdir().unwrap();
    let workflow_root = temp.path();
    let session_root = workflow_root.join("workflow-sessions");
    let graph_path = workflow_root.join("graph.air");
    let child_workflow_path = workflow_root.join("child.apxmw");
    let parent_workflow_path = workflow_root.join("parent.apxmw");

    std::fs::write(&graph_path, CONST_GRAPH).unwrap();
    write_json_file(
        &child_workflow_path,
        &serde_json::json!({
            "name": "child",
            "graphs": [
                {"id": "child_step", "path": "graph.air"}
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
