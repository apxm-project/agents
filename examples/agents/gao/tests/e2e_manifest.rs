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
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("read tools.json"))
            .expect("parse tools.json");
    value
        .get("handlers")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_else(|| value.as_array().cloned().expect("tools.json handlers array"))
}

fn read_compile_service_manifest(root: &Path) -> Vec<serde_json::Value> {
    let air = emit_air_from_agent(root)
        .expect("compile-service AIR");
    let sidecar = air
        .lines()
        .find_map(|line| line.strip_prefix("; __apxm_handler_manifest__ "))
        .expect("compile-service handler sidecar");
    let value: serde_json::Value = serde_json::from_str(sidecar).expect("parse handler sidecar");
    value
        .get("handlers")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .expect("handler sidecar array")
}

fn compile_source_manifest(root: &Path, source_file: &str) -> Vec<serde_json::Value> {
    let compiler = repo_root().join("crates/compiler/frontend/typescript/dist/compile-handlers.js");
    assert!(
        compiler.is_file(),
        "TypeScript frontend compile-handlers entry missing at {}",
        compiler.display()
    );
    let source = root.join(source_file);
    let output = Command::new("node")
        .arg("--input-type=module")
        .arg("--eval")
        .arg(
            "import { pathToFileURL } from 'node:url'; import(pathToFileURL(process.argv[1]).href).then(async ({ compileHandlers }) => { const manifest = await compileHandlers([process.argv[2]], { rootDir: process.argv[3] }); console.log(JSON.stringify(manifest)); })",
        )
        .arg(&compiler)
        .arg(&source)
        .arg(root)
        .output()
        .expect("run compile-handlers");
    assert!(
        output.status.success(),
        "compile-handlers failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse compile-handlers output");
    value
        .get("handlers")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .expect("compiled handler array")
}

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn worker_manifest_entry(root: &Path, selector: &str) -> serde_json::Value {
    let sidecar_entry = read_compile_service_manifest(root)
        .into_iter()
        .find(|entry| {
            entry.get("name").and_then(serde_json::Value::as_str) == Some(selector)
                || entry.get("qualname").and_then(serde_json::Value::as_str) == Some(selector)
        });
    if let Some(entry) = sidecar_entry {
        return entry;
    }

    let manifest_entry = read_tools_manifest(root)
        .into_iter()
        .find(|entry| {
            entry.get("name").and_then(serde_json::Value::as_str) == Some(selector)
                || entry.get("qualname").and_then(serde_json::Value::as_str) == Some(selector)
        })
        .unwrap_or_else(|| panic!("missing Gao worker entry {selector}"));
    if manifest_entry.get("source").is_some() {
        return manifest_entry;
    }
    let source_file = manifest_entry
        .get("source_file")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("Gao worker entry {selector} has no source_file"));
    let mut entry = compile_source_manifest(root, source_file)
        .into_iter()
        .find(|entry| {
            entry.get("name").and_then(serde_json::Value::as_str) == Some(selector)
                || entry.get("qualname").and_then(serde_json::Value::as_str) == Some(selector)
        })
        .unwrap_or_else(|| panic!("compiled Gao worker entry {selector} missing"));
    if let Some(source_file) = entry
        .get("source_file")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
    {
        entry
            .as_object_mut()
            .expect("worker manifest object")
            .insert(
                "source_file".to_string(),
                serde_json::Value::String(root.join(source_file).to_string_lossy().into_owned()),
            );
    }
    entry
}

fn run_typescript_worker(
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
fn gao_local_skill_capabilities_use_host_registered_builtins() {
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_sync(&root, true).expect("gao agent sync");
    agent_lint(&root, None, true).expect("gao agent lint");

    for capability in ["list_local_skills", "search_skills", "read_local_skill"] {
        let definition_path = root
            .join("capabilities")
            .join(capability)
            .join("capability.toml");
        let definition: toml::Value = toml::from_str(
            &fs::read_to_string(&definition_path).expect("read local-skill capability definition"),
        )
        .expect("parse local-skill capability definition");
        assert_eq!(
            definition.get("kind").and_then(toml::Value::as_str),
            Some("builtin")
        );
        assert!(
            !root
                .join("capabilities")
                .join(capability)
                .join("handler.ts")
                .exists(),
            "{capability} must not carry a package-local catalogue reader"
        );
    }

    let tool_names = read_tools_manifest(&root)
        .into_iter()
        .filter(|entry| entry.get("kind").and_then(serde_json::Value::as_str) == Some("tool"))
        .filter_map(|entry| {
            entry
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect::<std::collections::BTreeSet<_>>();
    for capability in ["list_local_skills", "search_skills", "read_local_skill"] {
        assert!(
            !tool_names.contains(capability),
            "{capability} must resolve through the host capability registry, not tools.json"
        );
    }
}

#[test]
fn gao_uses_generic_host_input_without_package_context_injection() {
    let root = require_gao_example();
    let context = fs::read_to_string(root.join("capabilities/handlers/context.ts"))
        .expect("read Gao context handler");
    let entry = fs::read_to_string(root.join("capabilities/handlers/main.ts"))
        .expect("read Gao entry source");

    assert!(
        !context.contains("fetch("),
        "package-local path helpers must not perform direct network I/O"
    );
    assert!(
        !context.contains("APXM_CAPABILITY_INVENTORY_"),
        "host context must not be read through process environment variables"
    );
    assert!(
        !root.join("shared/node_kinds.json").exists(),
        "Gao must not carry a hand-maintained Studio node-kind snapshot"
    );
    assert!(
        entry.contains("graph.awaitInput"),
        "Gao must receive turns through the generic AWAIT_INPUT primitive"
    );
    assert!(
        entry.contains("rearm: true"),
        "Gao must explicitly re-arm its generic host-input boundary"
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
    let declared_hooks = agent
        .get("hooks")
        .and_then(toml::Value::as_array)
        .expect("Gao agent manifest must declare hooks");
    let hook_entries = manifest
        .iter()
        .filter(|entry| entry.get("kind").and_then(serde_json::Value::as_str) == Some("hook"))
        .collect::<Vec<_>>();
    assert!(!hook_entries.is_empty(), "tools.json must include Gao hook handlers");
    for hook in declared_hooks {
        let handler = hook
            .get("handler")
            .and_then(toml::Value::as_str)
            .expect("declared hook handler");
        let qualname = handler.rsplit('.').next().expect("hook handler qualname");
        let event = hook
            .get("event")
            .and_then(toml::Value::as_str)
            .expect("declared hook event");
        let match_value = hook
            .get("match")
            .and_then(toml::Value::as_str)
            .unwrap_or("*");
        let mode = hook
            .get("mode")
            .and_then(toml::Value::as_str)
            .expect("declared hook mode");
        assert!(
            hook_entries.iter().any(|entry| {
                entry.get("qualname").and_then(serde_json::Value::as_str) == Some(qualname)
                    && entry.get("event").and_then(serde_json::Value::as_str) == Some(event)
                    && entry.get("match").and_then(serde_json::Value::as_str) == Some(match_value)
                    && entry.get("mode").and_then(serde_json::Value::as_str) == Some(mode)
            }),
            "tools.json must contain the declared hook {handler}"
        );
    }
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
    let permission = worker_manifest_entry(&root, "explain_permission");
    let plan_id = plan
        .get("handler_id")
        .and_then(serde_json::Value::as_str)
        .unwrap();
    let validation_id = validation
        .get("handler_id")
        .and_then(serde_json::Value::as_str)
        .unwrap();
    let permission_id = permission
        .get("handler_id")
        .and_then(serde_json::Value::as_str)
        .unwrap();
    let request =
        "When a request arrives. Fetch it with fetch_record, then publish it with publish_report.";
    let catalog = serde_json::json!([
        {"id": "fetch_record", "description": "Fetch a record", "read_only": true},
        {"id": "publish_report", "description": "Publish a report", "read_only": false}
    ]);
    let policy = serde_json::json!({
        "entries": [
            {"capability_id": "fetch_record", "decision": "allow"},
            {"capability_id": "publish_report", "decision": "ask", "reason": "Publishing changes external state."}
        ]
    });
    let frames = vec![
        serde_json::json!({
            "v": 1, "type": "call", "req_id": "gao-plan-test", "tool_id": plan_id,
            "args": {"request": request, "catalog": catalog, "policy": policy}, "deadline_ms": 5_000
        }),
        serde_json::json!({
            "v": 1, "type": "call", "req_id": "gao-validation-test", "tool_id": validation_id,
            "args": {
                "workflow_name": "publish-request",
                "artifact_kind": "air",
                "artifact": "module { fetch_record publish_report }",
                "catalog": catalog,
                "policy": policy
            },
            "deadline_ms": 5_000
        }),
        serde_json::json!({
            "v": 1, "type": "call", "req_id": "gao-permission-test", "tool_id": permission_id,
            "args": {"capability_id": "publish_report", "catalog": catalog, "policy": policy},
            "deadline_ms": 5_000
        }),
    ];
    let results = run_typescript_worker(&tmp, vec![plan, validation, permission], &frames);
    let plan_value = worker_value(&results, "gao-plan-test");
    assert_eq!(
        plan_value
            .get("request")
            .and_then(serde_json::Value::as_str),
        Some(request),
        "plan_workflow must retain the typed request field"
    );
    assert_eq!(
        plan_value
            .pointer("/workflow/triggers/0")
            .and_then(serde_json::Value::as_str),
        Some("When a request arrives")
    );
    assert!(plan_value
        .pointer("/workflow/capabilities")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|entries| entries.iter().any(|entry| entry
            .get("id")
            .and_then(serde_json::Value::as_str)
            == Some("publish_report")
            && entry
                .get("requires_approval")
                .and_then(serde_json::Value::as_bool)
                == Some(true))));

    let validation_value = worker_result(&results, "gao-validation-test")
        .get("value")
        .unwrap();
    assert!(validation_value
        .pointer("/validation_plan")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|steps| steps.iter().any(|step| step
            .get("operation")
            .and_then(serde_json::Value::as_str)
            == Some("enforce_write_boundary"))));
    assert_eq!(
        validation_value
            .get("next_action")
            .and_then(serde_json::Value::as_str),
        Some("submit_for_validation")
    );

    let permission_value = worker_result(&results, "gao-permission-test")
        .get("value")
        .unwrap();
    assert_eq!(
        permission_value
            .get("decision")
            .and_then(serde_json::Value::as_str),
        Some("ask")
    );
    assert_eq!(
        permission_value
            .get("requires_approval")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        permission_value
            .get("reason")
            .and_then(serde_json::Value::as_str),
        Some("Publishing changes external state.")
    );
}

#[test]
fn gao_compose_gate_defers_valid_input_and_denies_missing_input_in_real_worker() {
    if !node_available() || !npm_available() {
        eprintln!("skipping gao_compose_gate_defers_valid_input_and_denies_missing_input_in_real_worker: node/npm not on PATH");
        return;
    }
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_build(&root, true).expect("gao agent build");
    let gate = worker_manifest_entry(&root, "gate_compose_workflow");
    let handler_id = gate
        .get("handler_id")
        .and_then(serde_json::Value::as_str)
        .unwrap();
    let hook_call = |req_id: &str, args: serde_json::Value| {
        serde_json::json!({
            "v": 1,
            "type": "call",
            "req_id": req_id,
            "tool_id": handler_id,
            "args": {"__apxm_hook__": {"event": "pre_cap", "call": {"name": "compose_workflow", "args": args}}},
            "deadline_ms": 5_000
        })
    };
    let frames = vec![
        hook_call(
            "valid",
            serde_json::json!({"name": "approved-flow", "air": "module { func.func @main() }"}),
        ),
        hook_call(
            "missing-name",
            serde_json::json!({"air": "module { func.func @main() }"}),
        ),
        hook_call(
            "missing-air",
            serde_json::json!({"name": "incomplete-flow"}),
        ),
    ];
    let results = run_typescript_worker(&tmp, vec![gate], &frames);
    let valid = worker_value(&results, "valid");
    assert_eq!(
        valid.get("decision").and_then(serde_json::Value::as_str),
        Some("defer")
    );
    assert_eq!(
        worker_result(&results, "missing-name")
            .pointer("/value/decision")
            .and_then(serde_json::Value::as_str),
        Some("deny")
    );
    assert_eq!(
        worker_result(&results, "missing-air")
            .pointer("/value/decision")
            .and_then(serde_json::Value::as_str),
        Some("deny")
    );
}

#[test]
fn gao_post_cap_hook_recursively_scrubs_nested_results_without_losing_types() {
    if !node_available() || !npm_available() {
        eprintln!("skipping gao_post_cap_hook_recursively_scrubs_nested_results_without_losing_types: node/npm not on PATH");
        return;
    }
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_build(&root, true).expect("gao agent build");
    let redactor = worker_manifest_entry(&root, "redact_tool_results");
    let handler_id = redactor
        .get("handler_id")
        .and_then(serde_json::Value::as_str)
        .unwrap();
    let frame = serde_json::json!({
        "v": 1,
        "type": "call",
        "req_id": "nested-redaction",
        "tool_id": handler_id,
        "args": {"__apxm_hook__": {
            "event": "post_cap",
            "call": {"name": "nested_result"},
            "result": {
                "count": 3,
                "ok": true,
                "nested": {
                    "token": "top-secret",
                    "items": ["Authorization: Bearer abc.def", {"note": "api_key=visible-secret", "value": 7}]
                }
            }
        }},
        "deadline_ms": 5_000
    });
    let results = run_typescript_worker(&tmp, vec![redactor], &[frame]);
    let scrubbed = worker_value(&results, "nested-redaction")
        .get("result")
        .unwrap();
    assert_eq!(
        scrubbed.get("count").and_then(serde_json::Value::as_i64),
        Some(3)
    );
    assert_eq!(
        scrubbed.get("ok").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        scrubbed
            .pointer("/nested/token")
            .and_then(serde_json::Value::as_str),
        Some("<redacted>")
    );
    assert_eq!(
        scrubbed
            .pointer("/nested/items/0")
            .and_then(serde_json::Value::as_str),
        Some("Authorization: <redacted>")
    );
    assert_eq!(
        scrubbed
            .pointer("/nested/items/1/note")
            .and_then(serde_json::Value::as_str),
        Some("api_key=<redacted>")
    );
    assert_eq!(
        scrubbed
            .pointer("/nested/items/1/value")
            .and_then(serde_json::Value::as_i64),
        Some(7)
    );
}

#[test]
fn gao_compile_service_declarative_emits_recv_loop_air() {
    if !node_available() || !npm_available() {
        eprintln!("skipping gao_compile_service_emits_await_input_air: node/npm not on PATH");
        return;
    }
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_build(&root, true).expect("gao agent build must succeed before compile-service");

    let air = emit_air_from_agent(&root)
        .expect("declarative compile-service AIR for gao");

    assert!(air.contains("ais.await_input"), "expected explicit input park");
    assert!(air.contains("rearm = \"true\""), "expected re-arming input park");
    assert!(
        air.contains("__apxm_handler_manifest__"),
        "gao AIR must carry the TypeScript handler manifest sidecar"
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
