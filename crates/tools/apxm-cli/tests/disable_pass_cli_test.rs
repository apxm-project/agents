//! Smoke-test the --disable-pass / --pass-list CLI plumbing for ablation studies.
//!
//! Compiles a small inline AIR graph at O1 with a named pass dropped, then reads
//! the `--emit-diagnostics` JSON and confirms the disabled pass is absent from the
//! `pass_metrics` array. A second case exercises `--pass-list` to confirm the
//! override replaces the default selection wholesale.
//!
//! Note: as of Task 8 the only emitted-metrics path on the compile subcommand is
//! `--emit-diagnostics`; Task 9 may introduce a richer `--emit-metrics` artifact,
//! at which point the `metrics.json` schema described in the parent plan can be
//! asserted directly.
//!
//! Gated on the `driver` feature because the `compile` subcommand is itself
//! gated there. `dekk apxm test-cli` builds with `--features driver,metrics`
//! and exercises this file; the default `test-all` (no driver) skips it.

#![cfg(feature = "driver")]

use std::io::Write;
use std::process::Command;

use apxm_core::types::compiler::metadata as passes;

fn apxm() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_apxm"));
    cmd.env("NO_COLOR", "1");
    cmd
}

/// Two-ASK pipeline. The graph compiles cleanly at O1 and lets the test inspect
/// which pass names appear in diagnostics.
const SMALL_PIPELINE: &str = r#"module {
  func.func @disable_pass_test() -> !ais.token attributes {ais.entry} {
    %a = ais.ask "step 1" : !ais.token
    %b = ais.ask "step 2" [%a : !ais.token] : !ais.token
    func.return %b : !ais.token
  }
}
"#;

/// The explicit --pass-list case intentionally omits prompt-building passes,
/// so use a local graph that does not need the prompt/input-name contract.
const LOCAL_CONST_GRAPH: &str = r#"module {
  func.func @disable_pass_test() -> !ais.token attributes {ais.entry} {
    %value = ais.const_str "ok" : !ais.token
    func.return %value : !ais.token
  }
}
"#;

fn write_tmp_graph(content: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::Builder::new().suffix(".air").tempfile().unwrap();
    f.write_all(content.as_bytes()).unwrap();
    f.flush().unwrap();
    f
}

fn pass_names_from_diagnostics(diag_path: &std::path::Path) -> Vec<String> {
    let raw = std::fs::read_to_string(diag_path).expect("diagnostics file written");
    let json: serde_json::Value = serde_json::from_str(&raw).expect("diagnostics parses as JSON");
    json["pass_metrics"]
        .as_array()
        .expect("pass_metrics array present")
        .iter()
        .map(|p| {
            p["pass_name"]
                .as_str()
                .expect("pass_name string")
                .to_string()
        })
        .collect()
}

#[test]
fn disable_pass_removes_pass_from_diagnostics() {
    let graph = write_tmp_graph(SMALL_PIPELINE);
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out.apxmobj");
    let diag = tmp.path().join("diag.json");

    let status = apxm()
        .args([
            "compile",
            graph.path().to_str().unwrap(),
            "--opt-level",
            "1",
            "--disable-pass",
            passes::ASSIGN_PRIORITY.name,
            "-o",
            out.to_str().unwrap(),
            "--emit-diagnostics",
            diag.to_str().unwrap(),
        ])
        .status()
        .expect("apxm compile must run");
    assert!(status.success(), "apxm compile failed");

    let names = pass_names_from_diagnostics(&diag);
    assert!(
        !names.iter().any(|n| n == passes::ASSIGN_PRIORITY.name),
        "{} must be absent after --disable-pass; got {names:?}",
        passes::ASSIGN_PRIORITY.name
    );
    // Sanity: at least one O1 pass still present (override didn't accidentally clear).
    assert!(
        names.iter().any(|n| n == passes::NORMALIZE.name),
        "normalize must still appear at O1; got {names:?}"
    );
}

#[test]
fn pass_list_override_replaces_default_selection() {
    let graph = write_tmp_graph(LOCAL_CONST_GRAPH);
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out.apxmobj");
    let diag = tmp.path().join("diag.json");

    // Minimal viable list: normalize + canonicalizer. Notably omits the rest of
    // the O1 default pipeline.
    let pass_list = format!("{},{}", passes::NORMALIZE.name, passes::CANONICALIZER.name);
    let status = apxm()
        .args([
            "compile",
            graph.path().to_str().unwrap(),
            "--opt-level",
            "1",
            "--pass-list",
            pass_list.as_str(),
            "-o",
            out.to_str().unwrap(),
            "--emit-diagnostics",
            diag.to_str().unwrap(),
        ])
        .status()
        .expect("apxm compile must run");
    assert!(status.success(), "apxm compile failed");

    let names = pass_names_from_diagnostics(&diag);
    assert_eq!(
        names,
        vec![
            passes::NORMALIZE.name.to_string(),
            passes::CANONICALIZER.name.to_string()
        ],
        "--pass-list must replace the default selection wholesale; got {names:?}"
    );
}
