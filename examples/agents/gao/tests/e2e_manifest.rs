// End-to-end fixture for the ultrathin looped `gao` example agent.
//
// Exercised from `crates/tools/cli/src/commands/agent.rs` via
// `cargo test --features driver -p apxm-cli agent:: gao`.
//
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

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

fn read_tools_manifest(root: &Path) -> Vec<serde_json::Value> {
    let path = root.join("capabilities/handlers/tools.json");
    serde_json::from_str(&fs::read_to_string(&path).expect("read tools.json"))
        .expect("parse tools.json")
}

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[test]
fn gao_example_sync_and_lint_pass() {
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_sync(&root, true).expect("gao agent sync");
    agent_lint(&root, None, true).expect("gao agent lint");
}

#[test]
fn gao_declares_runtime_registered_discovery_and_http_builtins() {
    let root = require_gao_example();
    let agent: toml::Value = toml::from_str(
        &fs::read_to_string(root.join("agent.toml")).expect("read gao agent.toml"),
    )
    .expect("parse gao agent.toml");
    let capabilities = agent
        .get("capabilities")
        .and_then(toml::Value::as_array)
        .expect("gao capabilities array")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect::<Vec<_>>();

    for capability in ["capability_discovery", "http_get"] {
        assert!(
            capabilities.contains(&capability),
            "gao must declare runtime-registered builtin {capability}"
        );
        let definition = fs::read_to_string(
            root.join("capabilities")
                .join(capability)
                .join("capability.toml"),
        )
        .expect("read builtin capability definition");
        assert!(
            definition.contains("kind = \"builtin\""),
            "{capability} must use canonical builtin dispatch"
        );
    }
}

#[test]
fn gao_context_uses_typed_host_input_without_direct_networking() {
    let root = require_gao_example();
    let context = fs::read_to_string(root.join("capabilities/handlers/context.ts"))
        .expect("read Gao context handler");

    assert!(
        !context.contains("fetch("),
        "pre-turn hooks must not perform direct network I/O"
    );
    assert!(
        !context.contains("APXM_CAPABILITY_INVENTORY_"),
        "inventory transport must not be hidden in process environment variables"
    );
    assert!(context.contains("snapshot.capabilityInventory"));
    assert!(context.contains("snapshot.nodeKinds"));
    assert!(context.contains("do not emit workflow tool nodes"));
    assert!(
        !root.join("shared/node_kinds.json").exists(),
        "Gao must not carry a hand-maintained Studio node-kind snapshot"
    );
}

#[test]
fn gao_package_is_typescript_only() {
    let root = require_gao_example();
    let mut pending = vec![root.clone()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).expect("read gao package directory") {
            let entry = entry.expect("read gao package entry");
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            assert_ne!(
                path.extension().and_then(|value| value.to_str()),
                Some("py"),
                "gao must not contain Python source: {}",
                path.display()
            );
            if matches!(path.extension().and_then(|value| value.to_str()), Some("toml" | "json")) {
                let text = fs::read_to_string(&path).expect("read Gao manifest");
                assert!(
                    !text.contains("python_handler"),
                    "gao must not declare python_handler entries: {}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn gao_has_no_legacy_python_profile() {
    let legacy_profile = repo_root().join("examples/python/gao");
    assert!(
        !legacy_profile.exists(),
        "Gao has one canonical TypeScript package; remove legacy profile {}",
        legacy_profile.display()
    );
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
    let manifest = read_tools_manifest(&root);
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
        if entry.get("name").and_then(|value| value.as_str()) != Some("hook") {
            let schema = entry
                .get("schema")
                .and_then(serde_json::Value::as_object)
                .expect("every Gao tool must publish an object argument schema");
            assert_eq!(
                schema.get("type").and_then(serde_json::Value::as_str),
                Some("object"),
                "Gao tool schema must describe an argument object: {entry}"
            );
            assert_eq!(
                schema
                    .get("additionalProperties")
                    .and_then(serde_json::Value::as_bool),
                Some(false),
                "Gao tool schemas must reject undeclared arguments: {entry}"
            );
        }
    }

    let plan = manifest
        .iter()
        .find(|entry| entry.get("name").and_then(serde_json::Value::as_str) == Some("plan_workflow"))
        .expect("tools.json must include plan_workflow");
    assert_eq!(
        plan.pointer("/schema/required/0")
            .and_then(serde_json::Value::as_str),
        Some("request"),
        "plan_workflow must require the typed request argument"
    );
}

#[test]
fn gao_plan_workflow_executes_with_object_arguments() {
    if !node_available() || !npm_available() {
        eprintln!("skipping gao_plan_workflow_executes_with_object_arguments: node/npm not on PATH");
        return;
    }
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_build(&root, true).expect("gao agent build");

    let mut plan = read_tools_manifest(&root)
        .into_iter()
        .find(|entry| entry.get("name").and_then(serde_json::Value::as_str) == Some("plan_workflow"))
        .expect("plan_workflow manifest entry");
    let source = plan
        .get("source_file")
        .and_then(serde_json::Value::as_str)
        .expect("plan_workflow source_file");
    let source = source.to_string();
    plan.as_object_mut()
        .expect("plan_workflow manifest object")
        .insert(
            "source_file".to_string(),
            serde_json::Value::String(root.join(&source).to_string_lossy().into_owned()),
        );
    let handler_id = plan
        .get("handler_id")
        .and_then(serde_json::Value::as_str)
        .expect("plan_workflow handler_id")
        .to_string();
    let manifest_path = tmp.path().join("plan-tools.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec(&vec![plan]).expect("serialize plan manifest"),
    )
    .expect("write plan manifest");

    let worker = repo_root().join("crates/compiler/frontend/typescript/scripts/tool-worker.mjs");
    let mut child = Command::new("node")
        .arg(worker)
        .arg(&manifest_path)
        .current_dir(repo_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn TypeScript tool worker");
    let request = "Preserve this request exactly: {typed: true}";
    let frame = serde_json::json!({
        "v": 1,
        "type": "call",
        "req_id": "gao-plan-test",
        "tool_id": handler_id,
        "args": { "request": request },
        "deadline_ms": 5_000,
    });
    writeln!(
        child.stdin.as_mut().expect("tool worker stdin"),
        "{}",
        serde_json::to_string(&frame).expect("serialize tool call")
    )
    .expect("write tool call");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for tool worker");
    assert!(
        output.status.success(),
        "tool worker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response = String::from_utf8(output.stdout).expect("tool worker UTF-8 output");
    let result = response
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|value| value.get("req_id").and_then(serde_json::Value::as_str) == Some("gao-plan-test"))
        .unwrap_or_else(|| panic!("missing plan_workflow result; stderr: {}", String::from_utf8_lossy(&output.stderr)));
    assert_eq!(result.get("ok").and_then(serde_json::Value::as_bool), Some(true));
    let plan_text = result
        .get("value")
        .and_then(serde_json::Value::as_str)
        .expect("plan_workflow returns JSON text");
    let plan_value: serde_json::Value = serde_json::from_str(plan_text).expect("parse plan JSON");
    assert_eq!(
        plan_value.get("request").and_then(serde_json::Value::as_str),
        Some(request),
        "plan_workflow must consume the request object field without stringifying the object"
    );
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
        air.contains("__apxm_typescript_tools__"),
        "declarative gao AIR must carry the TypeScript handler manifest sidecar"
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
