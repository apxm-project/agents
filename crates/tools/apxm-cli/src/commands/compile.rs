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
use super::implementations::{load_config, parse_opt_level};

#[cfg(feature = "driver")]
fn is_python_graph_input(input: &Path) -> bool {
    ApxmPathFormat::from_path(input).is_python_frontend()
}

/// Sentinel prefix emitted by the Python frontend in a `;` comment when
/// `@tool`-decorated functions are registered via `Agent`.
#[cfg(feature = "driver")]
const PYTHON_TOOLS_PREFIX: &str = "; __apxm_python_tools__ ";

/// Extract the `; __apxm_python_tools__ <json>` comment from AIR text.
///
/// Returns `(air_without_sidecar, Option<json_bytes>)`.
#[cfg(feature = "driver")]
fn extract_python_tools_sidecar(air: &str) -> (String, Option<Vec<u8>>) {
    let mut sidecar: Option<Vec<u8>> = None;
    let mut filtered = String::with_capacity(air.len());
    for line in air.lines() {
        if let Some(json_str) = line.strip_prefix(PYTHON_TOOLS_PREFIX) {
            sidecar = Some(json_str.as_bytes().to_vec());
        } else {
            if !filtered.is_empty() {
                filtered.push('\n');
            }
            filtered.push_str(line);
        }
    }
    (filtered, sidecar)
}

#[cfg(feature = "driver")]
fn emit_air_from_python(
    input: &Path,
    config_path: Option<&Path>,
) -> Result<(tempfile::NamedTempFile, Option<Vec<u8>>)> {
    use std::io::Write;

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let python_frontend = repo_root.join("crates/compiler/apxm-frontend/python");

    let mut pythonpath_entries = vec![python_frontend, repo_root];
    if let Some(parent) = input.parent() {
        pythonpath_entries.push(parent.to_path_buf());
    }
    if let Some(existing) = env::var_os(apxm_env::PYTHONPATH) {
        pythonpath_entries.extend(env::split_paths(&existing));
    }

    let pythonpath = env::join_paths(pythonpath_entries)
        .context("Failed to build PYTHONPATH for APXM Python frontend")?;
    let mut output = None;
    for candidate in ["python3", "python"] {
        let mut command = std::process::Command::new(candidate);
        command
            .arg(input)
            .env(apxm_env::PYTHONPATH, &pythonpath)
            .env(apxm_env::APXM_EMIT_AIR, apxm_env::flag_values::ENABLED);
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

    // Extract python_tools sidecar before writing AIR to tempfile
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

/// Python tools sidecar data extracted from the AIR comment, if any.
#[cfg(feature = "driver")]
type PythonToolsSidecar = Option<Vec<u8>>;

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
) -> Result<(PathBuf, Option<tempfile::NamedTempFile>, PythonToolsSidecar)> {
    if is_python_graph_input(input) {
        let (tmp, sidecar) = emit_air_from_python(input, config_path)?;
        return Ok((tmp.path().to_path_buf(), Some(tmp), sidecar));
    }

    let input_format = ApxmPathFormat::from_path(input);
    if input_format.is_json_data() {
        return Err(anyhow::anyhow!(
            ".json is structured data for metrics, sessions, manifests, diagnostics, and API envelopes. \
             Graph source input must be .air, or a Python frontend source that emits .air."
        ));
    }

    if input_format.is_air_source() {
        let air = std::fs::read_to_string(input)
            .with_context(|| format!("Failed to read {}", input.display()))?;
        let (clean_air, sidecar) = extract_python_tools_sidecar(&air);
        if sidecar.is_some() {
            use std::io::Write;

            let mut tmp = tempfile::Builder::new()
                .suffix(".air")
                .tempfile()
                .context("Failed to create temporary .air file")?;
            tmp.write_all(clean_air.as_bytes())
                .context("Failed to write stripped .air to temporary file")?;
            tmp.flush()
                .context("Failed to flush stripped .air temporary file")?;
            return Ok((tmp.path().to_path_buf(), Some(tmp), sidecar));
        }
    }

    if !input_format.is_air_source() {
        return Err(anyhow::anyhow!(
            "Unsupported graph input '{}'. Use .air for canonical graph source, .py for a frontend source, or .apxmobj with 'dekk apxm run'.",
            input.display()
        ));
    }

    Ok((input.to_path_buf(), None, None))
}

#[cfg(feature = "driver")]
fn resolve_directory_air_source(dir: &Path) -> Result<PathBuf> {
    let mut graphs = Vec::new();
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
                graphs.push(path);
            }
        }
    }

    match graphs.len() {
        0 => Err(anyhow::anyhow!(
            "No .air graph source found in directory '{}'",
            dir.display()
        )),
        1 => Ok(graphs.remove(0)),
        count => Err(anyhow::anyhow!(
            "Directory '{}' contains {count} .air graph sources. Provide a single .air file instead.",
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
    target: String,
    no_cse_llm: bool,
    profile: Option<PathBuf>,
    warn: bool,
    disable_passes: Vec<String>,
    pass_list_override: Option<Vec<String>>,
    config: Option<PathBuf>,
) -> Result<()> {
    use apxm_core::constants::diagnostics;
    use apxm_core::constants::session::metrics_keys;
    use apxm_core::types::{OptimizationTarget, PipelineConfig};

    let opt = parse_opt_level(opt_level);
    let compiler_config_path = config.clone();
    let opt_target: OptimizationTarget = target
        .parse()
        .with_context(|| format!("Invalid optimization target: {}", target))?;
    let _apxm_config = load_config(config.clone())?;
    let input_source = if input.is_dir() {
        resolve_directory_air_source(&input)?
    } else {
        input.clone()
    };
    let (graph_input, _python_air, python_tools_sidecar) = if input_source.is_dir() {
        unreachable!("directory inputs are resolved to a canonical .air source before compilation")
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
            || opt_target != OptimizationTarget::Balanced
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
        let manifest: Option<Vec<apxm_compiler::passes::PythonToolManifestEntry>> =
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

        let bytes = artifact
            .to_bytes()
            .map_err(|err| anyhow::anyhow!("Failed to serialize artifact: {err}"))?;
        let artifact_time = artifact_start.elapsed();

        let out_path = output.unwrap_or_else(|| graph_input.with_extension(extensions::ARTIFACT));
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
        "Input '{}' is not canonical AIR. Graph source must be .air, or a Python frontend source that emits .air.",
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
}

/// Convert an ExecutionDag back to an AirModule (used by decompile + session output).
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
