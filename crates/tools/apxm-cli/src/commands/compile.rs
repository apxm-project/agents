//! Compile and decompile commands.

use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
#[cfg(feature = "driver")]
use apxm_driver::compiler::Compiler;
#[cfg(feature = "driver")]
use apxm_driver::ApXmConfig;

#[cfg(feature = "driver")]
use super::implementations::parse_opt_level;

#[cfg(feature = "driver")]
fn is_python_graph_input(input: &Path) -> bool {
    input.extension().and_then(|ext| ext.to_str()) == Some("py")
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
fn emit_air_from_python(input: &Path) -> Result<(tempfile::NamedTempFile, Option<Vec<u8>>)> {
    use std::io::Write;

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let python_frontend = repo_root.join("crates/compiler/apxm-frontend/python");

    let mut pythonpath_entries = vec![python_frontend, repo_root];
    if let Some(parent) = input.parent() {
        pythonpath_entries.push(parent.to_path_buf());
    }
    if let Some(existing) = env::var_os("PYTHONPATH") {
        pythonpath_entries.extend(env::split_paths(&existing));
    }

    let pythonpath = env::join_paths(pythonpath_entries)
        .context("Failed to build PYTHONPATH for APXM Python frontend")?;
    let mut output = None;
    for candidate in ["python3", "python"] {
        match std::process::Command::new(candidate)
            .arg(input)
            .env("PYTHONPATH", &pythonpath)
            .output()
        {
            Ok(result) => {
                output = Some(result);
                break;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "Failed to run Python workflow {} with {}: {}",
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
            "Python workflow {} failed: {}",
            input.display(),
            stderr.trim()
        ));
    }

    let air = String::from_utf8(output.stdout).with_context(|| {
        format!(
            "Python workflow {} did not emit valid UTF-8",
            input.display()
        )
    })?;
    let trimmed = air.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!(
            "Python workflow {} produced no .air output",
            input.display()
        ));
    }
    if !(trimmed.starts_with(';')
        || trimmed.starts_with('%')
        || trimmed.starts_with("module")
        || trimmed.starts_with("func.func"))
    {
        return Err(anyhow::anyhow!(
            "Python workflow {} did not emit recognizable .air text on stdout.\n\
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
pub(super) fn prepare_graph_input(
    input: &Path,
) -> Result<(PathBuf, Option<tempfile::NamedTempFile>, PythonToolsSidecar)> {
    if is_python_graph_input(input) {
        let (tmp, sidecar) = emit_air_from_python(input)?;
        return Ok((tmp.path().to_path_buf(), Some(tmp), sidecar));
    }

    Ok((input.to_path_buf(), None, None))
}

#[cfg(feature = "driver")]
pub fn compile_command(
    input: PathBuf,
    output: Option<PathBuf>,
    emit_diagnostics: Option<PathBuf>,
    opt_level: u8,
    target: String,
    no_cse_llm: bool,
    profile: Option<PathBuf>,
) -> Result<()> {
    use apxm_core::constants::diagnostics;
    use apxm_core::types::{OptimizationTarget, PipelineConfig};

    let opt = parse_opt_level(opt_level);
    let opt_target: OptimizationTarget = target
        .parse()
        .with_context(|| format!("Invalid optimization target: {}", target))?;
    let (graph_input, _python_air, python_tools_sidecar) = if input.is_dir() {
        (input.clone(), None, None)
    } else {
        prepare_graph_input(&input)?
    };

    let compile_start = std::time::Instant::now();
    let compiler = Compiler::with_opt_level(opt).context("Failed to initialize compiler")?;

    // Check if this is a new-format .air file (valid MLIR)
    let is_new_air = if !input.is_dir()
        && graph_input.extension().and_then(|e| e.to_str()) == Some("air")
    {
        if let Ok(text) = std::fs::read_to_string(&graph_input) {
            text.trim_start().starts_with("module") || text.trim_start().starts_with("func.func")
        } else {
            false
        }
    } else {
        false
    };

    // For new .air format (valid MLIR), compile directly without AirModule
    if is_new_air {
        // NEW PATH: .air file is valid MLIR — compile directly
        // (no-cse-llm and diagnostics are graph-level optimizations, not applicable here)
        let module = compiler
            .compile(&graph_input)
            .map_err(|e| anyhow::anyhow!("Failed to compile MLIR: {e}"))?;
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
            .generate_artifact_with_manifest(
                None,
                manifest.as_deref(),
            )
            .context("Failed to generate artifact")?;

        // Inject python_tools sidecar section if present
        if let Some(sidecar_data) = &python_tools_sidecar {
            artifact.add_section(apxm_artifact::ArtifactSection {
                kind: "python_tools".into(),
                data: sidecar_data.clone(),
            });
        }

        let bytes = artifact
            .to_bytes()
            .map_err(|err| anyhow::anyhow!("Failed to serialize artifact: {err}"))?;
        let artifact_time = artifact_start.elapsed();

        let out_path = output.unwrap_or_else(|| {
            graph_input.with_extension(apxm_core::constants::extensions::ARTIFACT)
        });
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

        return Ok(());
    }

    // OLD PATH: AirModule-based compilation
    let graph = if input.is_dir() {
        load_graph_from_directory(&input)?
    } else {
        compiler
            .load_graph(&graph_input)
            .map_err(|e| anyhow::anyhow!("Failed to parse graph: {e}"))?
    };

    // Validate model allowlist if configured.
    // Skip the check entirely if any registered backend uses the vLLM protocol —
    // vLLM models are user-deployed and not in any builtin allowlist.
    if let Ok(config) = ApXmConfig::load_default() {
        use apxm_core::types::ProviderProtocol;
        use apxm_credentials::BackendStore;
        let has_vllm = BackendStore::open()
            .and_then(|bs| bs.list())
            .map(|backends| backends.iter().any(|b| b.protocol == ProviderProtocol::Vllm))
            .unwrap_or(false);
        if !has_vllm {
            Compiler::validate_model_allowlist(&graph, config.models.allowlist.as_ref())?;
        }
    }

    // When diagnostics are requested, use the per-pass metrics path.
    // Otherwise use the fast bulk-run path.
    let (module, pass_diagnostics) = if emit_diagnostics.is_some() {
        let config = PipelineConfig {
            opt_level: opt,
            target: opt_target,
            verify: true,
            no_cse_llm,
            profile_path: profile.clone(),
            ..Default::default()
        };
        let (m, d) = compiler
            .compile_graph_with_config_and_diagnostics(&graph, config)
            .map_err(|e| anyhow::anyhow!("Failed to compile graph: {e}"))?;
        (m, Some(d))
    } else if no_cse_llm || opt_target != OptimizationTarget::Balanced || profile.is_some() {
        let config = PipelineConfig {
            opt_level: opt,
            target: opt_target,
            verify: true,
            no_cse_llm,
            profile_path: profile.clone(),
            ..Default::default()
        };
        let m = compiler
            .compile_graph_with_config(&graph, config)
            .map_err(|e| anyhow::anyhow!("Failed to compile graph: {e}"))?;
        (m, None)
    } else {
        let m = compiler
            .compile_graph(&graph)
            .map_err(|e| anyhow::anyhow!("Failed to compile graph: {e}"))?;
        (m, None)
    };
    let compile_time = compile_start.elapsed();

    let artifact_start = std::time::Instant::now();
    let bytes = module
        .generate_artifact_bytes()
        .context("Failed to generate artifact")?;
    let artifact_time = artifact_start.elapsed();

    let out_path = output.unwrap_or_else(|| {
        if input.is_dir() {
            input.join(format!(
                "{}.{}",
                graph.name,
                apxm_core::constants::extensions::ARTIFACT
            ))
        } else {
            input.with_extension(apxm_core::constants::extensions::ARTIFACT)
        }
    });
    std::fs::write(&out_path, &bytes)
        .with_context(|| format!("Failed to write {}", out_path.display()))?;

    // Emit diagnostics if requested
    if let Some(diag_path) = emit_diagnostics {
        let artifact = apxm_artifact::Artifact::from_bytes(&bytes)
            .map_err(|e| anyhow::anyhow!("Failed to parse artifact: {}", e))?;
        let dag = artifact
            .dag()
            .ok_or_else(|| anyhow::anyhow!("Artifact contains no DAGs"))?;

        // Build per-pass metrics array from diagnostics
        let per_pass_metrics: Vec<serde_json::Value> = pass_diagnostics
            .as_ref()
            .map(|d| {
                d.passes
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "pass_name": p.pass_name,
                            "duration_ms": p.duration_ms,
                            "ops_before": p.ops_before,
                            "ops_after": p.ops_after,
                            "ops_delta": p.ops_delta
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let diagnostics_json = serde_json::json!({
            "input": input.display().to_string(),
            "mode": diagnostics::MODE_GRAPH,
            "graph_name": graph.name,
            "optimization_level": format!("O{}", opt_level),
            "compilation_phases": {
                "total_ms": compile_time.as_secs_f64() * 1000.0,
                "artifact_gen_ms": artifact_time.as_secs_f64() * 1000.0,
                "passes_ms": pass_diagnostics.as_ref().map(|d| d.total_duration_ms).unwrap_or(0.0)
            },
            "dag_statistics": {
                "total_nodes": dag.nodes.len(),
                "entry_nodes": dag.entry_nodes.len(),
                "exit_nodes": dag.exit_nodes.len(),
                "total_edges": dag.edges.len()
            },
            "pass_metrics": per_pass_metrics,
            "pass_summary": {
                "total_passes": pass_diagnostics.as_ref().map(|d| d.pass_count()).unwrap_or(0),
                "initial_ops": pass_diagnostics.as_ref().map(|d| d.initial_ops).unwrap_or(0),
                "final_ops": pass_diagnostics.as_ref().map(|d| d.final_ops).unwrap_or(0),
                "total_ops_eliminated": pass_diagnostics.as_ref().map(|d| d.total_ops_eliminated()).unwrap_or(0),
                "active_passes": pass_diagnostics.as_ref().map(|d| d.active_passes().iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap_or_default()
            }
        });

        std::fs::write(&diag_path, serde_json::to_string_pretty(&diagnostics_json)?)
            .with_context(|| format!("Failed to write diagnostics to {}", diag_path.display()))?;

        println!("Wrote diagnostics to {}", diag_path.display());
    }

    println!("Wrote graph artifact to {}", out_path.display());
    println!(
        "Compiled in {:.2}ms, artifact generated in {:.2}ms",
        compile_time.as_secs_f64() * 1000.0,
        artifact_time.as_secs_f64() * 1000.0
    );
    Ok(())
}

#[cfg(feature = "driver")]
pub fn decompile_command(artifact_path: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let bytes = std::fs::read(&artifact_path)
        .with_context(|| format!("Failed to read {}", artifact_path.display()))?;
    let artifact = apxm_artifact::Artifact::from_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("Failed to parse artifact: {}", e))?;
    let dag = artifact
        .dag()
        .ok_or_else(|| anyhow::anyhow!("Artifact contains no DAGs"))?;

    let graph = dag_to_graph(dag);
    let json = serde_json::to_string_pretty(&graph)?;

    if let Some(out_path) = output {
        std::fs::write(&out_path, &json)
            .with_context(|| format!("Failed to write {}", out_path.display()))?;
        println!("Decompiled to {}", out_path.display());
    } else {
        println!("{}", json);
    }
    Ok(())
}

#[cfg(feature = "driver")]
fn dag_to_graph(dag: &apxm_core::types::execution::ExecutionDag) -> apxm_compiler::AirModule {
    use apxm_compiler::{AirEdge, AirNode};

    let nodes: Vec<AirNode> = dag
        .nodes
        .iter()
        .map(|n| AirNode {
            id: n.id,
            name: format!("node_{}", n.id),
            op: n.op_type,
            attributes: n.attributes.clone(),
        })
        .collect();

    let edges: Vec<AirEdge> = dag
        .edges
        .iter()
        .map(|e| AirEdge {
            from: e.from,
            to: e.to,
            dependency: e.dependency_type.clone(),
        })
        .collect();

    let parameters: Vec<apxm_compiler::AirParam> = dag
        .metadata
        .parameters
        .iter()
        .map(|p| apxm_compiler::AirParam {
            name: p.name.clone(),
            type_name: p.type_name.clone(),
        })
        .collect();

    apxm_compiler::AirModule {
        name: dag
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "decompiled".to_string()),
        nodes,
        edges,
        parameters,
        metadata: std::collections::HashMap::new(),
    }
}

#[cfg(feature = "driver")]
fn load_graph_from_directory(dir: &std::path::Path) -> Result<apxm_compiler::AirModule> {
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
        for entry in std::fs::read_dir(&search_dir)? {
            let entry = entry?;
            let path = entry.path();
            if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("json") | Some("air")
            ) {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read {}", path.display()))?;
                if text.contains("\"nodes\"") {
                    let graph: apxm_compiler::AirModule =
                        serde_json::from_str(&text).map_err(|e| {
                            anyhow::anyhow!("Failed to parse {}: {}", path.display(), e)
                        })?;
                    graphs.push(graph);
                }
            }
        }
    }

    if graphs.is_empty() {
        return Err(anyhow::anyhow!(
            "No graph files (.air) found in directory '{}'",
            dir.display()
        ));
    }

    if graphs.len() == 1 {
        return Ok(graphs.into_iter().next().unwrap());
    }

    Err(anyhow::anyhow!(
        "Directory '{}' contains {} graph files — multi-graph merge is no longer supported. \
         Provide a single graph file instead.",
        dir.display(),
        graphs.len()
    ))
}

/// Convert an ExecutionDag back to an AirModule for session output.
#[cfg(feature = "driver")]
pub(super) fn graph_from_execution_dag(
    dag: &apxm_core::types::execution::ExecutionDag,
) -> Option<apxm_compiler::AirModule> {
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
            "is_entry".to_string(),
            apxm_core::types::values::Value::Bool(true),
        );
    }

    Some(apxm_compiler::AirModule {
        name: dag
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "artifact".to_string()),
        nodes,
        edges,
        parameters,
        metadata,
    })
}

