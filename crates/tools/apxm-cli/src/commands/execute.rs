//! Execute and run commands.

#[cfg(feature = "driver")]
use std::path::PathBuf;

#[cfg(feature = "driver")]
use anyhow::Context;
use anyhow::Result;
#[cfg(feature = "driver")]
use apxm_driver::{Linker, LinkerConfig};
#[cfg(feature = "driver")]
use apxm_runtime::RuntimeExecutionResult;

#[cfg(feature = "driver")]
use super::compile::{graph_from_execution_dag, prepare_graph_input};
#[cfg(feature = "driver")]
use super::implementations::{load_config, parse_opt_level};

#[cfg(feature = "driver")]
fn load_graph_for_session(input: &std::path::Path) -> Result<apxm_compiler::AirModule> {
    use apxm_driver::compiler::Compiler;

    // Try to load via compiler first so .air gets its dedicated error message.
    if let Ok(compiler) = Compiler::new() {
        if let Ok(graph) = compiler.load_graph(input) {
            return Ok(graph);
        }
    }

    // Fallback: parse as JSON directly when the compiler is unavailable.
    let text = std::fs::read_to_string(input).context("Failed to read graph file")?;
    serde_json::from_str::<apxm_compiler::AirModule>(&text)
        .map_err(|e| anyhow::anyhow!("Failed to parse graph: {}", e))
}

#[cfg(feature = "driver")]
fn setup_session(
    emit_session: &Option<Option<PathBuf>>,
    input: &std::path::Path,
    default_stem: &str,
    input_graph: Option<&apxm_compiler::AirModule>,
    announce: bool,
) -> Result<(
    Option<apxm_driver::session_output::SessionOutputWriter>,
    Option<std::sync::Arc<apxm_driver::session_output::SessionEventEmitter>>,
    Option<String>,
)> {
    use apxm_core::paths::ApxmPaths;
    use apxm_driver::session_output::{SessionEventEmitter, SessionOutputWriter};

    let Some(custom_path) = emit_session else {
        return Ok((None, None, None));
    };

    let base_dir = match custom_path {
        Some(p) => p.clone(),
        None => ApxmPaths::discover()
            .context("Failed to discover APXM paths")?
            .sessions_dir()
            .context("Failed to create sessions directory")?,
    };

    let exec_id = format!(
        "{}-{}",
        input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(default_stem),
        chrono::Utc::now().format("%Y%m%dT%H%M%S")
    );

    let w = SessionOutputWriter::new(&base_dir, &exec_id)
        .context("Failed to create session output directory")?;

    let graph_name = input.file_stem().and_then(|s| s.to_str());
    w.write_manifest(
        &exec_id,
        graph_name,
        apxm_core::types::SessionStatus::Running,
        0,
        0,
        false,
    )
    .context("Failed to write manifest")?;

    if let Some(graph) = input_graph {
        w.write_input_graph(graph)
            .context("Failed to write input graph")?;
    }

    if announce {
        eprintln!("Session: {}", w.session_dir().display());
    }

    let project_root = std::env::current_dir().ok();
    let emitter = std::sync::Arc::new(
        SessionEventEmitter::new(
            w.session_dir(),
            exec_id.clone(),
            input_graph,
            project_root.as_deref(),
        )
        .context("Failed to create session event emitter")?,
    );

    // Set total node count for progress tracking in live.json
    if let Some(graph) = input_graph {
        emitter.set_total_nodes(graph.nodes.len() as u64);
    }

    Ok((Some(w), Some(emitter), Some(exec_id)))
}

#[cfg(feature = "driver")]
fn context_stack_config_from_graph(
    session_dir: &std::path::Path,
    graph: &apxm_compiler::AirModule,
) -> Option<apxm_runtime::context_stack::ContextStackConfig> {
    let node_metadata = graph
        .nodes
        .iter()
        .map(|node| {
            (
                node.id,
                apxm_runtime::context_stack::NodeMetadata {
                    name: node.name.clone(),
                    op_type: format!("{:?}", node.op),
                },
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let graph_edges = graph
        .edges
        .iter()
        .map(|edge| (edge.from, edge.to))
        .collect();

    Some(apxm_runtime::context_stack::ContextStackConfig {
        session_dir: session_dir.to_path_buf(),
        node_metadata,
        graph_edges,
    })
}

/// Extract execution profile from session output for PGO.
///
/// Reads node_statuses.json and input.air to build an ExecutionProfile that maps
/// node names to performance statistics (latency, tokens, error rate).
#[cfg(feature = "driver")]
fn extract_profile_from_session(
    session_dir: &std::path::Path,
    _graph_name: &str,
) -> Result<apxm_compiler::passes::profile::ExecutionProfile> {
    use apxm_compiler::passes::profile::{ExecutionProfile, NodeProfile};
    use serde::Deserialize;
    use std::collections::HashMap;

    // Read node_statuses.json for timing data
    #[derive(Deserialize)]
    struct NodeStatus {
        node_id: u32,
        #[allow(dead_code)]
        status: String,
        duration_ms: u64,
        #[serde(default)]
        retries: u32,
        last_error: Option<String>,
    }

    let statuses_path = session_dir.join("node_statuses.json");
    let statuses_data = std::fs::read_to_string(&statuses_path)
        .with_context(|| format!("Failed to read {}", statuses_path.display()))?;
    let statuses: Vec<NodeStatus> = serde_json::from_str(&statuses_data)
        .with_context(|| "Failed to parse node_statuses.json")?;

    // Read input.air to map node_id → node_name
    // AIR format: %1 = ask(node_1) ...
    let air_path = session_dir.join("input.air");
    let air_content = std::fs::read_to_string(&air_path)
        .with_context(|| format!("Failed to read {}", air_path.display()))?;

    let mut id_to_name: HashMap<u32, String> = HashMap::new();
    for line in air_content.lines() {
        // Parse lines like: "  %1 = ask(node_1) ..."
        if let Some(rest) = line.trim_start().strip_prefix('%') {
            if let Some((id_str, rest)) = rest.split_once('=') {
                let id: u32 = id_str.trim().parse().unwrap_or(0);
                // Extract node name from operation(node_name)
                if let Some(paren_start) = rest.find('(') {
                    if let Some(paren_end) = rest[paren_start + 1..].find(')') {
                        let name = &rest[paren_start + 1..paren_start + 1 + paren_end];
                        id_to_name.insert(id, name.to_string());
                    }
                }
            }
        }
    }

    // Build profile from statuses
    let mut profile = ExecutionProfile::default();
    profile.execution_count = 1;

    for status in statuses {
        let node_name = id_to_name
            .get(&status.node_id)
            .cloned()
            .unwrap_or_else(|| format!("node_{}", status.node_id));

        let error_rate = if status.last_error.is_some() {
            1.0
        } else {
            0.0
        };

        // For now, we don't track tokens per node (would need per-node metrics).
        // Use 0 as placeholder — future enhancement to parse node output for token counts.
        let avg_tokens = 0;

        profile.node_stats.insert(
            node_name,
            NodeProfile {
                avg_latency_ms: status.duration_ms,
                p99_latency_ms: status.duration_ms, // No p99 yet, use avg
                call_count: 1 + status.retries as u64,
                avg_tokens,
                error_rate,
            },
        );
    }

    Ok(profile)
}

#[cfg(feature = "driver")]
pub async fn execute_command(
    input: PathBuf,
    args: Vec<String>,
    opt_level: u8,
    config: Option<PathBuf>,
    json: bool,
    emit_metrics: Option<PathBuf>,
    emit_session: Option<Option<PathBuf>>,
    emit_profile: Option<PathBuf>,
) -> Result<()> {
    let apxm_config = load_config(config).context("Failed to load configuration")?;

    if apxm_config.backends.is_empty() && std::env::var("APXM_MOCK_BACKEND").is_err() {
        anyhow::bail!(
            "No backends configured.\n\n\
             Register a backend before executing this graph:\n\n\
             \x20 dekk apxm backend add openai --type cloud --protocol openai\n\
             \x20 dekk apxm backend add ollama --protocol ollama\n\
             \x20 dekk apxm backend add vllm-fork --type onprem --protocol vllm --endpoint http://127.0.0.1:8916/v1\n\n\
             Verify with: dekk apxm backend list\n\
             If the backend requires authentication, provide it on the backend registration."
        );
    }

    let opt = parse_opt_level(opt_level);
    let mut linker_config = LinkerConfig::from_apxm_config(apxm_config).with_opt_level(opt);
    let (graph_input, _python_air, _python_tools_sidecar) = prepare_graph_input(&input)?;

    // Enable all-outputs collection when session output is requested
    if emit_session.is_some() {
        linker_config
            .runtime_config
            .scheduler_config
            .collect_all_outputs = true;
    }

    // Load input graph for session output
    let input_graph = if emit_session.is_some() {
        load_graph_for_session(&graph_input).ok()
    } else {
        None
    };

    // Set up session output + live emitter BEFORE execution
    let (writer, emitter, execution_id) =
        setup_session(&emit_session, &input, "graph", input_graph.as_ref(), !json)?;

    if let (Some(graph), Some(writer)) = (input_graph.as_ref(), writer.as_ref()) {
        linker_config.runtime_config.context_stack =
            context_stack_config_from_graph(writer.session_dir(), graph);
    }

    let linker = Linker::new(linker_config)
        .await
        .context("Failed to initialize runtime")?;

    // Set memory system on emitter so context assembler can query episodic history
    if let Some(ref e) = emitter {
        e.set_memory(linker.runtime_executor().memory_system());
    }

    // Coerce Arc<SessionEventEmitter> → Arc<dyn ExecutionEventEmitter> for run_graph
    let emitter_dyn: Option<std::sync::Arc<dyn apxm_runtime::ExecutionEventEmitter>> = emitter
        .as_ref()
        .map(|e| e.clone() as std::sync::Arc<dyn apxm_runtime::ExecutionEventEmitter>);

    // Background ticker: updates live.json elapsed_ms every second so it stays
    // current even during long LLM calls between operation events.
    let ticker_handle = emitter.as_ref().map(|e| {
        let e = e.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                e.tick();
            }
        })
    });

    let result = match linker
        .run_graph(
            &graph_input,
            args,
            emitter_dyn,
            writer.as_ref().map(|w| w.session_dir()),
        )
        .await
    {
        Ok(r) => r,
        Err(err) => {
            // Stop the ticker and finalize live.json + manifest as failed.
            if let Some(h) = ticker_handle {
                h.abort();
            }
            if let Some(ref w) = writer {
                let exec_id = execution_id.as_deref();
                let graph_name = input.file_stem().and_then(|s| s.to_str());
                let _ = w.finalize_live_with_id(false, exec_id, graph_name);
            }
            // Also call emitter.finalize_live so elapsed/completed are preserved.
            if let Some(ref e) = emitter {
                let _ = e.finalize_live(false);
            }
            return Err(anyhow::anyhow!("Execution failed: {}", err));
        }
    };

    // Stop the background ticker now that execution is done.
    if let Some(h) = ticker_handle {
        h.abort();
    }

    let metrics_json = build_metrics_json(
        &input,
        Some(opt_level),
        &result.execution,
        #[cfg(feature = "metrics")]
        Some(&result.metrics),
        #[cfg(not(feature = "metrics"))]
        None,
    );

    let written_metrics_path = emit_metrics
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());

    if let Some(metrics_path) = emit_metrics {
        std::fs::write(&metrics_path, serde_json::to_string_pretty(&metrics_json)?)
            .with_context(|| format!("Failed to write metrics to {}", metrics_path.display()))?;
        if !json {
            eprintln!("Wrote metrics to {}", metrics_path.display());
        }
    }

    let session_dir_for_response = writer
        .as_ref()
        .map(|w| w.session_dir().to_string_lossy().to_string());

    let mut written_profile_path: Option<String> = None;

    // Finalize session output after execution
    if let Some(writer) = writer {
        let graph_name = input.file_stem().and_then(|s| s.to_str());
        let exec_id = execution_id.as_deref().unwrap_or("unknown");
        let stats = &result.execution.stats;

        // Query episodic entries for this execution
        let episodic_entries = linker
            .runtime_executor()
            .memory_system()
            .query_episodes(exec_id)
            .await
            .ok();

        writer
            .finalize(
                exec_id,
                graph_name,
                stats.duration_ms,
                stats.executed_nodes + stats.failed_nodes,
                stats.failed_nodes == 0,
                result.execution.all_outputs.as_ref(),
                result.execution.node_output_map.as_ref(),
                &result.execution.results,
                &metrics_json,
                &stats.node_statuses,
                episodic_entries.as_deref(),
            )
            .context("Failed to finalize session")?;

        if !json {
            eprintln!("Session complete: {}", writer.session_dir().display());
        }

        // Extract and emit execution profile if requested
        if let Some(profile_path) = emit_profile {
            let graph_name = input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            match extract_profile_from_session(writer.session_dir(), graph_name) {
                Ok(profile) => {
                    profile.save_to_file(&profile_path).with_context(|| {
                        format!("Failed to save profile to {}", profile_path.display())
                    })?;
                    written_profile_path = Some(profile_path.to_string_lossy().to_string());
                    if !json {
                        eprintln!("Wrote profile to {}", profile_path.display());
                    }
                }
                Err(e) => {
                    if !json {
                        eprintln!("Warning: Failed to extract profile: {}", e);
                    }
                }
            }
        }
    }

    if json {
        let response = build_execution_response(
            &result.execution,
            execution_id.clone(),
            session_dir_for_response,
            written_metrics_path,
            written_profile_path,
        );
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else if result.execution.results.is_empty() {
        eprintln!("Warning: No output values");
    } else {
        print_result_values(&result.execution.results);
    }

    // Gracefully shutdown runtime to close all agent processes
    linker.shutdown();

    Ok(())
}

#[cfg(feature = "driver")]
pub async fn run_command(
    input: PathBuf,
    args: Vec<String>,
    config: Option<PathBuf>,
    json: bool,
    emit_metrics: Option<PathBuf>,
    emit_session: Option<Option<PathBuf>>,
    emit_profile: Option<PathBuf>,
) -> Result<()> {
    use apxm_artifact::Artifact;
    use apxm_driver::runtime::RuntimeExecutor;

    // Validate file extension
    if input.extension().and_then(|e| e.to_str())
        != Some(apxm_core::constants::extensions::ARTIFACT)
    {
        return Err(anyhow::anyhow!(
            "Expected .apxmobj artifact file. Use 'execute' command for graph source files."
        ));
    }

    // Load artifact
    let artifact_bytes = std::fs::read(&input)
        .with_context(|| format!("Failed to read artifact {}", input.display()))?;
    let artifact = Artifact::from_bytes(&artifact_bytes)
        .map_err(|e| anyhow::anyhow!("Failed to parse artifact: {}", e))?;

    // Initialize runtime
    let apxm_config = load_config(config).context("Failed to load configuration")?;

    if apxm_config.backends.is_empty() && std::env::var("APXM_MOCK_BACKEND").is_err() {
        anyhow::bail!(
            "No backends configured.\n\n\
             Register a backend before running this artifact:\n\n\
             \x20 dekk apxm backend add openai --type cloud --protocol openai\n\
             \x20 dekk apxm backend add ollama --protocol ollama\n\
             \x20 dekk apxm backend add vllm-fork --type onprem --protocol vllm --endpoint http://127.0.0.1:8916/v1\n\n\
             Verify with: dekk apxm backend list\n\
             If the backend requires authentication, provide it on the backend registration."
        );
    }

    let mut linker_config = LinkerConfig::from_apxm_config(apxm_config);

    // Enable all-outputs collection when session output is requested
    if emit_session.is_some() {
        linker_config
            .runtime_config
            .scheduler_config
            .collect_all_outputs = true;
    }

    let artifact_graph = if emit_session.is_some() {
        artifact.entry_dag().map(graph_from_execution_dag)
    } else {
        None
    };

    // Set up session output + live emitter BEFORE execution
    let (writer, emitter, execution_id) = setup_session(
        &emit_session,
        &input,
        "artifact",
        artifact_graph.as_ref(),
        !json,
    )?;

    if let (Some(graph), Some(writer)) = (artifact_graph.as_ref(), writer.as_ref()) {
        linker_config.runtime_config.context_stack =
            context_stack_config_from_graph(writer.session_dir(), graph);
    }

    let runtime = RuntimeExecutor::new(&linker_config)
        .await
        .context("Failed to initialize runtime")?;

    // Coerce Arc<SessionEventEmitter> → Arc<dyn ExecutionEventEmitter>
    let emitter_dyn: Option<std::sync::Arc<dyn apxm_runtime::ExecutionEventEmitter>> = emitter
        .as_ref()
        .map(|e| e.clone() as std::sync::Arc<dyn apxm_runtime::ExecutionEventEmitter>);

    // Execute artifact with args + emitter
    let result = match runtime
        .execute_artifact_with_emitter(
            artifact,
            args,
            emitter_dyn,
            writer
                .as_ref()
                .map(|w| w.session_dir().to_string_lossy().to_string()),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            // Finalize live.json + manifest as failed.
            if let Some(ref w) = writer {
                let exec_id = execution_id.as_deref();
                let graph_name = input.file_stem().and_then(|s| s.to_str());
                let _ = w.finalize_live_with_id(false, exec_id, graph_name);
            }
            return Err(anyhow::anyhow!("Execution failed: {}", e));
        }
    };

    let metrics_json = build_metrics_json(
        &input,
        None,
        &result,
        #[cfg(feature = "metrics")]
        None,
        #[cfg(not(feature = "metrics"))]
        None,
    );

    let written_metrics_path = emit_metrics
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());

    // Emit metrics JSON if requested
    if let Some(metrics_path) = emit_metrics {
        std::fs::write(&metrics_path, serde_json::to_string_pretty(&metrics_json)?)
            .with_context(|| format!("Failed to write metrics to {}", metrics_path.display()))?;
        if !json {
            eprintln!("Wrote metrics to {}", metrics_path.display());
        }
    }

    let session_dir_for_response = writer
        .as_ref()
        .map(|w| w.session_dir().to_string_lossy().to_string());

    let mut written_profile_path: Option<String> = None;

    // Finalize session output after execution
    if let Some(writer) = writer {
        let graph_name = input.file_stem().and_then(|s| s.to_str());
        let exec_id = execution_id.as_deref().unwrap_or("unknown");
        let stats = &result.stats;

        // Query episodic entries for this execution
        let episodic_entries = runtime.memory_system().query_episodes(exec_id).await.ok();

        writer
            .finalize(
                exec_id,
                graph_name,
                stats.duration_ms,
                stats.executed_nodes + stats.failed_nodes,
                stats.failed_nodes == 0,
                result.all_outputs.as_ref(),
                result.node_output_map.as_ref(),
                &result.results,
                &metrics_json,
                &stats.node_statuses,
                episodic_entries.as_deref(),
            )
            .context("Failed to finalize session")?;

        if !json {
            eprintln!("Session complete: {}", writer.session_dir().display());
        }

        // Extract and emit execution profile if requested
        if let Some(profile_path) = emit_profile {
            let graph_name = input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            match extract_profile_from_session(writer.session_dir(), graph_name) {
                Ok(profile) => {
                    profile.save_to_file(&profile_path).with_context(|| {
                        format!("Failed to save profile to {}", profile_path.display())
                    })?;
                    written_profile_path = Some(profile_path.to_string_lossy().to_string());
                    if !json {
                        eprintln!("Wrote profile to {}", profile_path.display());
                    }
                }
                Err(e) => {
                    if !json {
                        eprintln!("Warning: Failed to extract profile: {}", e);
                    }
                }
            }
        }
    }

    if json {
        let response = build_execution_response(
            &result,
            execution_id.clone(),
            session_dir_for_response,
            written_metrics_path,
            written_profile_path,
        );
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else if result.results.is_empty() {
        eprintln!("Warning: No output values");
    } else {
        print_result_values(&result.results);
    }

    // Gracefully shutdown runtime to close all agent processes
    runtime.shutdown();

    Ok(())
}

#[cfg(feature = "driver")]
fn print_result_values(results: &std::collections::HashMap<u64, apxm_core::types::Value>) {
    for (key, value) in results {
        if matches!(value, apxm_core::types::values::Value::Null) {
            continue;
        }

        let rendered = value_rendered_text(value);

        if results.len() == 1 {
            println!("{}", rendered);
        } else {
            println!("{}={}", key, rendered);
        }
    }
}

#[cfg(feature = "driver")]
fn value_rendered_text(value: &apxm_core::types::Value) -> String {
    match value {
        apxm_core::types::values::Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

#[cfg(feature = "driver")]
fn best_result_content(
    results: &std::collections::HashMap<u64, apxm_core::types::Value>,
) -> Option<String> {
    let mut best: Option<(u64, String)> = None;
    for (key, value) in results {
        if matches!(value, apxm_core::types::values::Value::Null) {
            continue;
        }
        let candidate = value_rendered_text(value);
        match best {
            Some((best_key, _)) if *key <= best_key => {}
            _ => best = Some((*key, candidate)),
        }
    }
    best.map(|(_, value)| value)
}

#[cfg(feature = "driver")]
fn build_execution_response(
    result: &RuntimeExecutionResult,
    execution_id: Option<String>,
    session_dir: Option<String>,
    metrics_path: Option<String>,
    profile_path: Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "content": best_result_content(&result.results),
        "execution_id": execution_id,
        "session_dir": session_dir,
        "metrics_path": metrics_path,
        "profile_path": profile_path,
        "results": result.results,
        "stats": {
            "executed_nodes": result.stats.executed_nodes,
            "failed_nodes": result.stats.failed_nodes,
            "duration_ms": result.stats.duration_ms,
        },
        "llm_usage": llm_usage_json(result),
    })
}

#[cfg(feature = "driver")]
fn llm_usage_json(_result: &RuntimeExecutionResult) -> serde_json::Value {
    #[cfg(feature = "metrics")]
    {
        return serde_json::json!({
            "input_tokens": _result.llm_metrics.total_input_tokens,
            "output_tokens": _result.llm_metrics.total_output_tokens,
            "total_requests": _result.llm_metrics.total_requests,
        });
    }

    #[cfg(not(feature = "metrics"))]
    {
        serde_json::json!({
            "input_tokens": 0,
            "output_tokens": 0,
            "total_requests": 0,
        })
    }
}

#[cfg(feature = "driver")]
fn build_metrics_json(
    input: &std::path::Path,
    opt_level: Option<u8>,
    result: &RuntimeExecutionResult,
    #[cfg(feature = "metrics")] link_metrics: Option<&apxm_driver::linker::LinkMetrics>,
    #[cfg(not(feature = "metrics"))] _link_metrics: Option<()>,
) -> serde_json::Value {
    let mut metrics_json = serde_json::json!({
        "input": input.display().to_string(),
        "optimization_level": opt_level.map(|level| format!("O{}", level)),
        "execution": {
            "nodes_executed": result.stats.executed_nodes,
            "nodes_failed": result.stats.failed_nodes,
            "duration_ms": result.stats.duration_ms,
            "status": if result.stats.failed_nodes == 0 { "success" } else { "partial_failure" }
        },
        "scheduler": result.scheduler_metrics.to_json(),
        "llm": llm_usage_json(result),
        "link_phases": serde_json::Value::Null,
    });

    let token_json = result.token_snapshot.to_json();
    if let Some(obj) = token_json.get("token_accounting").cloned() {
        metrics_json["token_accounting"] = obj;
    }

    #[cfg(feature = "metrics")]
    {
        let llm_metrics = &result.llm_metrics;
        metrics_json["llm"] = serde_json::json!({
            "total_requests": llm_metrics.total_requests,
            "total_input_tokens": llm_metrics.total_input_tokens,
            "total_output_tokens": llm_metrics.total_output_tokens,
            "avg_latency_ms": llm_metrics.average_latency.as_millis(),
            "p50_latency_ms": llm_metrics.p50_latency.as_millis(),
            "p99_latency_ms": llm_metrics.p99_latency.as_millis()
        });

        if let Some(link_metrics) = link_metrics {
            metrics_json["link_phases"] = serde_json::json!({
                "compile_ms": link_metrics.compile_time.as_secs_f64() * 1000.0,
                "runtime_ms": link_metrics.runtime_time.as_secs_f64() * 1000.0
            });
        }
    }

    metrics_json
}

#[cfg(all(test, feature = "driver"))]
mod tests {
    use super::{build_execution_response, build_metrics_json};
    use apxm_core::types::{execution::ExecutionStats, values::Value};
    use apxm_runtime::{
        RuntimeExecutionResult, SchedulerMetrics,
        executor::token_accounting::{TokenAccountingSnapshot, TokenUsageSummary},
    };
    use std::collections::HashMap;
    use std::path::Path;

    fn sample_result() -> RuntimeExecutionResult {
        let mut results = HashMap::new();
        results.insert(7, Value::String("done".to_string()));

        RuntimeExecutionResult {
            results,
            stats: ExecutionStats {
                executed_nodes: 3,
                failed_nodes: 0,
                duration_ms: 42,
                node_statuses: vec![],
            },
            #[cfg(feature = "metrics")]
            llm_metrics: apxm_backends::AggregatedMetrics::default(),
            scheduler_metrics: SchedulerMetrics::default(),
            all_outputs: None,
            node_output_map: None,
            token_snapshot: TokenAccountingSnapshot {
                per_node: HashMap::new(),
                per_flow: HashMap::new(),
                per_agent: HashMap::new(),
                total: TokenUsageSummary::default(),
            },
        }
    }

    #[test]
    fn execution_response_includes_machine_paths_and_ids() {
        let response = build_execution_response(
            &sample_result(),
            Some("exec-123".to_string()),
            Some("/tmp/session".to_string()),
            Some("/tmp/metrics.json".to_string()),
            Some("/tmp/profile.json".to_string()),
        );

        assert_eq!(response["execution_id"], "exec-123");
        assert_eq!(response["session_dir"], "/tmp/session");
        assert_eq!(response["metrics_path"], "/tmp/metrics.json");
        assert_eq!(response["profile_path"], "/tmp/profile.json");
        assert_eq!(response["content"], "done");
        assert_eq!(response["stats"]["executed_nodes"], 3);
    }

    #[test]
    fn metrics_json_keeps_shared_schema_for_run_and_execute() {
        let metrics = build_metrics_json(Path::new("demo.air"), None, &sample_result(), None);
        assert_eq!(metrics["input"], "demo.air");
        assert!(metrics.get("optimization_level").is_some());
        assert!(metrics.get("llm").is_some());
        assert!(metrics.get("token_accounting").is_some());
        assert!(metrics.get("link_phases").is_some());
    }
}
