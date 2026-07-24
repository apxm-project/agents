// Packaging conformance fixture for the ultrathin looped `gao` example agent.
//
// Exercised from `crates/tools/cli/src/commands/agent.rs` via
// `cargo test --features driver -p apxm-cli agent:: gao`.
//
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

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
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("read tools.json"))
            .expect("parse tools.json");
    value
        .get("handlers")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_else(|| value.as_array().cloned().expect("tools.json handlers array"))
}

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn worker_manifest_entry(root: &Path, selector: &str) -> serde_json::Value {
    read_tools_manifest(root)
        .into_iter()
        .find(|entry| {
            entry.get("name").and_then(serde_json::Value::as_str) == Some(selector)
                || entry.get("qualname").and_then(serde_json::Value::as_str) == Some(selector)
        })
        .unwrap_or_else(|| panic!("missing Gao worker entry {selector}"))
}

fn run_typescript_packaging_worker(
    tmp: &TempDir,
    entries: Vec<serde_json::Value>,
    frames: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let manifest_path = tmp.path().join("focused-tools.json");
    let manifest = serde_json::json!({
        "version": "apxm.handler-manifest.v1",
        "handlers": entries,
    });
    fs::write(
        &manifest_path,
        serde_json::to_vec(&manifest).expect("serialize focused worker manifest"),
    )
    .expect("write focused worker manifest");

    let worker = repo_root().join("crates/tools/cli/agent-packaging/tool-worker.mjs");
    let mut child = Command::new("node")
        .arg(worker)
        .arg(&manifest_path)
        .current_dir(repo_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn TypeScript tool worker");
    for frame in frames {
        writeln!(
            child.stdin.as_mut().expect("tool worker stdin"),
            "{}",
            serde_json::to_string(frame).expect("serialize tool worker call")
        )
        .expect("write tool worker call");
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for tool worker");
    assert!(
        output.status.success(),
        "tool worker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("tool worker UTF-8 output")
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value.get("type").and_then(serde_json::Value::as_str) == Some("result"))
        .collect()
}

fn worker_result<'a>(results: &'a [serde_json::Value], req_id: &str) -> &'a serde_json::Value {
    results
        .iter()
        .find(|value| value.get("req_id").and_then(serde_json::Value::as_str) == Some(req_id))
        .unwrap_or_else(|| panic!("missing TypeScript worker result for {req_id}"))
}

fn worker_value<'a>(results: &'a [serde_json::Value], req_id: &str) -> &'a serde_json::Value {
    let result = worker_result(results, req_id);
    assert_eq!(
        result.get("ok").and_then(serde_json::Value::as_bool),
        Some(true),
        "TypeScript worker returned error for {req_id}: {result}"
    );
    result
        .get("value")
        .unwrap_or_else(|| panic!("TypeScript worker result for {req_id} has no value: {result}"))
}

#[test]
fn gao_example_sync_and_lint_pass() {
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_sync(&root, true).expect("gao agent sync");
    agent_lint(&root, None, true).expect("gao agent lint");
}

#[test]
fn gao_declares_a_small_apxm_authoring_capability_set() {
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

    assert_eq!(
        capabilities,
        vec![
            "capability_discovery",
            "plan_workflow",
            "prepare_validation",
        ],
        "Gao keeps only its three APXM authoring capabilities",
    );

    for capability in &capabilities {
        assert!(
            root.join("capabilities")
                .join(capability)
                .join("capability.toml")
                .is_file(),
            "Gao must define its declared capability {capability}"
        );
    }

    let discovery = fs::read_to_string(root.join("capabilities/capability_discovery/capability.toml"))
        .expect("read capability discovery definition");
    assert!(
        discovery.contains("kind = \"builtin\""),
        "capability discovery is provided by the APXM host"
    );
    for capability in ["plan_workflow", "prepare_validation"] {
        let definition = fs::read_to_string(
            root.join("capabilities")
                .join(capability)
                .join("capability.toml"),
        )
        .expect("read Gao capability definition");
        assert!(
            definition.contains("kind = \"typescript_handler\""),
            "{capability} is implemented by the focused Gao extension"
        );
    }
}

#[test]
fn gao_uses_generic_host_input_without_package_context_injection() {
    let root = require_gao_example();
    let entry = fs::read_to_string(root.join("src/gao.ts")).expect("read Gao entry source");

    assert!(
        !root.join("shared/node_kinds.json").exists(),
        "Gao must not carry a hand-maintained Studio node-kind snapshot"
    );
    assert!(
        entry.contains("Agent<"),
        "Gao must use the installed generic Agent authoring API"
    );
    assert!(
        entry.contains("DiscoverCapabilities")
            && entry.contains("PlanWorkflow")
            && entry.contains("PrepareValidation")
            && entry.contains("agent.yield_"),
        "Gao must use its three APXM authoring capabilities and yield the next input"
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
                if matches!(
                    entry.file_name().to_str(),
                    Some("node_modules" | "dist")
                ) {
                    continue;
                }
                pending.push(path);
                continue;
            }
            assert_ne!(
                path.extension().and_then(|value| value.to_str()),
                Some("py"),
                "gao must not contain Python source: {}",
                path.display()
            );
            if path.extension().and_then(|value| value.to_str()) == Some("toml") {
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
    let agent: toml::Value = toml::from_str(
        &fs::read_to_string(root.join("agent.toml")).expect("read gao agent.toml"),
    )
    .expect("parse gao agent.toml");
    assert!(
        agent.get("hooks").is_none(),
        "Gao behavior Hooks are authored in Agent Program source"
    );
    assert!(
        manifest
            .iter()
            .all(|entry| entry.get("kind").and_then(serde_json::Value::as_str) == Some("tool")),
        "tools.json contains only package-local Capability handlers"
    );
    for entry in &manifest {
        for key in ["module"] {
            let value = entry.get(key).and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                !value.starts_with('/') && !value.contains("/home/"),
                "tools.json {key} must be package-relative, got {value:?}"
            );
        }
        let artifact_path = entry
            .get("source")
            .and_then(serde_json::Value::as_object)
            .and_then(|source| source.get("artifact_path"))
            .and_then(serde_json::Value::as_str)
            .expect("handler manifest source artifact path");
        assert!(
            !artifact_path.starts_with('/') && !artifact_path.contains("/home/"),
            "tools.json source artifact path must be package-relative, got {artifact_path:?}"
        );
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
fn gao_semantic_tools_derive_typed_outputs_from_inputs() {
    if !node_available() || !npm_available() {
        eprintln!(
            "skipping gao_semantic_tools_derive_typed_outputs_from_inputs: node/npm not on PATH"
        );
        return;
    }
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_build(&root, true).expect("gao agent build");
    let plan = worker_manifest_entry(&root, "plan_workflow");
    let validation = worker_manifest_entry(&root, "prepare_validation");
    let plan_id = plan
        .get("handler_id")
        .and_then(serde_json::Value::as_str)
        .unwrap();
    let validation_id = validation
        .get("handler_id")
        .and_then(serde_json::Value::as_str)
        .unwrap();
    let request = "Draft an APXM workflow that validates a source-first agent.";
    let catalog = "capability_discovery, plan_workflow, prepare_validation";
    let frames = vec![
        serde_json::json!({
            "v": 1, "type": "call", "req_id": "gao-plan-test", "tool_id": plan_id,
            "args": {"request": request, "catalog": catalog}, "deadline_ms": 5_000
        }),
        serde_json::json!({
            "v": 1, "type": "call", "req_id": "gao-validation-test", "tool_id": validation_id,
            "args": {
                "request": request,
                "plan": "Review the source-first agent workflow."
            },
            "deadline_ms": 5_000
        }),
    ];
    let results = run_typescript_packaging_worker(&tmp, vec![plan, validation], &frames);
    let plan_value = worker_value(&results, "gao-plan-test");
    let plan_text = plan_value
        .get("plan")
        .and_then(serde_json::Value::as_str)
        .expect("plan_workflow returns a typed plan answer");
    assert!(plan_text.contains(request));
    assert!(plan_text.contains(catalog));
    assert_eq!(
        plan_value.get("next_action").and_then(serde_json::Value::as_str),
        Some("review_plan")
    );

    let validation_value = worker_value(&results, "gao-validation-test");
    let validation_text = validation_value
        .get("validation_request")
        .and_then(serde_json::Value::as_str)
        .expect("prepare_validation returns a typed validation answer");
    assert!(validation_text.contains(request));
    assert!(validation_text.contains("Review the source-first agent workflow."));
    assert_eq!(
        validation_value
            .get("next_action")
            .and_then(serde_json::Value::as_str),
        Some("submit_for_validation")
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
        if matches!(
            entry.file_name().to_str(),
            Some("node_modules" | "dist" | "__pycache__" | "artifacts")
        ) || file_type.is_symlink()
        {
            continue;
        }
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
