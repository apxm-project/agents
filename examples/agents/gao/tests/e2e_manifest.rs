// End-to-end fixture for the ultrathin looped `gao` example agent.
//
// Exercised from `crates/tools/cli/src/commands/agent.rs` via
// `cargo test --features driver -p apxm-cli agent:: gao`.
//
// Cross-repo server coverage lives in
// `workspace/server/crates/server/src/tests.rs`:
// `agent_compile_endpoint_compiles_real_gao_via_agents_cli`.

use std::fs;
use std::path::Path;

use crate::commands::compile::emit_air_from_agent;
use tempfile::TempDir;

use super::super::{agent_build, agent_lint, agent_sync, gao_example_agent_dir, IntegrityToml};

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn npm_available() -> bool {
    std::process::Command::new("npm")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn require_gao_example() -> std::path::PathBuf {
    let root = gao_example_agent_dir();
    assert!(
        root.join("agent.toml").is_file(),
        "gao example missing at {}",
        root.display()
    );
    root
}

fn copy_gao_example() -> TempDir {
    let tmp = tempfile::tempdir().expect("gao tempdir");
    let root = tmp.path().join("gao");
    copy_dir_all(&require_gao_example(), &root).expect("copy gao fixture");
    tmp
}

#[test]
fn gao_example_sync_and_lint_pass() {
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_sync(&root, true).expect("gao agent sync");
    agent_lint(&root, None, true).expect("gao agent lint");
}

#[test]
fn gao_example_build_seals_integrity() {
    if !node_available() || !npm_available() {
        eprintln!("skipping gao_example_build_seals_integrity: node/npm not on PATH");
        return;
    }
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_build(&root, true).expect("gao agent build");
    let integrity: IntegrityToml = toml::from_str(
        &fs::read_to_string(root.join("integrity.toml")).expect("read integrity.toml"),
    )
    .expect("parse integrity.toml");
    assert!(
        integrity.algorithm == "sha256" && !integrity.chain.is_empty(),
        "build must write generated integrity.toml for gao"
    );
}

#[test]
fn gao_example_build_writes_tools_json_manifest() {
    if !node_available() || !npm_available() {
        eprintln!("skipping gao_example_build_writes_tools_json_manifest: node/npm not on PATH");
        return;
    }
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_build(&root, true).expect("gao agent build");
    let tools_json = root.join("capabilities/handlers/tools.json");
    assert!(
        tools_json.is_file(),
        "typescript agent build must emit capabilities/handlers/tools.json"
    );
    let manifest: Vec<serde_json::Value> =
        serde_json::from_str(&fs::read_to_string(&tools_json).unwrap()).unwrap();
    assert!(
        !manifest.is_empty(),
        "tools.json must list compiled typescript handlers"
    );
    assert!(
        manifest.iter().any(|entry| {
            entry.get("qualname").and_then(|v| v.as_str()) == Some("inject_context")
        }),
        "tools.json must include gao hook handlers"
    );
    for entry in &manifest {
        for key in ["module", "source_file"] {
            let value = entry.get(key).and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                !value.starts_with('/') && !value.contains("/home/"),
                "tools.json {key} must be package-relative, got {value:?}"
            );
        }
    }
}

#[test]
fn gao_compile_service_declarative_emits_recv_loop_air() {
    if !node_available() || !npm_available() {
        eprintln!(
            "skipping gao_compile_service_declarative_emits_recv_loop_air: node/npm not on PATH"
        );
        return;
    }
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_build(&root, true).expect("gao agent build must succeed before compile-service");

    let air = emit_air_from_agent(&root, false)
        .expect("declarative compile-service AIR for gao");

    assert!(air.contains("mode = \"recv\""), "expected in-graph recv loop");
    assert!(
        air.contains("recv_once = \"false\""),
        "expected re-arming recv loop"
    );
    assert!(
        air.contains("turn_param = \"user_message\""),
        "expected gao turn_param in recv attrs"
    );
    assert!(
        air.contains("ais.autonomous"),
        "declarative gao AIR must lower to ais.autonomous recv anchor"
    );
    assert!(
        !air.contains("__apxm_"),
        "declarative gao AIR must not carry handler manifests as comments"
    );
}

#[test]
fn gao_sync_rejects_package_prefixed_capability_id() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("gao-bad-cap");
    fs::create_dir_all(root.join("capabilities/bad_cap")).unwrap();
    copy_dir_all(&gao_example_agent_dir(), &root).unwrap();
    fs::write(
        root.join("capabilities/bad_cap/capability.toml"),
        "id = \"gao.bad_cap\"\ndescription = \"bad\"\n",
    )
    .unwrap();
    fs::write(
        root.join("capabilities/bad_cap/permission.toml"),
        "capability = \"gao.bad_cap\"\ndecision = \"allow\"\n",
    )
    .unwrap();

    let err = agent_sync(&root, true).expect_err("prefixed capability id must fail sync/lint");
    let message = err.to_string();
    assert!(
        message.contains("must not embed the package id prefix")
            || message.contains("must be flat (no '.' segments)")
            || message.contains("ids must match their folder name"),
        "expected package-prefixed capability rejection, got: {message}"
    );
}

#[test]
fn gao_sync_rejects_typescript_handler_without_handler_ts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("gao-missing-handler");
    copy_dir_all(&gao_example_agent_dir(), &root).unwrap();
    fs::create_dir_all(root.join("capabilities/missing_handler")).unwrap();
    fs::write(
        root.join("capabilities/missing_handler/capability.toml"),
        "id = \"missing_handler\"\ndescription = \"bad\"\nkind = \"typescript_handler\"\n",
    )
    .unwrap();
    fs::write(
        root.join("capabilities/missing_handler/permission.toml"),
        "capability = \"missing_handler\"\ndecision = \"allow\"\n",
    )
    .unwrap();

    let err =
        agent_sync(&root, true).expect_err("typescript_handler without handler.ts must fail");
    let message = err.to_string();
    assert!(
        message.contains("typescript_handler") && message.contains("handler.ts"),
        "expected missing handler.ts rejection, got: {message}"
    );
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
