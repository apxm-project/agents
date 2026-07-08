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

/// Sentinel prefix emitted by the Python frontend in a `;` comment when
/// `@tool`-decorated functions are registered via `Agent`.
#[cfg(feature = "driver")]
const PYTHON_TOOLS_PREFIX: &str = "; __apxm_python_tools__ ";
#[cfg(feature = "driver")]
const TYPESCRIPT_TOOLS_PREFIX: &str = "; __apxm_typescript_tools__ ";
/// Any APXM sidecar comment line (e.g. `; __apxm_hooks__ ...`). These `;`-lines
/// are metadata the MLIR parser cannot read and MUST be stripped before compile.
/// Only the python-tools sidecar is captured for the bridge; the rest (hooks)
/// travel inside the artifact as REGISTER_HOOK nodes + the tools manifest.
#[cfg(feature = "driver")]
const SIDECAR_LINE_PREFIX: &str = "; __apxm_";

/// Extract the `; __apxm_python_tools__ <json>` comment from AIR text.
///
/// Returns `(air_without_sidecar, Option<json_bytes>)`.
#[cfg(feature = "driver")]
fn extract_python_tools_sidecar(air: &str) -> (String, Option<Vec<u8>>) {
    let (filtered, python, typescript) = extract_handler_sidecars(air);
    let _ = typescript;
    (filtered, python)
}

#[cfg(feature = "driver")]
fn extract_handler_sidecars(air: &str) -> (String, Option<Vec<u8>>, Option<Vec<u8>>) {
    let mut python: Option<Vec<u8>> = None;
    let mut typescript: Option<Vec<u8>> = None;
    let mut filtered = String::with_capacity(air.len());
    for line in air.lines() {
        if let Some(json_str) = line.strip_prefix(PYTHON_TOOLS_PREFIX) {
            python = Some(json_str.as_bytes().to_vec());
        } else if let Some(json_str) = line.strip_prefix(TYPESCRIPT_TOOLS_PREFIX) {
            typescript = Some(json_str.as_bytes().to_vec());
        } else if line.starts_with(SIDECAR_LINE_PREFIX) {
        } else {
            if !filtered.is_empty() {
                filtered.push('\n');
            }
            filtered.push_str(line);
        }
    }
    (filtered, python, typescript)
}

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

/// `pub(super)` (not private) so `commands::package`'s skill-build
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
    let apxm_exe = env::current_exe().ok();
    let mut output = None;
    for candidate in ["python3", "python"] {
        let mut command = std::process::Command::new(candidate);
        command
            .arg(input)
            .env(apxm_env::PYTHONPATH, &pythonpath)
            .env(apxm_env::APXM_EMIT_AIR, apxm_env::flag_values::ENABLED);
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
    if !(trimmed.starts_with(';')
        || trimmed.starts_with('%')
        || trimmed.starts_with("module")
        || trimmed.starts_with("func.func"))
    {
        return Err(anyhow::anyhow!(
            "Python graph {} did not emit recognizable .air text on stdout.\n\
             Expected MLIR text starting with 'module', 'func.func', ';', or '%'.",
            input.display()
        ));
    }

    let (clean_air, sidecar) = extract_python_tools_sidecar(&air);

    let mut tmp = tempfile::Builder::new()
        .suffix(".air")
        .tempfile()
        .context("Failed to create temporary .air file")?;
    tmp.write_all(clean_air.as_bytes())
        .context("Failed to write emitted .air to temporary file")?;
    tmp.flush().context("Failed to flush temporary .air file")?;
    Ok((tmp, sidecar))
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
    let apxm_exe = env::current_exe().ok();
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
    if !(trimmed.starts_with(';')
        || trimmed.starts_with('%')
        || trimmed.starts_with("module")
        || trimmed.starts_with("func.func"))
    {
        return Err(anyhow::anyhow!(
            "TypeScript frontend source {} did not emit recognizable .air text on stdout.\n\
             Expected MLIR text starting with 'module', 'func.func', ';', or '%'.",
            input.display()
        ));
    }
    Ok(air)
}

#[cfg(feature = "driver")]
fn emit_air_from_typescript(input: &Path) -> Result<(tempfile::NamedTempFile, Option<Vec<u8>>)> {
    use std::io::Write;

    let air = emit_air_from_typescript_text(input)?;
    let (clean_air, _python, typescript) = extract_handler_sidecars(&air);

    let mut tmp = tempfile::Builder::new()
        .suffix(".air")
        .tempfile()
        .context("Failed to create temporary .air file")?;
    tmp.write_all(clean_air.as_bytes())
        .context("Failed to write emitted .air to temporary file")?;
    tmp.flush().context("Failed to flush temporary .air file")?;
    Ok((tmp, typescript))
}

/// Python tools sidecar data extracted from the AIR comment, if any.
#[cfg(feature = "driver")]
type PythonHandlersSidecar = Option<Vec<u8>>;
#[cfg(feature = "driver")]
type TypeScriptHandlersSidecar = Option<Vec<u8>>;

// ---------------------------------------------------------------------------
// `apxm compile-service` — the cross-repo process contract Server (and any
// other non-`agents` caller) uses to compile an agent package declaratively
// into AIR, without reaching into this repo's file layout. This is the
// canonical, agents-owned home for declarative package AIR emission across the
// service boundary.
// ---------------------------------------------------------------------------

/// Compile a bundled conversational agent package into canonical AIR text.
///
/// # Cross-repo I/O contract
///
/// This is the exact contract `apxm compile-service` exposes on stdout/stderr
/// for callers in other repos (Server) that invoke this binary as a
/// subprocess instead of duplicating a private path dependency on this repo's
/// Python frontend:
///
/// - **Input**: a single positional argument, the package directory
///   (containing `pack.toml`, `agent.toml`, and `capabilities/`). No stdin
///   is read.
/// - `--web-tools`: include the web capability group in the emitted ASK node.
/// - **stdout**: on success, ONLY emitted AIR text, including any
///   `; __apxm_typescript_tools__ ...` sidecar comment lines. No other text is
///   ever written to stdout; all progress/log/diagnostic output goes to stderr.
/// - **Exit code**: `0` on success. Nonzero on any failure, with a
///   human-readable message on stderr.
#[cfg(feature = "driver")]
pub fn compile_service_command(
    package: PathBuf,
    web_tools: bool,
    _config: Option<PathBuf>,
) -> Result<()> {
    let air = emit_air_from_agent_package(&package, web_tools)?;

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
pub(crate) fn emit_air_from_agent_package(package: &Path, web_tools: bool) -> Result<String> {
    if !package.is_dir() {
        return Err(anyhow::anyhow!(
            "'{}' is not a directory",
            package.display()
        ));
    }

    emit_air_from_declarative_package(package, web_tools)
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
        "hook handler '{handler}' not found in capabilities/handlers/tools.json; run package build first"
    ))
}

#[cfg(feature = "driver")]
fn emit_air_from_declarative_package(package: &Path, web_tools: bool) -> Result<String> {
    use apxm_ais::chat::escape_air_string;

    let agent_path = package.join("agent.toml");
    let agent_text = std::fs::read_to_string(&agent_path)
        .with_context(|| format!("Failed to read {}", agent_path.display()))?;
    let agent: DeclarativeAgentToml = toml::from_str(&agent_text)
        .with_context(|| format!("Failed to parse {}", agent_path.display()))?;

    let tools_path = package.join("capabilities/handlers/tools.json");
    let manifest: Vec<serde_json::Value> = if tools_path.is_file() {
        serde_json::from_str(
            &std::fs::read_to_string(&tools_path)
                .with_context(|| format!("Failed to read {}", tools_path.display()))?,
        )
        .with_context(|| format!("Failed to parse {}", tools_path.display()))?
    } else {
        Vec::new()
    };

    let persona = agent
        .prompts
        .get("persona")
        .map(|rel| package.join(rel))
        .filter(|path| path.is_file())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();

    let turn_param = declarative_turn_param(&agent);
    let loop_mode = declarative_loop_mode(&agent);
    let loop_rearms = declarative_loop_rearms(&agent);
    let use_recv_loop = loop_mode == "recv" && loop_rearms;

    let mut prefix = String::new();
    if !manifest.is_empty() {
        prefix.push_str(&format!(
            "{TYPESCRIPT_TOOLS_PREFIX}{}\n",
            serde_json::to_string(&manifest)?
        ));
    }

    let mut main_body = String::new();
    for (idx, hook) in agent.hooks.iter().enumerate() {
        let handler_id = resolve_declarative_handler_id(&hook.handler, &manifest)?;
        let name = format!("register_hook_{idx}");
        main_body.push_str(&format!(
            "    %{name} = ais.register_hook \"{}\" {{hook_match = \"{}\", hook_mode = \"{}\", python_hook_handler_id = \"{}\"}} : !ais.token\n",
            hook.event,
            hook.hook_match.as_deref().unwrap_or("*"),
            hook.mode,
            handler_id,
        ));
    }

    if use_recv_loop {
        let persona_attr = if persona.trim().is_empty() {
            String::new()
        } else {
            format!("system_prompt = \"{}\"", escape_air_string(persona.trim()))
        };
        let mut recv_attrs = vec![
            "mode = \"recv\"".to_string(),
            "recv_once = \"false\"".to_string(),
            "turn_agent = \"conversation\"".to_string(),
            "turn_flow = \"turn\"".to_string(),
            format!("turn_param = \"{turn_param}\""),
        ];
        if !persona_attr.is_empty() {
            recv_attrs.push(persona_attr);
        }
        main_body.push_str(&format!(
            "    %turn_loop = ais.autonomous \"{}\" {{{}}} (%arg0 : !ais.token) : !ais.token\n",
            escape_air_string(persona.trim()),
            recv_attrs.join(", "),
        ));
        main_body.push_str("    func.return %turn_loop : !ais.token\n");
    } else {
        main_body.push_str(&format!(
            "    %run_turn = ais.flow_call \"conversation\" \"turn\" {{args = {{{turn_param} = \"{{{turn_param}}}\"}}, input_names = [\"{turn_param}\"]}} (%arg0 : !ais.token) : !ais.token\n"
        ));
        main_body.push_str("    %done = ais.done %run_turn : !ais.token\n");
        main_body.push_str("    func.return %done : !ais.token\n");
    }

    let mut ask_attrs = vec!["conversational_turn = \"true\"".to_string()];
    if !persona.trim().is_empty() {
        ask_attrs.push(format!(
            "system_prompt = \"{}\"",
            escape_air_string(persona.trim())
        ));
    }
    let mut groups = vec!["discovery"];
    if web_tools {
        groups.push("web");
    }
    ask_attrs.push(format!(
        "capability_groups = [{}]",
        groups
            .iter()
            .map(|group| format!("\"{group}\""))
            .collect::<Vec<_>>()
            .join(", ")
    ));

    let turn_body = format!(
        "  func.func @conversation.turn(%arg0: !ais.token {{ais.param_name = \"{turn_param}\", ais.param_type = \"str\"}}) -> !ais.token {{\n    %answer = ais.ask \"{{{turn_param}}}\" {{{}}} : !ais.token\n    %turn_done = ais.done %answer : !ais.token\n    func.return %turn_done : !ais.token\n  }}\n",
        ask_attrs.join(", ")
    );

    Ok(format!(
        "{prefix}module {{\n  func.func @main(%arg0: !ais.token {{ais.param_name = \"{turn_param}\", ais.param_type = \"str\"}}) -> !ais.token attributes {{ais.entry}} {{\n{main_body}  }}\n{turn_body}}}\n"
    ))
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
    PythonHandlersSidecar,
    TypeScriptHandlersSidecar,
)> {
    if is_python_graph_input(input) {
        let (tmp, sidecar) = emit_air_from_python(input, config_path)?;
        return Ok((tmp.path().to_path_buf(), Some(tmp), sidecar, None));
    }

    if is_typescript_graph_input(input) {
        let (tmp, sidecar) = emit_air_from_typescript(input)?;
        return Ok((tmp.path().to_path_buf(), Some(tmp), None, sidecar));
    }

    let input_format = ApxmPathFormat::from_path(input);
    if input_format.is_json_data() {
        return Err(anyhow::anyhow!(
            ".json is structured data for metrics, sessions, manifests, diagnostics, and API envelopes. \
             Graph source input must be .air, .py, or .ts frontend source that emits .air."
        ));
    }

    if input_format.is_air_source() {
        let air = std::fs::read_to_string(input)
            .with_context(|| format!("Failed to read {}", input.display()))?;
        let (clean_air, python_sidecar, typescript_sidecar) = extract_handler_sidecars(&air);
        if python_sidecar.is_some() || typescript_sidecar.is_some() {
            use std::io::Write;

            let mut tmp = tempfile::Builder::new()
                .suffix(".air")
                .tempfile()
                .context("Failed to create temporary .air file")?;
            tmp.write_all(clean_air.as_bytes())
                .context("Failed to write stripped .air to temporary file")?;
            tmp.flush()
                .context("Failed to flush stripped .air temporary file")?;
            return Ok((
                tmp.path().to_path_buf(),
                Some(tmp),
                python_sidecar,
                typescript_sidecar,
            ));
        }
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
    let input_source = if input.is_dir() {
        resolve_directory_air_source(&input)?
    } else {
        input.clone()
    };
    let (graph_input, _frontend_air, python_tools_sidecar, typescript_tools_sidecar) =
        if input_source.is_dir() {
            unreachable!(
                "directory inputs are resolved to a canonical .air source before compilation"
            )
        } else {
            prepare_graph_input(&input_source, compiler_config_path.as_deref())?
        };

    let compile_start = std::time::Instant::now();
    let compiler = Compiler::with_opt_level(opt).context("Failed to initialize compiler")?;

    // Check if this is a new-format .air file (valid MLIR)
    let is_new_air =
        if !input_source.is_dir() && ApxmPathFormat::from_path(&graph_input).is_air_source() {
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
        // Parse manifest from sidecar for orphan @tool detection (W723)
        let manifest: Option<Vec<apxm_compiler::passes::PythonCapabilityManifestEntry>> =
            python_tools_sidecar.as_ref().and_then(|data| {
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

        // Inject python_tools sidecar section if present
        if let Some(sidecar_data) = &python_tools_sidecar {
            artifact.add_section(apxm_artifact::ArtifactSection {
                kind: apxm_runtime::python_tools::CAPABILITY_NAME.into(),
                data: sidecar_data.clone(),
            });
        }

        if let Some(sidecar_data) = &typescript_tools_sidecar {
            artifact.add_section(apxm_artifact::ArtifactSection {
                kind: apxm_runtime::typescript_tools::CAPABILITY_NAME.into(),
                data: sidecar_data.clone(),
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
    use apxm_core::constants::mlir::syntax as mlir_syntax;

    #[test]
    fn mlir_air_text_accepts_leading_sidecar_comments() {
        let text = format!(
            "{} frontend sidecar\n\n{} {{\n}}\n",
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
    fn extracts_python_and_typescript_sidecars() {
        let air = concat!(
            "; __apxm_python_tools__ [{\"handler_id\":\"py\"}]\n",
            "; __apxm_typescript_tools__ [{\"handler_id\":\"ts\"}]\n",
            "module {\n}\n",
        );

        let (clean, python, typescript) = extract_handler_sidecars(air);

        assert_eq!(clean, "module {\n}");
        assert_eq!(
            python.as_deref(),
            Some(br#"[{"handler_id":"py"}]"#.as_slice())
        );
        assert_eq!(
            typescript.as_deref(),
            Some(br#"[{"handler_id":"ts"}]"#.as_slice())
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
