//! Compile and decompile commands.

#[cfg(feature = "driver")]
use std::env;
#[cfg(feature = "driver")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(feature = "driver")]
use anyhow::Context;
use anyhow::Result;
#[cfg(feature = "driver")]
use apxm_core::constants::env as apxm_env;
#[cfg(feature = "driver")]
use apxm_core::constants::extensions;
#[cfg(feature = "driver")]
use apxm_core::types::ApxmPathFormat;
#[cfg(feature = "driver")]
use apxm_driver::compiler::Compiler;
#[cfg(feature = "driver")]
use serde::Deserialize;

#[cfg(feature = "driver")]
use super::implementations::{load_config, parse_opt_level};

#[cfg(feature = "driver")]
fn is_python_graph_input(input: &Path) -> bool {
    ApxmPathFormat::from_path(input).is_python_frontend()
}

#[cfg(feature = "driver")]
fn is_typescript_graph_input(input: &Path) -> bool {
    ApxmPathFormat::from_path(input).is_typescript_frontend()
}

#[cfg(feature = "driver")]
const PYTHON_TOOLS_MANIFEST_ENV: &str = "APXM_PYTHON_TOOLS_OUT";

/// Rewrite the `@apxm/frontend` bare import specifier to the built TypeScript
/// frontend entrypoint so standalone `.ts` source files can run without a local
/// `node_modules/` install next to each workflow file.
#[cfg(feature = "driver")]
fn rewrite_frontend_import(source: &str, frontend_dist_index: &Path) -> String {
    let dist_url = format!("file://{}", frontend_dist_index.display());
    source
        .replace("\"@apxm/frontend\"", &format!("\"{dist_url}\""))
        .replace("'@apxm/frontend'", &format!("'{dist_url}'"))
}

#[cfg(feature = "driver")]
fn current_apxm_exe() -> Option<PathBuf> {
    let current = env::current_exe().ok()?;
    let file_name = current.file_name()?.to_str()?;
    if let Some(parent) = current.parent()
        && parent.file_name().and_then(|name| name.to_str()) == Some("deps")
        && file_name.starts_with("apxm-")
        && let Some(profile_dir) = parent.parent()
    {
        let candidate = profile_dir.join(format!("apxm{}", std::env::consts::EXE_SUFFIX));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    Some(current)
}

/// `pub(super)` (not private) so `commands::agent`'s skill-build
/// step can drive the same Python-frontend AIR emission `apxm compile`
/// itself uses, rather than duplicating the PYTHONPATH/subprocess dance.
#[cfg(feature = "driver")]
pub(super) fn emit_air_from_python(
    input: &Path,
    config_path: Option<&Path>,
) -> Result<(tempfile::NamedTempFile, Option<Vec<u8>>)> {
    use std::io::Write;

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let python_frontend = repo_root.join("crates/compiler/frontend/python");

    let mut pythonpath_entries = vec![python_frontend, repo_root];
    if let Some(parent) = input.parent() {
        pythonpath_entries.push(parent.to_path_buf());
    }
    if let Some(existing) = env::var_os(apxm_env::PYTHONPATH) {
        pythonpath_entries.extend(env::split_paths(&existing));
    }

    let pythonpath = env::join_paths(pythonpath_entries)
        .context("Failed to build PYTHONPATH for APXM Python frontend")?;
    // Route the frontend's AIR emission back to this same binary's single Rust
    // printer (`apxm emit-air`), so compile is self-contained and
    // version-consistent.
    let apxm_exe = current_apxm_exe();
    let manifest_tmp = tempfile::Builder::new()
        .suffix(".json")
        .tempfile()
        .context("Failed to create temporary Python handler manifest file")?;
    let manifest_path = manifest_tmp.path().to_path_buf();
    let mut output = None;
    for candidate in ["python3", "python"] {
        let mut command = std::process::Command::new(candidate);
        command
            .arg(input)
            .env(apxm_env::PYTHONPATH, &pythonpath)
            .env(apxm_env::APXM_EMIT_AIR, apxm_env::flag_values::ENABLED)
            .env(PYTHON_TOOLS_MANIFEST_ENV, &manifest_path);
        if let Some(exe) = apxm_exe.as_ref() {
            command.env(apxm_env::APXM_BIN, exe);
        }
        if let Some(config_path) = config_path {
            command.env(apxm_env::APXM_CONFIG, config_path);
        }
        match command.output() {
            Ok(result) => {
                output = Some(result);
                break;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "Failed to run Python graph {} with {}: {}",
                    input.display(),
                    candidate,
                    err
                ));
            }
        }
    }

    let output = output.ok_or_else(|| {
        anyhow::anyhow!("Python interpreter not found on PATH (tried python3, python)")
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "Python graph {} failed: {}",
            input.display(),
            stderr.trim()
        ));
    }

    let air = String::from_utf8(output.stdout)
        .with_context(|| format!("Python graph {} did not emit valid UTF-8", input.display()))?;
    let trimmed = air.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!(
            "Python graph {} produced no .air output",
            input.display()
        ));
    }
    if !(trimmed.starts_with('%')
        || trimmed.starts_with("module")
        || trimmed.starts_with("func.func"))
    {
        return Err(anyhow::anyhow!(
            "Python graph {} did not emit recognizable .air text on stdout.\n\
             Expected MLIR text starting with 'module', 'func.func', or '%'.",
            input.display()
        ));
    }

    let manifest = std::fs::read(&manifest_path)
        .ok()
        .filter(|data| !data.is_empty());

    let mut tmp = tempfile::Builder::new()
        .suffix(".air")
        .tempfile()
        .context("Failed to create temporary .air file")?;
    tmp.write_all(air.as_bytes())
        .context("Failed to write emitted .air to temporary file")?;
    tmp.flush().context("Failed to flush temporary .air file")?;
    Ok((tmp, manifest))
}

/// Run TypeScript frontend source through Node and capture the AIR it emits.
#[cfg(feature = "driver")]
pub(super) fn emit_air_from_typescript_text(input: &Path) -> Result<String> {
    use anyhow::bail;

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let frontend_dist = repo_root.join("crates/compiler/frontend/typescript/dist/index.js");
    if !frontend_dist.is_file() {
        bail!(
            "TypeScript frontend is not built: expected {} (run 'npm run build' in \
             crates/compiler/frontend/typescript first)",
            frontend_dist.display()
        );
    }

    let source = std::fs::read_to_string(input)
        .with_context(|| format!("Failed to read {}", input.display()))?;
    let rewritten = rewrite_frontend_import(&source, &frontend_dist);

    let mut tmp = tempfile::Builder::new()
        .suffix(".ts")
        .tempfile()
        .context("Failed to create temporary TypeScript frontend source")?;
    {
        use std::io::Write;
        tmp.write_all(rewritten.as_bytes())
            .context("Failed to write rewritten TypeScript frontend source")?;
        tmp.flush()
            .context("Failed to flush temporary TypeScript frontend source")?;
    }

    // Route the frontend's AIR emission back to this same binary's single Rust
    // printer (`apxm emit-air`).
    let apxm_exe = current_apxm_exe();
    let mut node_command = std::process::Command::new("node");
    node_command.arg(tmp.path());
    if let Some(exe) = apxm_exe.as_ref() {
        node_command.env(apxm_env::APXM_BIN, exe);
    }
    let output = node_command.output().map_err(|err| {
        anyhow::anyhow!(
            "Failed to run Node on TypeScript frontend source {}: {err} (is Node.js installed?)",
            input.display()
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "TypeScript frontend source {} failed: {}",
            input.display(),
            stderr.trim()
        ));
    }

    let air = String::from_utf8(output.stdout).with_context(|| {
        format!(
            "TypeScript frontend source {} did not emit valid UTF-8",
            input.display()
        )
    })?;
    let trimmed = air.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!(
            "TypeScript frontend source {} produced no .air output",
            input.display()
        ));
    }
    if !(trimmed.starts_with('%')
        || trimmed.starts_with("module")
        || trimmed.starts_with("func.func"))
    {
        return Err(anyhow::anyhow!(
            "TypeScript frontend source {} did not emit recognizable .air text on stdout.\n\
             Expected MLIR text starting with 'module', 'func.func', or '%'.",
            input.display()
        ));
    }
    Ok(air)
}

#[cfg(feature = "driver")]
fn emit_air_from_typescript(input: &Path) -> Result<(tempfile::NamedTempFile, Option<Vec<u8>>)> {
    use std::io::Write;

    let air = emit_air_from_typescript_text(input)?;

    let mut tmp = tempfile::Builder::new()
        .suffix(".air")
        .tempfile()
        .context("Failed to create temporary .air file")?;
    tmp.write_all(air.as_bytes())
        .context("Failed to write emitted .air to temporary file")?;
    tmp.flush().context("Failed to flush temporary .air file")?;
    Ok((tmp, None))
}

/// Python tools manifest data extracted from the frontend subprocess, if any.
#[cfg(feature = "driver")]
type PythonHandlersManifest = Option<Vec<u8>>;
#[cfg(feature = "driver")]
type TypeScriptHandlersManifest = Option<Vec<u8>>;

// ---------------------------------------------------------------------------
// `apxm compile-service` — the cross-repo process contract Server (and any
// other non-`agents` caller) uses to compile an agent declaratively
// into AIR, without reaching into this repo's file layout. This is the
// canonical, agents-owned home for declarative agent AIR emission across the
// service boundary.
// ---------------------------------------------------------------------------

/// Compile a bundled conversational agent into canonical AIR text.
///
/// # Cross-repo I/O contract
///
/// This is the exact contract `apxm compile-service` exposes on stdout/stderr
/// for callers in other repos (Server) that invoke this binary as a
/// subprocess instead of duplicating a private path dependency on this repo's
/// Python frontend:
///
/// - **Input**: a single positional argument, the agent directory
///   (containing `agent.toml`, generated `integrity.toml`, and `capabilities/`). No stdin
///   is read.
/// - `--web-tools`: include the web capability group in the emitted ASK node.
/// - **stdout**: on success, ONLY emitted AIR text. No other text is ever
///   written to stdout; all progress/log/diagnostic output goes to stderr.
/// - **Exit code**: `0` on success. Nonzero on any failure, with a
///   human-readable message on stderr.
#[cfg(feature = "driver")]
pub fn compile_service_command(
    agent_dir: PathBuf,
    web_tools: bool,
    _config: Option<PathBuf>,
) -> Result<()> {
    let air = emit_air_from_agent(&agent_dir, web_tools)?;

    // Only the AIR text goes to stdout, written byte-for-byte as the frontend
    // produced it (no added trailing newline) — this is the process contract
    // Server relies on to capture stdout verbatim as the compiled AIR.
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(air.as_bytes())?;
    Ok(())
}

/// Core logic behind `apxm compile-service`, factored out from the stdout-
/// writing wrapper above so it can be exercised directly by tests (including
/// the Studio-equivalence fixture test) without spawning a subprocess.
#[cfg(feature = "driver")]
pub(crate) fn emit_air_from_agent(agent_dir: &Path, web_tools: bool) -> Result<String> {
    if !agent_dir.is_dir() {
        return Err(anyhow::anyhow!(
            "'{}' is not a directory",
            agent_dir.display()
        ));
    }

    emit_air_from_declarative_agent(agent_dir, web_tools)
}

#[cfg(feature = "driver")]
#[derive(Debug, Deserialize)]
struct DeclarativeHookToml {
    event: String,
    #[serde(rename = "match")]
    hook_match: Option<String>,
    mode: String,
    handler: String,
}

#[cfg(feature = "driver")]
#[derive(Debug, Deserialize, Default)]
struct DeclarativeRuntimeToml {
    #[serde(default, rename = "loop")]
    runtime_loop: Option<toml::Value>,
    /// `[runtime] compaction_policy = '{"keep_recent":...}'` — the W2.7
    /// declarative-package compaction dial
    /// (`RUNTIME_EXTRA_COMPACTION_POLICY_KEY` in
    /// `apxm_core::types::agent_definition`): a JSON string stamped into the
    /// same open `extra` bag every other `[runtime]` knob uses. Absent
    /// entirely is the opt-out dial — no attributes are stamped on the
    /// marked conversational-turn ASK, so `ConversationMemoryMiddleware`
    /// never starts measuring (docs/plans/tasks/W5.1.md).
    #[serde(default)]
    compaction_policy: Option<String>,
}

/// Parsed `[runtime] compaction_policy` JSON shape
/// (`{"keep_recent","compact_at_tokens","strategy","summary_key"}`) — the
/// same fields `conversational.py::CompactionPolicy` round-trips through the
/// `extra` bag (`agent_definition.rs`'s
/// `runtime_extra_round_trips_compaction_policy_knob` test).
#[cfg(feature = "driver")]
#[derive(Debug, Deserialize)]
struct DeclarativeCompactionPolicy {
    compact_at_tokens: i64,
    #[serde(default)]
    keep_recent: Option<i64>,
    #[serde(default)]
    summary_key: Option<String>,
}

/// Node attribute keys `ConversationMemoryMiddleware`
/// (`crates/runtime/engine/src/executor/middlewares/conversation_memory.rs`)
/// reads off the marked conversational-turn ASK. Duplicated string literals
/// here match the existing cross-frontend precedent (the Python frontend's
/// `conversational.py` independently duplicates the same four literals
/// rather than importing them from the runtime crate) — every producer
/// agrees on the wire string, not a shared Rust constant.
#[cfg(feature = "driver")]
mod compaction_attrs {
    pub const AT_TOKENS: &str = "compaction_at_tokens";
    pub const KEEP_RECENT: &str = "compaction_keep_recent";
    pub const SUMMARY_KEY: &str = "compaction_summary_key";
    pub const OVERRIDE_PRESENT: &str = "compaction_override_present";
}

/// Parse `[runtime] compaction_policy` (if present) into the node attributes
/// `ConversationMemoryMiddleware` reads. Returns `None` for the opt-out
/// dial (key absent) or a malformed value (fail-loud: `Err`), never a
/// silent partial policy.
#[cfg(feature = "driver")]
fn declarative_compaction_policy(
    agent: &DeclarativeAgentToml,
) -> Result<Option<DeclarativeCompactionPolicy>> {
    let Some(runtime) = agent.runtime.as_ref() else {
        return Ok(None);
    };
    let Some(raw) = runtime.compaction_policy.as_ref() else {
        return Ok(None);
    };
    let policy: DeclarativeCompactionPolicy = serde_json::from_str(raw).with_context(|| {
        format!("Failed to parse [runtime] compaction_policy as JSON: {raw:?}")
    })?;
    Ok(Some(policy))
}

/// Fail-closed precedence (W2.7 threat model): an author-declared `post_turn`
/// hook already owns compaction, so the runtime default must never
/// double-compact. Declarative packages express hooks via `[[hooks]]`, so
/// this is a scan for `event = "post_turn"`, not a Python-only signal.
#[cfg(feature = "driver")]
fn declarative_compaction_override_present(agent: &DeclarativeAgentToml) -> bool {
    agent.hooks.iter().any(|hook| hook.event == "post_turn")
}

#[cfg(feature = "driver")]
#[derive(Debug, Deserialize)]
struct DeclarativeAgentToml {
    #[serde(default)]
    hooks: Vec<DeclarativeHookToml>,
    #[serde(default)]
    runtime: Option<DeclarativeRuntimeToml>,
    #[serde(default)]
    prompts: std::collections::BTreeMap<String, String>,
}

#[cfg(feature = "driver")]
#[derive(Debug, Deserialize, Default)]
struct DeclarativeCapabilitiesToml {
    #[serde(default, rename = "capability")]
    capability: Vec<DeclarativeCapabilityToml>,
}

#[cfg(feature = "driver")]
#[derive(Debug, Deserialize)]
struct DeclarativeCapabilityToml {
    id: String,
    kind: String,
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    builtin_group: Option<String>,
}

#[cfg(feature = "driver")]
fn declarative_turn_param(agent: &DeclarativeAgentToml) -> String {
    agent
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.runtime_loop.as_ref())
        .and_then(|value| value.get("turn_param"))
        .and_then(|value| value.as_str())
        .unwrap_or("user_message")
        .to_string()
}

#[cfg(feature = "driver")]
fn declarative_loop_mode(agent: &DeclarativeAgentToml) -> String {
    agent
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.runtime_loop.as_ref())
        .and_then(|value| value.get("mode"))
        .and_then(|value| value.as_str())
        .unwrap_or("host")
        .to_string()
}

#[cfg(feature = "driver")]
fn declarative_loop_rearms(agent: &DeclarativeAgentToml) -> bool {
    agent
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.runtime_loop.as_ref())
        .and_then(|value| value.get("rearm"))
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

#[cfg(feature = "driver")]
fn manifest_entry_matches_handler(entry: &serde_json::Value, handler: &str) -> bool {
    let entry_qual = entry.get("qualname").and_then(|v| v.as_str()).unwrap_or("");
    let entry_source = entry
        .get("source_file")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .replace('\\', "/");

    if let Some((path_part, qualname)) = handler.split_once(':') {
        let path_part = path_part.trim_start_matches("./").replace('\\', "/");
        return entry_qual == qualname
            && (entry_source.ends_with(&path_part)
                || entry_source.ends_with(&format!("{path_part}.ts")));
    }

    let normalized = handler.replace('.', "/").replace('\\', "/");
    let Some((path_part, qualname)) = normalized.rsplit_once('/') else {
        return entry_qual == handler;
    };
    entry_qual == qualname
        && (entry_source.ends_with(path_part)
            || entry_source.ends_with(&format!("{path_part}.ts"))
            || entry_source.contains(&format!("/{path_part}.ts")))
}

#[cfg(feature = "driver")]
fn resolve_declarative_handler_id(handler: &str, manifest: &[serde_json::Value]) -> Result<String> {
    for entry in manifest {
        if manifest_entry_matches_handler(entry, handler) {
            return entry
                .get("handler_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| {
                    anyhow::anyhow!("manifest entry for '{handler}' missing handler_id")
                });
        }
    }
    Err(anyhow::anyhow!(
        "hook handler '{handler}' not found in capabilities/handlers/tools.json; run agent build first"
    ))
}

#[cfg(feature = "driver")]
fn declarative_tool_surface(
    capabilities: &DeclarativeCapabilitiesToml,
    web_tools: bool,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut tools = Vec::new();
    let mut groups = Vec::new();
    if web_tools {
        groups.push(apxm_ais::capabilities::groups::WEB.to_string());
    }
    for cap in &capabilities.capability {
        match cap.kind.as_str() {
            "typescript_handler" | "python_handler" => {
                if cap.read_only {
                    tools.push(cap.id.clone());
                }
            }
            "builtin" => {
                if cap.read_only {
                    tools.push(cap.id.clone());
                }
                if let Some(group) = cap.builtin_group.as_ref()
                    && !groups.contains(group)
                {
                    groups.push(group.clone());
                }
            }
            "host" | "provider" | "http" | "static" | "mcp" => {}
            other => {
                anyhow::bail!(
                    "capability '{}' has unsupported kind '{other}'; expected one of builtin, typescript_handler, python_handler, host, provider, http, static, mcp",
                    cap.id
                );
            }
        }
    }
    tools.sort();
    tools.dedup();
    groups.sort();
    groups.dedup();
    Ok((tools, groups))
}

#[cfg(feature = "driver")]
fn append_typescript_tools_sidecar(
    mut air: String,
    manifest: &[serde_json::Value],
) -> Result<String> {
    if manifest.is_empty() {
        return Ok(air);
    }
    let sidecar =
        serde_json::to_string(manifest).context("Failed to serialize TypeScript tools sidecar")?;
    if !air.ends_with('\n') {
        air.push('\n');
    }
    air.push_str("; __apxm_typescript_tools__ ");
    air.push_str(&sidecar);
    Ok(air)
}

#[cfg(feature = "driver")]
fn absolutize_typescript_tool_sources(
    manifest: Vec<serde_json::Value>,
    agent_dir: &Path,
) -> Result<Vec<serde_json::Value>> {
    let agent_dir = agent_dir
        .canonicalize()
        .with_context(|| format!("Failed to resolve {}", agent_dir.display()))?;
    Ok(manifest
        .into_iter()
        .map(|mut entry| {
            if let Some(source_file) = entry
                .get("source_file")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                && !Path::new(&source_file).is_absolute()
                && let Some(object) = entry.as_object_mut()
            {
                object.insert(
                    "source_file".to_string(),
                    serde_json::Value::String(
                        agent_dir.join(source_file).to_string_lossy().into_owned(),
                    ),
                );
            }
            entry
        })
        .collect())
}

#[cfg(feature = "driver")]
fn emit_air_from_declarative_agent(agent_dir: &Path, web_tools: bool) -> Result<String> {
    use apxm_compiler::{
        AirModule, AirProgram, FrontendEdge, FrontendGraph, FrontendNode, FrontendParameter,
    };
    use apxm_core::constants::graph::{attrs as graph_attrs, metadata as graph_meta};
    use apxm_core::types::{AISOperationType, DependencyType, Value};
    use std::collections::HashMap;

    let agent_path = agent_dir.join("agent.toml");
    let agent_text = std::fs::read_to_string(&agent_path)
        .with_context(|| format!("Failed to read {}", agent_path.display()))?;
    let agent: DeclarativeAgentToml = toml::from_str(&agent_text)
        .with_context(|| format!("Failed to parse {}", agent_path.display()))?;

    let tools_path = agent_dir.join("capabilities/handlers/tools.json");
    let manifest: Vec<serde_json::Value> = if tools_path.is_file() {
        serde_json::from_str(
            &std::fs::read_to_string(&tools_path)
                .with_context(|| format!("Failed to read {}", tools_path.display()))?,
        )
        .with_context(|| format!("Failed to parse {}", tools_path.display()))?
    } else {
        Vec::new()
    };
    let manifest = absolutize_typescript_tool_sources(manifest, agent_dir)?;
    let capabilities_path = agent_dir.join("capabilities/capabilities.toml");
    let capabilities: DeclarativeCapabilitiesToml = if capabilities_path.is_file() {
        toml::from_str(
            &std::fs::read_to_string(&capabilities_path)
                .with_context(|| format!("Failed to read {}", capabilities_path.display()))?,
        )
        .with_context(|| format!("Failed to parse {}", capabilities_path.display()))?
    } else {
        DeclarativeCapabilitiesToml::default()
    };
    let (tool_names, capability_groups) = declarative_tool_surface(&capabilities, web_tools)?;

    let persona = agent
        .prompts
        .get("persona")
        .map(|rel| agent_dir.join(rel))
        .filter(|path| path.is_file())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();

    let turn_param = declarative_turn_param(&agent);
    let loop_mode = declarative_loop_mode(&agent);
    let loop_rearms = declarative_loop_rearms(&agent);
    let use_recv_loop = loop_mode == "recv" && loop_rearms;

    let parameter = FrontendParameter {
        name: turn_param.clone(),
        type_name: "str".to_string(),
    };

    let mut nodes = Vec::new();
    for (idx, hook) in agent.hooks.iter().enumerate() {
        let handler_id = resolve_declarative_handler_id(&hook.handler, &manifest)?;
        nodes.push(FrontendNode {
            id: idx as u64 + 1,
            name: format!("register_hook_{idx}"),
            op: AISOperationType::RegisterHook,
            attributes: HashMap::from([
                (
                    graph_attrs::HOOK_EVENT.to_string(),
                    Value::String(hook.event.clone()),
                ),
                (
                    graph_attrs::HOOK_MATCH.to_string(),
                    Value::String(hook.hook_match.clone().unwrap_or_else(|| "*".to_string())),
                ),
                (
                    graph_attrs::HOOK_MODE.to_string(),
                    Value::String(hook.mode.clone()),
                ),
                (
                    graph_attrs::PYTHON_HOOK_HANDLER_ID.to_string(),
                    Value::String(handler_id),
                ),
            ]),
        });
    }

    let run_node_id = nodes.len() as u64 + 1;
    let return_node_id = run_node_id + 1;
    let edges = Vec::from([FrontendEdge {
        from: run_node_id,
        to: return_node_id,
        dependency: DependencyType::Data,
    }]);

    if use_recv_loop {
        let mut attrs = HashMap::from([
            (
                graph_attrs::PROMPT.to_string(),
                Value::String(persona.trim().to_string()),
            ),
            (
                graph_attrs::MODE.to_string(),
                Value::String("recv".to_string()),
            ),
            ("recv_once".to_string(), Value::String("false".to_string())),
            (
                "turn_agent".to_string(),
                Value::String("conversation".to_string()),
            ),
            ("turn_flow".to_string(), Value::String("turn".to_string())),
            ("turn_param".to_string(), Value::String(turn_param.clone())),
            (
                graph_attrs::INPUT_NAMES.to_string(),
                Value::Array(vec![Value::String(turn_param.clone())]),
            ),
        ]);
        if !persona.trim().is_empty() {
            attrs.insert(
                graph_attrs::SYSTEM_PROMPT.to_string(),
                Value::String(persona.trim().to_string()),
            );
        }
        nodes.push(FrontendNode {
            id: run_node_id,
            name: "turn_loop".to_string(),
            op: AISOperationType::Autonomous,
            attributes: attrs,
        });
    } else {
        nodes.push(FrontendNode {
            id: run_node_id,
            name: "run_turn".to_string(),
            op: AISOperationType::FlowCall,
            attributes: HashMap::from([
                (
                    graph_attrs::AGENT_NAME.to_string(),
                    Value::String("conversation".to_string()),
                ),
                (
                    graph_attrs::FLOW_NAME.to_string(),
                    Value::String("turn".to_string()),
                ),
                (
                    graph_attrs::ARGS.to_string(),
                    Value::Object(HashMap::from([(
                        turn_param.clone(),
                        Value::String(format!("{{{turn_param}}}")),
                    )])),
                ),
                (
                    graph_attrs::INPUT_NAMES.to_string(),
                    Value::Array(vec![Value::String(turn_param.clone())]),
                ),
            ]),
        });
    }

    nodes.push(FrontendNode {
        id: return_node_id,
        name: "return_turn".to_string(),
        op: AISOperationType::Return,
        attributes: HashMap::new(),
    });

    let main_graph = FrontendGraph {
        name: "main".to_string(),
        nodes,
        edges,
        parameters: vec![parameter.clone()],
        metadata: HashMap::from([(graph_meta::IS_ENTRY.to_string(), Value::Bool(true))]),
    };

    let mut ask_attrs = HashMap::from([
        (
            graph_attrs::TEMPLATE_STR.to_string(),
            Value::String(format!("{{{turn_param}}}")),
        ),
        (
            "conversational_turn".to_string(),
            Value::String("true".to_string()),
        ),
    ]);
    if !persona.trim().is_empty() {
        ask_attrs.insert(
            graph_attrs::SYSTEM_PROMPT.to_string(),
            Value::String(persona.trim().to_string()),
        );
    }
    if !tool_names.is_empty() {
        ask_attrs.insert(
            graph_attrs::TOOLS.to_string(),
            Value::Array(tool_names.into_iter().map(Value::String).collect()),
        );
    }
    if !capability_groups.is_empty() {
        ask_attrs.insert(
            graph_attrs::CAPABILITY_GROUPS.to_string(),
            Value::Array(capability_groups.into_iter().map(Value::String).collect()),
        );
    }
    // W2.7 declarative compaction dial: absent `compaction_policy` is the
    // opt-out no-op (no attributes stamped at all); present-but-malformed is
    // a hard compile error (fail loud), never a silently-ignored policy.
    if let Some(policy) = declarative_compaction_policy(&agent)? {
        ask_attrs.insert(
            compaction_attrs::AT_TOKENS.to_string(),
            Value::Number(apxm_core::types::values::Number::Integer(
                policy.compact_at_tokens,
            )),
        );
        if let Some(keep_recent) = policy.keep_recent {
            ask_attrs.insert(
                compaction_attrs::KEEP_RECENT.to_string(),
                Value::Number(apxm_core::types::values::Number::Integer(keep_recent)),
            );
        }
        if let Some(summary_key) = policy.summary_key {
            ask_attrs.insert(
                compaction_attrs::SUMMARY_KEY.to_string(),
                Value::String(summary_key),
            );
        }
        ask_attrs.insert(
            compaction_attrs::OVERRIDE_PRESENT.to_string(),
            Value::Bool(declarative_compaction_override_present(&agent)),
        );
    }

    let turn_graph = FrontendGraph {
        name: "conversation.turn".to_string(),
        nodes: vec![
            FrontendNode {
                id: 1,
                name: "answer".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs,
            },
            FrontendNode {
                id: 2,
                name: "return_answer".to_string(),
                op: AISOperationType::Return,
                attributes: HashMap::new(),
            },
        ],
        edges: vec![FrontendEdge {
            from: 1,
            to: 2,
            dependency: DependencyType::Data,
        }],
        parameters: vec![parameter],
        metadata: HashMap::from([(graph_meta::IS_ENTRY.to_string(), Value::Bool(false))]),
    };

    let modules = [main_graph, turn_graph]
        .iter()
        .map(FrontendGraph::to_air_module)
        .collect::<std::result::Result<Vec<AirModule>, _>>()?;
    append_typescript_tools_sidecar(AirProgram::new(modules).to_air()?, &manifest)
}

/// `; __apxm_typescript_tools__ <json>` — the trailing sidecar comment line
/// `append_typescript_tools_sidecar` appends to declarative-agent AIR.
/// Consumed by Server directly; `apxm compile` (below) strips it before
/// handing the AIR to the MLIR parser and re-attaches it as an embedded
/// artifact section instead.
#[cfg(feature = "driver")]
const TYPESCRIPT_TOOLS_SIDECAR_PREFIX: &str = "; __apxm_typescript_tools__ ";

/// Split a declarative agent's emitted AIR (as `compile-service` writes it:
/// canonical AIR text plus a trailing typescript-tools sidecar comment) into
/// sidecar-free AIR text (safe for the MLIR parser) and the raw sidecar JSON
/// bytes, if present.
#[cfg(feature = "driver")]
fn split_typescript_tools_sidecar(air: &str) -> (String, Option<Vec<u8>>) {
    let mut clean_lines = Vec::new();
    let mut sidecar = None;
    for line in air.lines() {
        if let Some(json) = line.strip_prefix(TYPESCRIPT_TOOLS_SIDECAR_PREFIX) {
            sidecar = Some(json.as_bytes().to_vec());
        } else {
            clean_lines.push(line);
        }
    }
    (clean_lines.join("\n"), sidecar)
}

/// Prepare a declarative agent-package directory (`agent.toml`, no
/// Python/TypeScript entry file) for `apxm compile`, the same way
/// [`prepare_graph_input`] prepares a `.py`/`.ts` frontend entry: emit AIR
/// via [`emit_air_from_agent`] (the exact `compile-service` code path),
/// strip the trailing typescript-tools sidecar comment into a temp `.air`
/// file, and thread the sidecar through as the `typescript_tools_manifest`
/// so `compile_command` embeds it into the artifact exactly like a `.ts`
/// frontend entry's tool manifest.
#[cfg(feature = "driver")]
fn prepare_graph_input_from_declarative_agent(
    agent_dir: &Path,
) -> Result<(
    PathBuf,
    Option<tempfile::NamedTempFile>,
    PythonHandlersManifest,
    TypeScriptHandlersManifest,
)> {
    use std::io::Write;

    let air_with_sidecar = emit_air_from_agent(agent_dir, false)?;
    let (clean_air, ts_manifest) = split_typescript_tools_sidecar(&air_with_sidecar);

    let mut tmp = tempfile::Builder::new()
        .suffix(".air")
        .tempfile()
        .context("Failed to create temporary .air file for declarative agent compile")?;
    tmp.write_all(clean_air.as_bytes())
        .context("Failed to write emitted declarative-agent AIR to temporary file")?;
    tmp.flush().context("Failed to flush temporary .air file")?;

    Ok((tmp.path().to_path_buf(), Some(tmp), None, ts_manifest))
}

#[cfg(feature = "driver")]
fn is_mlir_air_text(text: &str) -> bool {
    use apxm_core::constants::mlir::syntax as mlir_syntax;

    text.lines()
        .map(str::trim_start)
        .find(|line| !line.is_empty() && !line.starts_with(mlir_syntax::LINE_COMMENT_PREFIX))
        .is_some_and(|line| {
            line.starts_with(mlir_syntax::MODULE_KEYWORD)
                || line.starts_with(mlir_syntax::FUNC_FUNC_PREFIX)
        })
}

#[cfg(feature = "driver")]
pub(super) fn prepare_graph_input(
    input: &Path,
    config_path: Option<&Path>,
) -> Result<(
    PathBuf,
    Option<tempfile::NamedTempFile>,
    PythonHandlersManifest,
    TypeScriptHandlersManifest,
)> {
    if is_python_graph_input(input) {
        let (tmp, manifest) = emit_air_from_python(input, config_path)?;
        return Ok((tmp.path().to_path_buf(), Some(tmp), manifest, None));
    }

    if is_typescript_graph_input(input) {
        let (tmp, manifest) = emit_air_from_typescript(input)?;
        return Ok((tmp.path().to_path_buf(), Some(tmp), None, manifest));
    }

    let input_format = ApxmPathFormat::from_path(input);
    if input_format.is_json_data() {
        return Err(anyhow::anyhow!(
            ".json is structured data for metrics, sessions, manifests, diagnostics, and API envelopes. \
             Graph source input must be .air, .py, or .ts frontend source that emits .air."
        ));
    }

    if !input_format.is_air_source() {
        return Err(anyhow::anyhow!(
            "Unsupported workflow input '{}'. Use .air for canonical workflow source, .py or .ts for frontend source, or .apxmobj with 'dekk agents run'.",
            input.display()
        ));
    }

    Ok((input.to_path_buf(), None, None, None))
}

#[cfg(feature = "driver")]
fn resolve_directory_air_source(dir: &Path) -> Result<PathBuf> {
    let mut workflows = Vec::new();
    let subdirs = ["flows", "nodes", ""];

    for subdir in &subdirs {
        let search_dir = if subdir.is_empty() {
            dir.to_path_buf()
        } else {
            dir.join(subdir)
        };
        if !search_dir.is_dir() {
            continue;
        }

        for entry in std::fs::read_dir(&search_dir)
            .with_context(|| format!("Failed to read {}", search_dir.display()))?
        {
            let path = entry?.path();
            if ApxmPathFormat::from_path(&path).is_air_source() {
                workflows.push(path);
            }
        }
    }

    match workflows.len() {
        0 => Err(anyhow::anyhow!(
            "No .air workflow source found in directory '{}'",
            dir.display()
        )),
        1 => Ok(workflows.remove(0)),
        count => Err(anyhow::anyhow!(
            "Directory '{}' contains {count} .air workflow sources. Provide a single .air file instead.",
            dir.display()
        )),
    }
}

/// Strip `artifact_hash` from a `skill.toml` text so the resulting bytes can
/// be embedded inside the artifact they would otherwise hash. Other fields are
/// preserved verbatim so the server's `validate_embedded_manifest_field`
/// round-trip succeeds field-for-field against the on-disk manifest.
#[cfg(feature = "driver")]
fn strip_artifact_hash_for_embed(manifest_toml: &str) -> Result<Vec<u8>> {
    let mut value: toml::Value = toml::from_str(manifest_toml)
        .with_context(|| "failed to parse skill.toml for --embed-manifest")?;
    if let Some(table) = value.as_table_mut() {
        table.remove("artifact_hash");
        if let Some(nested) = table.get_mut("skill").and_then(|v| v.as_table_mut()) {
            nested.remove("artifact_hash");
        }
    }
    let text = toml::to_string(&value)
        .with_context(|| "failed to re-serialize skill.toml for --embed-manifest")?;
    Ok(text.into_bytes())
}

#[cfg(feature = "driver")]
#[allow(clippy::too_many_arguments)]
pub fn compile_command(
    input: PathBuf,
    output: Option<PathBuf>,
    emit_diagnostics: Option<PathBuf>,
    emit_metrics: Option<PathBuf>,
    opt_level: u8,
    target: apxm_core::types::OptimizationTarget,
    no_cse_llm: bool,
    profile: Option<PathBuf>,
    warn: bool,
    disable_passes: Vec<String>,
    pass_list_override: Option<Vec<String>>,
    config: Option<PathBuf>,
    embed_manifest: Option<PathBuf>,
) -> Result<()> {
    use apxm_core::constants::diagnostics;
    use apxm_core::constants::session::metrics_keys;
    use apxm_core::types::PipelineConfig;

    let opt = parse_opt_level(opt_level);
    let compiler_config_path = config.clone();
    let opt_target = target;
    let _apxm_config = load_config(config.clone())?;
    // A directory containing `agent.toml` (no Python/TypeScript entry file)
    // is a declarative agent package — the same `compile-service` compiles
    // to AIR-with-trailing-typescript-tools-sidecar. Route it through
    // `prepare_graph_input_from_declarative_agent` instead of
    // `resolve_directory_air_source` (which only ever looks for a
    // pre-lowered `.air` file) so `apxm compile examples/agents/<id> -o
    // out.apxmobj` + `apxm run out.apxmobj` works for a hooks-only package,
    // the same way it already does for `.py`/`.ts` frontend entries
    // (docs/plans/tasks/W5.1.md).
    let is_declarative_agent_dir = input.is_dir() && input.join("agent.toml").is_file();
    let input_source = if input.is_dir() {
        if is_declarative_agent_dir {
            input.clone()
        } else {
            resolve_directory_air_source(&input)?
        }
    } else {
        input.clone()
    };
    let (graph_input, _frontend_air, python_tools_manifest, typescript_tools_manifest) =
        if is_declarative_agent_dir {
            prepare_graph_input_from_declarative_agent(&input_source)?
        } else if input_source.is_dir() {
            unreachable!(
                "directory inputs are resolved to a canonical .air source before compilation"
            )
        } else {
            prepare_graph_input(&input_source, compiler_config_path.as_deref())?
        };

    let compile_start = std::time::Instant::now();
    let compiler = Compiler::with_opt_level(opt).context("Failed to initialize compiler")?;

    // Check if this is a new-format .air file (valid MLIR). The declarative
    // agent-package branch above always produces a clean (sidecar-stripped)
    // temp `.air` file, so it takes this same fast path.
    let is_new_air = if (is_declarative_agent_dir || !input_source.is_dir())
        && ApxmPathFormat::from_path(&graph_input).is_air_source()
    {
        std::fs::read_to_string(&graph_input)
            .map(|text| is_mlir_air_text(&text))
            .unwrap_or(false)
    } else {
        false
    };

    // For new .air format (valid MLIR), compile directly without AirModule.
    // The .air path skips graph lowering, but still honors PipelineConfig
    // (target / disable_passes / pass_list_override / warn_unconsumed) and
    // --emit-diagnostics so the ablation harness can drive .air corpora.
    if is_new_air {
        let air_text = std::fs::read_to_string(&graph_input)
            .with_context(|| format!("Failed to read {}", graph_input.display()))?;
        let needs_custom_config = no_cse_llm
            || opt_target != apxm_core::types::OptimizationTarget::Balanced
            || compiler_config_path.is_some()
            || profile.is_some()
            || warn
            || !disable_passes.is_empty()
            || pass_list_override.is_some();

        let needs_diagnostics = emit_diagnostics.is_some() || emit_metrics.is_some();
        let (module, air_pass_diagnostics) = if needs_diagnostics {
            let config = PipelineConfig {
                opt_level: opt,
                target: opt_target,
                verify: true,
                no_cse_llm,
                profile_path: profile.clone(),
                compiler_config_path: compiler_config_path.clone(),
                warn_unconsumed: warn,
                disable_passes: disable_passes.clone(),
                pass_list_override: pass_list_override.clone(),
                ..Default::default()
            };
            let (m, d) = compiler
                .compile_air_with_config_and_diagnostics(&air_text, config)
                .map_err(|e| anyhow::anyhow!("Failed to compile MLIR: {e}"))?;
            (m, Some(d))
        } else if needs_custom_config {
            let config = PipelineConfig {
                opt_level: opt,
                target: opt_target,
                verify: true,
                no_cse_llm,
                profile_path: profile.clone(),
                compiler_config_path: compiler_config_path.clone(),
                warn_unconsumed: warn,
                disable_passes: disable_passes.clone(),
                pass_list_override: pass_list_override.clone(),
                ..Default::default()
            };
            let m = compiler
                .compile_air_with_config(&air_text, config)
                .map_err(|e| anyhow::anyhow!("Failed to compile MLIR: {e}"))?;
            (m, None)
        } else {
            let m = compiler
                .compile(&graph_input)
                .map_err(|e| anyhow::anyhow!("Failed to compile MLIR: {e}"))?;
            (m, None)
        };
        let compile_time = compile_start.elapsed();

        let artifact_start = std::time::Instant::now();
        // Parse manifest from the frontend subprocess for orphan @tool detection (W723).
        let manifest: Option<Vec<apxm_compiler::passes::PythonCapabilityManifestEntry>> =
            python_tools_manifest.as_ref().and_then(|data| {
                serde_json::from_slice(data)
                    .map_err(|e| {
                        eprintln!("warning: failed to parse python_tools manifest: {e}");
                        e
                    })
                    .ok()
            });
        let mut artifact = module
            .generate_artifact_with_manifest(None, manifest.as_deref())
            .context("Failed to generate artifact")?;

        // Embed Python handler manifest section when the frontend supplied one.
        if let Some(manifest_data) = &python_tools_manifest {
            artifact.add_section(apxm_artifact::ArtifactSection {
                kind: apxm_runtime::python_tools::CAPABILITY_NAME.into(),
                data: manifest_data.clone(),
            });
        }

        if let Some(manifest_data) = &typescript_tools_manifest {
            artifact.add_section(apxm_artifact::ArtifactSection {
                kind: apxm_runtime::typescript_tools::CAPABILITY_NAME.into(),
                data: manifest_data.clone(),
            });
        }

        // Embed the supplied skill.toml as an apxm.skill_manifest.v1 section.
        // The embedded copy has artifact_hash stripped (it cannot live inside
        // the artifact it hashes); the server's
        // `validate_embedded_manifest_field` round-trip succeeds against the
        // on-disk manifest because all other fields are preserved verbatim.
        if let Some(manifest_path) = embed_manifest.as_ref() {
            let manifest_toml = std::fs::read_to_string(manifest_path)
                .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
            let stripped = strip_artifact_hash_for_embed(&manifest_toml)?;
            artifact.add_section(apxm_artifact::ArtifactSection {
                kind: apxm_artifact::section_kinds::SKILL_MANIFEST_V1.into(),
                data: stripped,
            });
            // Pin created_at to 0 so the wire bytes (and the BLAKE3 recorded
            // in skill.toml::artifact_hash) are stable across rebuilds.
            artifact.set_created_at(0);
        }

        let bytes = artifact
            .to_bytes()
            .map_err(|err| anyhow::anyhow!("Failed to serialize artifact: {err}"))?;
        let artifact_time = artifact_start.elapsed();

        let out_path = output.unwrap_or_else(|| input_source.with_extension(extensions::ARTIFACT));
        std::fs::write(&out_path, &bytes)
            .with_context(|| format!("Failed to write {}", out_path.display()))?;

        println!(
            "{} Compiled to {}",
            apxm_core::constants::ui::icons::SUCCESS,
            out_path.display()
        );
        println!("  Compilation: {:?}", compile_time);
        println!("  Artifact generation: {:?}", artifact_time);
        println!("  Artifact size: {} bytes", bytes.len());

        // Emit diagnostics if requested.
        if let (Some(diag_path), Some(diag)) = (&emit_diagnostics, &air_pass_diagnostics) {
            let compiler_json = diag.to_json();
            let diagnostics_json = serde_json::json!({
                "input": graph_input.display().to_string(),
                "mode": diagnostics::MODE_AIR,
                "optimization_level": format!("O{}", opt_level),
                "compilation_phases": {
                    "total_ms": compile_time.as_secs_f64() * 1000.0,
                    "artifact_gen_ms": artifact_time.as_secs_f64() * 1000.0,
                    "passes_ms": diag.total_duration_ms
                },
                "pass_metrics": compiler_json[metrics_keys::COMPILER_PASSES].clone(),
                "pass_summary": compiler_json[metrics_keys::COMPILER_SUMMARY].clone()
            });

            std::fs::write(diag_path, serde_json::to_string_pretty(&diagnostics_json)?)
                .with_context(|| {
                    format!("Failed to write diagnostics to {}", diag_path.display())
                })?;
            println!("Wrote diagnostics to {}", diag_path.display());
        }

        // Emit unified metrics report (compiler-only).
        if let Some(metrics_path) = emit_metrics {
            use apxm_compiler::passes::metrics::CompilerMetricsSource;

            let mut report = apxm_core::MetricsReport::new();
            if let Some(ref diag) = air_pass_diagnostics {
                report.add_source(&CompilerMetricsSource { diagnostics: diag });
            }
            std::fs::write(
                &metrics_path,
                serde_json::to_string_pretty(&report.to_json())?,
            )
            .with_context(|| format!("Failed to write metrics to {}", metrics_path.display()))?;
            println!("Wrote metrics to {}", metrics_path.display());
        }

        return Ok(());
    }

    Err(anyhow::anyhow!(
        "Input '{}' is not canonical AIR. Graph source must be .air, .py, or .ts frontend source that emits .air.",
        input_source.display()
    ))
}

#[cfg(feature = "driver")]
pub fn decompile_command(artifact_path: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let bytes = std::fs::read(&artifact_path)
        .with_context(|| format!("Failed to read {}", artifact_path.display()))?;
    let artifact = apxm_artifact::Artifact::from_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("Failed to parse artifact: {}", e))?;
    let dag = artifact
        .entry_dag()
        .ok_or_else(|| anyhow::anyhow!("Artifact contains no entry DAG"))?;

    let graph = graph_from_execution_dag(dag);
    let air = graph
        .to_air()
        .map_err(|err| anyhow::anyhow!("Failed to emit AIR from artifact: {err}"))?;

    if let Some(out_path) = output {
        std::fs::write(&out_path, &air)
            .with_context(|| format!("Failed to write {}", out_path.display()))?;
        println!("Decompiled to {}", out_path.display());
    } else {
        println!("{}", air);
    }
    Ok(())
}

#[cfg(feature = "driver")]
pub(super) fn air_graph_from_source(input: &Path) -> Result<apxm_compiler::AirModule> {
    let compiler = Compiler::with_opt_level(apxm_core::types::OptimizationLevel::O0)
        .context("Failed to initialize compiler for AIR inspection")?;
    let module = compiler.compile(input).map_err(|err| {
        anyhow::anyhow!("Failed to compile AIR source '{}': {err}", input.display())
    })?;
    let artifact = module
        .generate_artifact_with_manifest(None, None)
        .context("Failed to generate inspection artifact")?;
    let dag = artifact
        .entry_dag()
        .ok_or_else(|| anyhow::anyhow!("Inspection artifact contains no entry DAG"))?;
    Ok(graph_from_execution_dag(dag))
}

#[cfg(all(test, feature = "driver"))]
mod tests {
    use super::*;
    use apxm_compiler::{Context, Module, Pipeline};
    use apxm_core::constants::mlir::syntax as mlir_syntax;
    use apxm_core::types::OptimizationLevel;
    use std::fs;
    use tempfile::tempdir;

    fn write_declarative_agent(mode: &str) -> tempfile::TempDir {
        let tmp = tempdir().expect("temp agent");
        let root = tmp.path();
        fs::create_dir_all(root.join("capabilities/handlers")).expect("handlers dir");
        fs::write(root.join("persona.md"), "You are concise.\n").expect("persona");
        fs::write(
            root.join("agent.toml"),
            format!(
                r#"
id = "demo"

[runtime.loop]
mode = "{mode}"
rearm = true
turn_param = "user_message"

[prompts]
persona = "persona.md"

[[hooks]]
event = "pre_turn"
match = "*"
mode = "observe"
handler = "hooks.pre_turn"
"#
            ),
        )
        .expect("agent toml");
        fs::write(
            root.join("capabilities/handlers/tools.json"),
            r#"[{"source_file":"hooks.ts","qualname":"pre_turn","handler_id":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]"#,
        )
        .expect("tools manifest");
        tmp
    }

    fn assert_cli_air_round_trips(air: &str) {
        let parseable_air = air
            .lines()
            .filter(|line| !line.starts_with("; __apxm_"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!parseable_air.contains("__apxm_"));
        assert!(!parseable_air.contains("ais.done"));
        assert!(parseable_air.contains("ais.return"));

        let context = Context::new().expect("compiler context");
        let parsed =
            Module::parse(&context, &parseable_air).expect("Module::parse accepts CLI AIR");
        parsed.verify().expect("parsed module verifies");

        Pipeline::with_opt_level(&context, OptimizationLevel::O0)
            .compile(&parseable_air)
            .expect("O0 compile accepts CLI AIR");
    }

    fn typescript_tools_sidecar(air: &str) -> Vec<serde_json::Value> {
        let raw = air
            .lines()
            .find_map(|line| line.strip_prefix("; __apxm_typescript_tools__ "))
            .expect("typescript sidecar");
        serde_json::from_str(raw).expect("typescript sidecar json")
    }

    #[test]
    fn mlir_air_text_accepts_leading_mlir_comments() {
        let text = format!(
            "{} frontend comment\n\n{} {{\n}}\n",
            mlir_syntax::LINE_COMMENT_PREFIX,
            mlir_syntax::MODULE_KEYWORD
        );

        assert!(is_mlir_air_text(&text));
    }

    #[test]
    fn mlir_air_text_accepts_standalone_function() {
        let text = format!("{} @main() {{}}\n", mlir_syntax::FUNC_FUNC_PREFIX);

        assert!(is_mlir_air_text(&text));
    }

    #[test]
    fn mlir_air_text_rejects_json_graph() {
        assert!(!is_mlir_air_text("{\"nodes\": []}"));
    }

    #[test]
    fn declarative_host_loop_air_round_trips_through_mlir_parser() {
        let agent_dir = write_declarative_agent("host");
        let air = emit_air_from_agent(agent_dir.path(), true).expect("declarative AIR");

        assert!(air.contains("ais.register_hook \"pre_turn\""));
        assert!(air.contains("python_hook_handler_id"));
        assert!(air.contains("ais.flow_call \"conversation\" \"turn\""));
        assert!(air.contains("args = {user_message = \"{user_message}\"}"));
        assert!(air.contains("capability_groups = [\"web\"]"));
        assert!(air.contains("__apxm_typescript_tools__"));
        assert_cli_air_round_trips(&air);
    }

    #[test]
    fn declarative_recv_loop_air_round_trips_through_mlir_parser() {
        let agent_dir = write_declarative_agent("recv");
        let air = emit_air_from_agent(agent_dir.path(), false).expect("declarative AIR");

        assert!(air.contains("ais.autonomous"));
        assert!(air.contains("mode = \"recv\""));
        assert!(air.contains("recv_once = \"false\""));
        assert!(!air.contains("capability_groups = ["));
        assert!(air.contains("__apxm_typescript_tools__"));
        assert_cli_air_round_trips(&air);
    }

    #[test]
    fn declarative_air_sidecar_resolves_typescript_sources_for_server_worker() {
        let agent_dir = write_declarative_agent("recv");
        let air = emit_air_from_agent(agent_dir.path(), false).expect("declarative AIR");
        let sidecar = typescript_tools_sidecar(&air);
        let source_file = sidecar[0]
            .get("source_file")
            .and_then(|value| value.as_str())
            .expect("source_file");

        assert!(Path::new(source_file).is_absolute(), "{source_file}");
        assert_eq!(
            source_file,
            agent_dir.path().join("hooks.ts").to_string_lossy()
        );
    }

    #[test]
    fn declarative_air_sidecar_resolves_relative_agent_dir_sources_for_server_worker() {
        let root = PathBuf::from("target/apxm-relative-agent-fixture");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("capabilities/handlers")).expect("fixture dirs");
        fs::write(root.join("persona.md"), "You are concise.\n").expect("persona");
        fs::write(
            root.join("agent.toml"),
            r#"
id = "relative-demo"

[runtime.loop]
mode = "recv"
rearm = true
turn_param = "user_message"

[prompts]
persona = "persona.md"

[[hooks]]
event = "pre_turn"
match = "*"
mode = "observe"
handler = "hooks.pre_turn"
"#,
        )
        .expect("agent toml");
        fs::write(
            root.join("capabilities/handlers/tools.json"),
            r#"[{"source_file":"hooks.ts","qualname":"pre_turn","handler_id":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]"#,
        )
        .expect("tools manifest");

        let air = emit_air_from_agent(&root, false).expect("declarative AIR");
        let sidecar = typescript_tools_sidecar(&air);
        let source_file = sidecar[0]
            .get("source_file")
            .and_then(|value| value.as_str())
            .expect("source_file");

        assert!(Path::new(source_file).is_absolute(), "{source_file}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn declarative_capability_entries_require_kind() {
        let agent_dir = write_declarative_agent("recv");
        fs::create_dir_all(agent_dir.path().join("capabilities")).expect("capabilities dir");
        fs::write(
            agent_dir.path().join("capabilities/capabilities.toml"),
            r#"
[[capability]]
id = "fixture.read"
read_only = true
"#,
        )
        .expect("capabilities toml");

        let err = emit_air_from_agent(agent_dir.path(), false).expect_err("kind is required");
        let debug = format!("{err:?}");
        assert!(
            debug.contains("Failed to parse")
                && debug.contains("capabilities.toml")
                && debug.contains("kind"),
            "expected missing kind error, got {err}"
        );
    }

    #[test]
    fn declarative_capability_entries_reject_unknown_kind() {
        let agent_dir = write_declarative_agent("recv");
        fs::create_dir_all(agent_dir.path().join("capabilities")).expect("capabilities dir");
        fs::write(
            agent_dir.path().join("capabilities/capabilities.toml"),
            r#"
[[capability]]
id = "fixture.read"
kind = "custom"
read_only = true
"#,
        )
        .expect("capabilities toml");

        let err = emit_air_from_agent(agent_dir.path(), false).expect_err("kind is canonical");
        assert!(
            err.to_string().contains("unsupported kind 'custom'"),
            "expected unsupported kind error, got {err}"
        );
    }

    #[test]
    fn strip_artifact_hash_round_trips_other_fields() {
        let original = r#"
skill_id = "demo"
version = "0.1.0"
entry_flow = "main"
artifact_hash = "blake3:deadbeef"
required_capabilities = ["workflow_emission_v1"]
"#;
        let stripped = strip_artifact_hash_for_embed(original).expect("strip");
        let parsed = apxm_skill::parse_manifest(std::str::from_utf8(&stripped).expect("utf8"))
            .expect("re-parse stripped manifest");
        assert_eq!(parsed.skill_id, "demo");
        assert_eq!(parsed.version, "0.1.0");
        assert_eq!(parsed.entry_flow, "main");
        assert_eq!(parsed.artifact_hash, None);
        assert_eq!(parsed.required_capabilities, vec!["workflow_emission_v1"]);
    }

    #[test]
    fn embedded_manifest_section_round_trips_through_artifact() {
        use apxm_artifact::{Artifact, ArtifactMetadata, ArtifactSection, section_kinds};

        let manifest_toml = r#"
skill_id = "demo"
version = "0.1.0"
entry_flow = "main"
required_capabilities = ["workflow_emission_v1"]
"#;
        let stripped = strip_artifact_hash_for_embed(manifest_toml).expect("strip");

        let mut artifact = Artifact::new(
            ArtifactMetadata::new(Some("demo".into()), "test"),
            Vec::new(),
        );
        artifact.add_section(ArtifactSection {
            kind: section_kinds::SKILL_MANIFEST_V1.into(),
            data: stripped.clone(),
        });

        let bytes = artifact.to_bytes().expect("serialize artifact");
        let decoded = Artifact::from_bytes(&bytes).expect("read back artifact");
        let section_bytes = decoded
            .section_data(section_kinds::SKILL_MANIFEST_V1)
            .expect("embedded skill_manifest section");
        assert_eq!(section_bytes, stripped.as_slice());

        let reparsed =
            apxm_skill::parse_manifest(std::str::from_utf8(section_bytes).expect("utf8"))
                .expect("re-parse embedded manifest");
        let original = apxm_skill::parse_manifest(manifest_toml).expect("parse original manifest");
        // artifact_hash is intentionally stripped; every other declared field
        // matches the on-disk manifest field-for-field.
        let mut expected = original;
        expected.artifact_hash = None;
        assert_eq!(reparsed, expected);
    }

    #[test]
    fn set_created_at_yields_byte_identical_artifacts() {
        // When the compiler pins created_at, two consecutive serializations
        // of the same (graph + manifest + sections) tuple must produce
        // identical bytes — required for the artifact_hash chain to mean
        // anything across rebuilds.
        use apxm_artifact::{Artifact, ArtifactMetadata, ArtifactSection, section_kinds};

        let make = || {
            let mut a = Artifact::new(
                ArtifactMetadata::new(Some("demo".into()), "test"),
                Vec::new(),
            );
            a.add_section(ArtifactSection {
                kind: section_kinds::SKILL_MANIFEST_V1.into(),
                data: b"skill_id = \"demo\"\n".to_vec(),
            });
            a.set_created_at(0);
            a.to_bytes().expect("serialize artifact")
        };

        let b1 = make();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b2 = make();
        assert_eq!(b1, b2, "set_created_at must produce byte-identical output");
    }

    // -----------------------------------------------------------------
    // W2.7 declarative compaction dial (docs/plans/tasks/W5.1.md, step 2:
    // "replace the hand-rolled compact_conversation hook with the W2.7
    // declarative CompactionPolicy").
    // -----------------------------------------------------------------

    fn write_declarative_agent_with_compaction(compaction_policy: Option<&str>) -> tempfile::TempDir {
        let tmp = tempdir().expect("temp agent");
        let root = tmp.path();
        fs::create_dir_all(root.join("capabilities/handlers")).expect("handlers dir");
        fs::write(root.join("persona.md"), "You are concise.\n").expect("persona");
        let compaction_line = compaction_policy
            .map(|json| format!("compaction_policy = {json:?}\n"))
            .unwrap_or_default();
        fs::write(
            root.join("agent.toml"),
            format!(
                r#"
id = "demo"

[runtime]
{compaction_line}

[runtime.loop]
mode = "host"
rearm = true
turn_param = "user_message"

[prompts]
persona = "persona.md"

[[hooks]]
event = "pre_turn"
match = "*"
mode = "observe"
handler = "hooks.pre_turn"
"#
            ),
        )
        .expect("agent toml");
        fs::write(
            root.join("capabilities/handlers/tools.json"),
            r#"[{"source_file":"hooks.ts","qualname":"pre_turn","handler_id":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]"#,
        )
        .expect("tools manifest");
        tmp
    }

    #[test]
    fn declarative_compaction_policy_stamps_marked_ask_attrs() {
        let policy = r#"{"keep_recent":2,"compact_at_tokens":300,"strategy":"summarize","summary_key":"conversation.summary"}"#;
        let agent_dir = write_declarative_agent_with_compaction(Some(policy));
        let air = emit_air_from_agent(agent_dir.path(), false).expect("declarative AIR");

        assert!(air.contains("compaction_at_tokens = 300"), "{air}");
        assert!(air.contains("compaction_keep_recent = 2"), "{air}");
        assert!(
            air.contains("compaction_summary_key = \"conversation.summary\""),
            "{air}"
        );
        assert!(
            air.contains("compaction_override_present = false"),
            "no post_turn hook is declared, so override_present must be false: {air}"
        );
        assert_cli_air_round_trips(&air);
    }

    #[test]
    fn declarative_compaction_policy_absent_is_the_opt_out_dial() {
        let agent_dir = write_declarative_agent_with_compaction(None);
        let air = emit_air_from_agent(agent_dir.path(), false).expect("declarative AIR");

        assert!(!air.contains("compaction_at_tokens"), "{air}");
        assert!(!air.contains("compaction_keep_recent"), "{air}");
        assert!(!air.contains("compaction_override_present"), "{air}");
    }

    #[test]
    fn declarative_compaction_policy_malformed_json_is_a_hard_compile_error() {
        let agent_dir = write_declarative_agent_with_compaction(Some("not json"));
        let err = emit_air_from_agent(agent_dir.path(), false)
            .expect_err("malformed compaction_policy must fail loud, not silently no-op");
        assert!(
            format!("{err}").contains("compaction_policy"),
            "error must name the offending field: {err}"
        );
    }

    #[test]
    fn declarative_compaction_override_present_true_when_author_declares_post_turn_hook() {
        let tmp = tempdir().expect("temp agent");
        let root = tmp.path();
        fs::create_dir_all(root.join("capabilities/handlers")).expect("handlers dir");
        fs::write(root.join("persona.md"), "You are concise.\n").expect("persona");
        fs::write(
            root.join("agent.toml"),
            r#"
id = "demo"

[runtime]
compaction_policy = "{\"compact_at_tokens\":300}"

[runtime.loop]
mode = "host"
rearm = true
turn_param = "user_message"

[prompts]
persona = "persona.md"

[[hooks]]
event = "post_turn"
match = "*"
mode = "observe"
handler = "hooks.author_compaction"
"#,
        )
        .expect("agent toml");
        fs::write(
            root.join("capabilities/handlers/tools.json"),
            r#"[{"source_file":"hooks.ts","qualname":"author_compaction","handler_id":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]"#,
        )
        .expect("tools manifest");

        let air = emit_air_from_agent(root, false).expect("declarative AIR");
        assert!(
            air.contains("compaction_override_present = true"),
            "an author-declared post_turn hook must set the fail-closed override signal: {air}"
        );
    }

    // -----------------------------------------------------------------
    // `apxm compile <agent_dir>` — declarative agent-package directory
    // compile support (docs/plans/tasks/W5.1.md step 4: "shell out to
    // `apxm run examples/agents/conversational/<artifact>.apxmobj`").
    // -----------------------------------------------------------------

    #[test]
    fn split_typescript_tools_sidecar_extracts_trailing_comment() {
        let air = "module {\n}\n; __apxm_typescript_tools__ [{\"a\":1}]";
        let (clean, sidecar) = split_typescript_tools_sidecar(air);
        assert!(!clean.contains("__apxm_typescript_tools__"));
        assert!(clean.contains("module {"));
        assert_eq!(sidecar.as_deref(), Some(&b"[{\"a\":1}]"[..]));
    }

    #[test]
    fn split_typescript_tools_sidecar_is_none_when_absent() {
        let air = "module {\n}\n";
        let (clean, sidecar) = split_typescript_tools_sidecar(air);
        assert_eq!(clean.trim_end(), "module {\n}".trim_end());
        assert!(sidecar.is_none());
    }

    #[test]
    fn prepare_graph_input_from_declarative_agent_yields_clean_air_and_manifest() {
        let agent_dir = write_declarative_agent("host");
        let (air_path, _tmp, python_manifest, ts_manifest) =
            prepare_graph_input_from_declarative_agent(agent_dir.path())
                .expect("prepare declarative agent graph input");

        let air_text = fs::read_to_string(&air_path).expect("read temp air");
        assert!(!air_text.contains("__apxm_typescript_tools__"));
        assert!(python_manifest.is_none());
        assert!(ts_manifest.is_some(), "declarative TS handlers must round-trip a manifest");

        // The clean AIR must still parse and verify through the real MLIR
        // pipeline — the exact bar `apxm compile` itself applies.
        assert_cli_air_round_trips(&air_text);
    }
}

/// Convert an ExecutionDag back to an AirModule for decompile and session output.
#[cfg(feature = "driver")]
pub(super) fn graph_from_execution_dag(
    dag: &apxm_core::types::execution::ExecutionDag,
) -> apxm_compiler::AirModule {
    use apxm_compiler::{AirEdge, AirNode, AirParam};
    use std::collections::HashMap;

    let nodes = dag
        .nodes
        .iter()
        .map(|node| AirNode {
            id: node.id,
            name: node
                .metadata
                .name
                .clone()
                .unwrap_or_else(|| format!("node_{}", node.id)),
            op: node.op_type,
            attributes: node.attributes.clone(),
        })
        .collect::<Vec<_>>();

    let edges = dag
        .edges
        .iter()
        .map(|edge| AirEdge {
            from: edge.from,
            to: edge.to,
            dependency: edge.dependency_type.clone(),
        })
        .collect::<Vec<_>>();

    let parameters = dag
        .metadata
        .parameters
        .iter()
        .map(|param| AirParam {
            name: param.name.clone(),
            type_name: param.type_name.clone(),
        })
        .collect::<Vec<_>>();

    let mut metadata = HashMap::new();
    if dag.metadata.is_entry {
        metadata.insert(
            apxm_core::constants::graph::metadata::IS_ENTRY.to_string(),
            apxm_core::types::values::Value::Bool(true),
        );
    }

    apxm_compiler::AirModule {
        name: dag
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "artifact".to_string()),
        nodes,
        edges,
        parameters,
        metadata,
    }
}
