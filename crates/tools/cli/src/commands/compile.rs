//! Compile and decompile commands.

use std::env;
#[cfg(feature = "driver")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(feature = "driver")]
use anyhow::Context;
use anyhow::Result;
use apxm_core::constants::env as apxm_env;
#[cfg(feature = "driver")]
use apxm_core::constants::extensions;
#[cfg(feature = "driver")]
use apxm_core::types::{ApxmPathFormat, HANDLER_MANIFEST_AIR_SIDECAR_PREFIX, HandlerManifest};
#[cfg(feature = "driver")]
use apxm_driver::compiler::Compiler;
#[cfg(feature = "driver")]
use serde::Deserialize;

#[cfg(feature = "driver")]
use super::agent::{CompileToml, FrontendLanguage, installed_typescript_frontend_entry};
#[cfg(feature = "driver")]
use super::implementations::{load_config, parse_opt_level};
#[cfg(feature = "driver")]
use apxm_ais::chat::CompileServiceOptions;

#[cfg(feature = "driver")]
fn is_python_graph_input(input: &Path) -> bool {
    ApxmPathFormat::from_path(input).is_python_frontend()
}

#[cfg(feature = "driver")]
fn is_typescript_graph_input(input: &Path) -> bool {
    ApxmPathFormat::from_path(input).is_typescript_frontend()
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
) -> Result<(tempfile::NamedTempFile, HandlerManifestBytes)> {
    use std::io::Write;

    let mut pythonpath_entries = Vec::new();
    if let Some(parent) = input.parent() {
        pythonpath_entries.push(parent.to_path_buf());
    }
    if let Some(existing) = env::var_os(apxm_env::PYTHONPATH) {
        pythonpath_entries.extend(env::split_paths(&existing));
    }

    let pythonpath = (!pythonpath_entries.is_empty())
        .then(|| env::join_paths(pythonpath_entries))
        .transpose()
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
            .env(apxm_env::APXM_EMIT_AIR, apxm_env::flag_values::ENABLED)
            .env(apxm_env::APXM_PYTHON_TOOLS_OUT, &manifest_path);
        if let Some(pythonpath) = pythonpath.as_ref() {
            command.env(apxm_env::PYTHONPATH, pythonpath);
        }
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
    let apxm_exe = current_apxm_exe();
    let runner = installed_typescript_frontend_entry("dist/runner.js")?;
    let mut node_command = std::process::Command::new("node");
    node_command
        .arg("--input-type=module")
        .arg("--eval")
        .arg(
            "import { pathToFileURL } from 'node:url'; import(pathToFileURL(process.argv[1]).href).then(({ runSource }) => runSource(process.argv[2]))",
        )
        .arg(runner)
        .arg(input);
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
pub(super) fn emit_air_from_typescript(
    input: &Path,
) -> Result<(tempfile::NamedTempFile, HandlerManifestBytes)> {
    use std::io::Write;

    let air = emit_air_from_typescript_text(input)?;
    let manifest_tmp = tempfile::Builder::new()
        .suffix(".json")
        .tempfile()
        .context("Failed to create temporary TypeScript handler manifest file")?;
    let manifest_path = manifest_tmp.path().to_path_buf();
    let root = input.parent().unwrap_or_else(|| Path::new("."));
    let compiler = installed_typescript_frontend_entry("dist/compile-handlers.js")?;
    let mut node_command = std::process::Command::new("node");
    node_command
        .arg("--input-type=module")
        .arg("--eval")
        .arg(
            "import { pathToFileURL } from 'node:url'; import(pathToFileURL(process.argv[1]).href).then(async ({ compileHandlers }) => { const fs = await import('node:fs/promises'); const manifest = await compileHandlers([process.argv[2]], { rootDir: process.argv[3] }); await fs.writeFile(process.argv[4], JSON.stringify(manifest)); })",
        )
        .arg(compiler)
        .arg(input)
        .arg(root)
        .arg(&manifest_path);
    if let Some(exe) = current_apxm_exe().as_ref() {
        node_command.env(apxm_env::APXM_BIN, exe);
    }
    let output = node_command.output().map_err(|error| {
        anyhow::anyhow!(
            "Failed to emit TypeScript handler manifest for {}: {error}",
            input.display()
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "TypeScript handler manifest for {} failed: {}",
            input.display(),
            stderr.trim()
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

/// Handler-manifest data extracted from a frontend subprocess, if any.
#[cfg(feature = "driver")]
type HandlerManifestBytes = Option<Vec<u8>>;

// ---------------------------------------------------------------------------
// `apxm compile-service` — the cross-repo process contract Server (and any
// other non-`agents` caller) uses to compile an agent declaratively
// into AIR, without reaching into this repo's file layout. This is the
// canonical, agents-owned home for declarative agent AIR emission across the
// service boundary.
// ---------------------------------------------------------------------------

#[cfg(feature = "driver")]
fn parse_compile_service_options_json(json: &str) -> Result<CompileServiceOptions> {
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let options = CompileServiceOptions::deserialize(&mut deserializer)
        .context("compile-service --options-stdin expects one typed JSON object on stdin")?;
    deserializer
        .end()
        .context("compile-service --options-stdin expects exactly one JSON object on stdin")?;
    options
        .validate()
        .context("compile-service options validation failed")?;
    Ok(options)
}

#[cfg(feature = "driver")]
fn read_compile_service_options_from_stdin() -> Result<CompileServiceOptions> {
    use std::io::Read;

    let mut stdin = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin)
        .context("Failed to read compile-service options JSON from stdin")?;
    parse_compile_service_options_json(&stdin)
}

/// Compile a package-declared frontend entry into canonical AIR text.
///
/// # Cross-repo I/O contract
///
/// This is the exact contract `apxm compile-service` exposes on stdout/stderr
/// for callers in other repos (Server) that invoke this binary as a
/// subprocess instead of duplicating a private path dependency on this repo's
/// Python frontend:
///
/// - **Input**: a single positional argument, the agent directory containing
///   `agent.toml` with `[compile].entry` and `[compile].frontend`.
///   `--options-stdin` is required, and stdin must contain exactly one JSON
///   object matching the typed compile-service options contract.
/// - **stdout**: on success, ONLY emitted AIR text. No other text is ever
///   written to stdout; all progress/log/diagnostic output goes to stderr.
/// - **Exit code**: `0` on success. Nonzero on any failure, with a
///   human-readable message on stderr.
#[cfg(feature = "driver")]
pub fn compile_service_command(agent_dir: PathBuf, _config: Option<PathBuf>) -> Result<()> {
    let options = read_compile_service_options_from_stdin()?;
    let air = emit_air_from_agent(&agent_dir, &options)?;

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
pub(crate) fn emit_air_from_agent(
    agent_dir: &Path,
    options: &CompileServiceOptions,
) -> Result<String> {
    if !agent_dir.is_dir() {
        return Err(anyhow::anyhow!(
            "'{}' is not a directory",
            agent_dir.display()
        ));
    }

    options
        .validate()
        .context("compile-service options validation failed")?;
    let entry = declared_agent_package_entry(agent_dir)?.ok_or_else(|| {
        anyhow::anyhow!(
            "{} must declare [compile].entry and [compile].frontend; implicit graph synthesis is no longer supported",
            agent_dir.join("agent.toml").display()
        )
    })?;
    super::agent::verify_agent_integrity(agent_dir)?;

    let (air_path, _tmp, handler_manifest) = prepare_graph_input(&entry, None)?;
    let air = std::fs::read_to_string(&air_path)
        .with_context(|| format!("Failed to read emitted AIR from {}", air_path.display()))?;
    let Some(handler_manifest) = handler_manifest else {
        return Ok(air);
    };
    let manifest = HandlerManifest::from_json_slice(&handler_manifest)
        .context("Failed to parse frontend handler manifest")?;
    manifest
        .validate()
        .context("Invalid frontend handler manifest")?;
    append_handler_manifest_sidecar(air, &manifest)
}

/// The only package-level source declaration accepted by `apxm compile`.
///
/// A package that names executable source must declare both the relative entry
/// path and its frontend under `[compile]`. An entry-less package is compiled
/// by explicit source only; runtime-loop synthesis is not supported.
#[cfg(feature = "driver")]
#[derive(Debug, Deserialize, Default)]
struct AgentPackageToml {
    #[serde(default)]
    compile: Option<CompileToml>,
}

#[cfg(feature = "driver")]
fn declared_agent_package_entry(agent_dir: &Path) -> Result<Option<PathBuf>> {
    let agent_path = agent_dir.join("agent.toml");
    let source: AgentPackageToml = toml::from_str(
        &std::fs::read_to_string(&agent_path)
            .with_context(|| format!("Failed to read {}", agent_path.display()))?,
    )
    .with_context(|| format!("Failed to parse {}", agent_path.display()))?;
    let Some(compile) = source.compile else {
        return Ok(None);
    };

    match (compile.entry, compile.frontend) {
        (None, None) => Ok(None),
        (Some(_), None) => anyhow::bail!(
            "{} declares [compile].entry without [compile].frontend",
            agent_path.display()
        ),
        (None, Some(_)) => anyhow::bail!(
            "{} declares [compile].frontend without [compile].entry; remove [compile] for an entry-less declarative package",
            agent_path.display()
        ),
        (Some(entry), Some(frontend)) => {
            let entry_path = Path::new(&entry);
            if entry_path.is_absolute()
                || entry_path
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                anyhow::bail!(
                    "{} has an invalid [compile].entry {:?}; entries must be workspace-relative package paths",
                    agent_path.display(),
                    entry
                );
            }
            let resolved = agent_dir.join(entry_path);
            if !resolved.is_file() {
                anyhow::bail!(
                    "{} declares [compile].entry {:?}, but {} does not exist",
                    agent_path.display(),
                    entry,
                    resolved.display()
                );
            }
            let is_expected_frontend = match frontend {
                FrontendLanguage::Python => is_python_graph_input(&resolved),
                FrontendLanguage::TypeScript => is_typescript_graph_input(&resolved),
            };
            if !is_expected_frontend {
                anyhow::bail!(
                    "{} declares [compile].frontend = {frontend}, but entry {entry:?} has the wrong source extension",
                    agent_path.display(),
                );
            }
            Ok(Some(resolved))
        }
    }
}

#[cfg(feature = "driver")]
fn append_handler_manifest_sidecar(mut air: String, manifest: &HandlerManifest) -> Result<String> {
    if manifest.handlers.is_empty() {
        return Ok(air);
    }
    let sidecar =
        serde_json::to_string(manifest).context("Failed to serialize handler manifest sidecar")?;
    if !air.ends_with('\n') {
        air.push('\n');
    }
    air.push_str(HANDLER_MANIFEST_AIR_SIDECAR_PREFIX);
    air.push_str(&sidecar);
    Ok(air)
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
    HandlerManifestBytes,
)> {
    if is_python_graph_input(input) {
        let (tmp, manifest) = emit_air_from_python(input, config_path)?;
        return Ok((tmp.path().to_path_buf(), Some(tmp), manifest));
    }

    if is_typescript_graph_input(input) {
        let (tmp, manifest) = emit_air_from_typescript(input)?;
        return Ok((tmp.path().to_path_buf(), Some(tmp), manifest));
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

    Ok((input.to_path_buf(), None, None))
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
) -> Result<()> {
    use apxm_core::constants::diagnostics;
    use apxm_core::constants::session::metrics_keys;
    use apxm_core::types::PipelineConfig;

    let opt = parse_opt_level(opt_level);
    let compiler_config_path = config.clone();
    let opt_target = target;
    let apxm_config = load_config(config.clone())?;
    let analysis_inputs =
        apxm_compiler::CompilerAnalysisInputs::from_backend_configs(&apxm_config.backends);
    let is_agent_package_dir = input.is_dir() && input.join("agent.toml").is_file();
    let declared_package_entry = if is_agent_package_dir {
        declared_agent_package_entry(&input)?
    } else {
        None
    };
    let input_source = if input.is_dir() {
        if let Some(entry) = declared_package_entry {
            entry
        } else if is_agent_package_dir {
            anyhow::bail!(
                "{} must declare [compile].entry and [compile].frontend; implicit graph synthesis is no longer supported",
                input.join("agent.toml").display()
            );
        } else {
            resolve_directory_air_source(&input)?
        }
    } else {
        input.clone()
    };
    let (graph_input, _frontend_air, handler_manifest_data) = if input_source.is_dir() {
        unreachable!("directory inputs are resolved to a canonical .air source before compilation")
    } else {
        prepare_graph_input(&input_source, compiler_config_path.as_deref())?
    };

    let compile_start = std::time::Instant::now();
    let compiler = Compiler::with_opt_level(opt).context("Failed to initialize compiler")?;

    // Check if this is a new-format .air file (valid MLIR).
    let is_new_air = if !input_source.is_dir() && ApxmPathFormat::from_path(&graph_input).is_air_source() {
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
        let (module, mut air_pass_diagnostics) = if needs_diagnostics {
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
        let manifest = handler_manifest_data
            .as_deref()
            .map(|data| {
                let manifest = HandlerManifest::from_json_slice(data)
                    .context("Failed to parse frontend handler manifest")?;
                manifest
                    .validate()
                    .context("Invalid frontend handler manifest")?;
                Ok::<_, anyhow::Error>(manifest)
            })
            .transpose()?;
        let (mut artifact, artifact_stage_metrics) = module
            .generate_artifact_with_manifest_and_caps_with_diagnostics_and_analysis_inputs(
                None,
                manifest.as_ref(),
                &std::collections::HashSet::new(),
                &analysis_inputs,
            )
            .context("Failed to generate artifact")?;

        if let Some(diagnostics) = air_pass_diagnostics.as_mut() {
            diagnostics.record_artifact_stage_metrics(artifact_stage_metrics);
        }

        if let Some(manifest_data) = handler_manifest_data {
            artifact.add_section(apxm_artifact::ArtifactSection {
                kind: apxm_core::types::HANDLER_MANIFEST_ARTIFACT_SECTION.into(),
                data: manifest_data,
            });
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
    use apxm_core::types::{HandlerDescriptor, HandlerKind, HandlerLanguage, HandlerSource};
    use std::fs;
    use tempfile::tempdir;

    fn seal_agent(root: &Path) {
        super::super::agent::seal_agent_integrity_for_test(root).expect("seal test agent");
    }

    fn typescript_hook_manifests(qualnames: &[&str]) -> String {
        let handlers = qualnames
            .iter()
            .enumerate()
            .map(|(index, qualname)| HandlerDescriptor {
                kind: HandlerKind::Hook,
                language: HandlerLanguage::TypeScript,
                handler_id: format!("sha256:{:064x}", index + 1),
                module: "hooks".to_string(),
                qualname: (*qualname).to_string(),
                name: (*qualname).to_string(),
                source: HandlerSource {
                    artifact_path: format!("handlers/{qualname}.mjs"),
                    content: format!("export function {qualname}() {{}}\n"),
                },
                description: String::new(),
                schema: serde_json::json!({}),
                read_only: None,
                requires_approval: None,
                event: Some(
                    if *qualname == "pre_turn" {
                        "pre_turn"
                    } else {
                        "post_turn"
                    }
                    .to_string(),
                ),
                r#match: Some("*".to_string()),
                mode: Some("observe".to_string()),
            })
            .collect();
        serde_json::to_string(&HandlerManifest::new(handlers)).expect("serialize handler manifest")
    }

    fn typescript_hook_manifest(qualname: &str) -> String {
        typescript_hook_manifests(&[qualname])
    }

    fn write_entryless_agent_with_loop_config(mode: &str) -> tempfile::TempDir {
        let tmp = tempdir().expect("temp agent");
        let root = tmp.path();
        fs::create_dir_all(root.join("capabilities/handlers")).expect("handlers dir");
        fs::create_dir_all(root.join("prompts")).expect("prompts dir");
        fs::write(root.join("prompts/persona.md"), "You are concise.\n").expect("persona");
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
persona = "prompts/persona.md"

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
            typescript_hook_manifest("pre_turn"),
        )
        .expect("tools manifest");
        seal_agent(root);
        tmp
    }

    fn write_source_agent(entry: &str, frontend: FrontendLanguage) -> tempfile::TempDir {
        let tmp = tempdir().expect("temp source agent");
        let root = tmp.path();
        let source = root.join(entry);
        fs::create_dir_all(source.parent().expect("entry has parent")).expect("source dir");
        fs::write(
            &source,
            if frontend == FrontendLanguage::Python {
                "from apxm import compile\n"
            } else {
                "console.log('frontend source');\n"
            },
        )
        .expect("source");
        fs::write(
            root.join("agent.toml"),
            format!("[compile]\nentry = {entry:?}\nfrontend = \"{frontend}\"\n"),
        )
        .expect("agent toml");
        tmp
    }

    #[test]
    fn source_bearing_agent_package_routes_through_its_declared_entry() {
        let agent = write_source_agent("python/main.py", FrontendLanguage::Python);
        let expected = agent.path().join("python/main.py");
        assert_eq!(
            declared_agent_package_entry(agent.path())
                .expect("source package resolves")
                .as_deref(),
            Some(expected.as_path())
        );
    }

    #[test]
    fn entryless_package_cannot_retain_a_frontend_without_source() {
        let agent = tempdir().expect("temp declarative agent");
        fs::write(
            agent.path().join("agent.toml"),
            format!(
                "[compile]\nfrontend = \"{}\"\n",
                FrontendLanguage::TypeScript
            ),
        )
        .expect("agent toml");
        let error = declared_agent_package_entry(agent.path())
            .expect_err("entry-less package with frontend must fail");
        assert!(error.to_string().contains("without [compile].entry"));
    }

    #[test]
    fn compile_service_rejects_entryless_packages() {
        let agent_dir = write_entryless_agent_with_loop_config("host");
        let error = emit_air_from_agent(agent_dir.path(), &CompileServiceOptions::default())
            .expect_err("entry-less package must not synthesize AIR");
        assert!(
            error.to_string().contains("[compile].entry"),
            "error should tell the package author to provide explicit frontend source: {error:#}"
        );
    }

    #[test]
    fn compile_service_options_json_requires_exactly_one_typed_object() {
        let options = parse_compile_service_options_json(
            r#"{
                "system_prompt": "Stay concise.",
                "backend": "corp-gateway",
                "model": "gpt-4.1-mini",
                "context_profile": "configured-profile",
                "max_output_tokens": 256,
                "effort": "high",
                "tools": true,
                "skills": false,
                "capability_discovery": true,
                "authoring": false
            }"#,
        )
        .expect("typed options json parses");

        assert_eq!(
            options,
            CompileServiceOptions {
                system_prompt: Some("Stay concise.".to_string()),
                backend: Some("corp-gateway".to_string()),
                model: Some("gpt-4.1-mini".to_string()),
                context_profile: Some("configured-profile".to_string()),
                max_output_tokens: Some(256),
                effort: Some("high".to_string()),
                tools: true,
                skills: false,
                capability_discovery: true,
                authoring: false,
            }
        );

        let err = parse_compile_service_options_json(
            r#"{
                "system_prompt": null,
                "backend": null,
                "model": null,
                "context_profile": null,
                "max_output_tokens": null,
                "effort": null,
                "tools": false,
                "skills": false,
                "capability_discovery": false,
                "authoring": false
            } {}"#,
        )
        .expect_err("multiple JSON values must fail");
        assert!(
            err.to_string().contains("exactly one JSON object"),
            "expected strict single-object error, got {err}"
        );

        let err = parse_compile_service_options_json(
            r#"{
                "backend": null,
                "model": null,
                "effort": null,
                "tools": false,
                "skills": false,
                "capability_discovery": false,
                "authoring": false
            }"#,
        )
        .expect_err("nullable fields must still be present");
        assert!(
            format!("{err:#}").contains("missing field `system_prompt`"),
            "expected required nullable field error, got {err:#}"
        );

        for (field, value) in [
            ("backend", "corp gateway"),
            ("model", "model@preview"),
            ("context_profile", "profile preview"),
            ("effort", "HIGH"),
        ] {
            let json = format!(
                r#"{{
                    "system_prompt": null,
                    "backend": {},
                    "model": {},
                    "context_profile": {},
                    "max_output_tokens": null,
                    "effort": {},
                    "tools": false,
                    "skills": false,
                    "capability_discovery": false,
                    "authoring": false
                }}"#,
                if field == "backend" {
                    serde_json::to_string(value).unwrap()
                } else {
                    "null".to_string()
                },
                if field == "model" {
                    serde_json::to_string(value).unwrap()
                } else {
                    "null".to_string()
                },
                if field == "context_profile" {
                    serde_json::to_string(value).unwrap()
                } else {
                    "null".to_string()
                },
                if field == "effort" {
                    serde_json::to_string(value).unwrap()
                } else {
                    "null".to_string()
                },
            );
            let err = parse_compile_service_options_json(&json)
                .expect_err("invalid option values must fail instead of being sanitized");
            assert!(
                format!("{err:#}").contains(&format!("invalid {field}")),
                "expected exact {field} validation error, got {err:#}"
            );
        }
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
