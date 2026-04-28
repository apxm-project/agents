//! Execute and run commands.

#[cfg(feature = "driver")]
use std::path::PathBuf;

#[cfg(feature = "driver")]
use anyhow::Context;
use anyhow::Result;
#[cfg(feature = "driver")]
use apxm_core::constants::env as apxm_env;
#[cfg(feature = "driver")]
use apxm_core::types::OptimizationTarget;
#[cfg(feature = "driver")]
use apxm_driver::{Linker, LinkerConfig};
#[cfg(feature = "driver")]
use apxm_runtime::RuntimeExecutionResult;

#[cfg(feature = "driver")]
use super::compile::{air_graph_from_source, graph_from_execution_dag, prepare_graph_input};
#[cfg(feature = "driver")]
use super::dekk_hints;
#[cfg(feature = "driver")]
use super::implementations::{load_config, parse_opt_level};

#[cfg(feature = "driver")]
fn load_graph_for_session(input: &std::path::Path) -> Result<apxm_compiler::AirModule> {
    air_graph_from_source(input)
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
                    op_type: node.op,
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
    target: OptimizationTarget,
    config: Option<PathBuf>,
    json: bool,
    emit_metrics: Option<PathBuf>,
    emit_metrics_level: apxm_core::types::MetricsLevel,
    emit_session: Option<Option<PathBuf>>,
    emit_profile: Option<PathBuf>,
) -> Result<()> {
    let apxm_config = load_config(config.clone()).context("Failed to load configuration")?;

    if apxm_config.backends.is_empty() && std::env::var(apxm_env::APXM_MOCK_BACKEND).is_err() {
        anyhow::bail!(
            "No backends configured.\n\n\
             Register a backend before executing this graph:\n\n\
             \x20 {}\n\
             \x20 {}\n\
             \x20 {}\n\n\
             Verify with: {}\n\
             If the backend requires authentication, provide it on the backend registration.",
            dekk_hints::BACKEND_ADD_OPENAI,
            dekk_hints::BACKEND_ADD_OLLAMA,
            dekk_hints::VLLM_ENABLE_SERVED_MODEL,
            dekk_hints::BACKEND_LIST
        );
    }

    let opt = parse_opt_level(opt_level);
    let pipeline_config = apxm_core::types::PipelineConfig {
        opt_level: opt,
        target,
        compiler_config_path: config.clone(),
        ..Default::default()
    };
    let mut linker_config =
        LinkerConfig::from_apxm_config(apxm_config).with_pipeline_config(pipeline_config);
    linker_config.runtime_config.metrics_level = emit_metrics_level;
    let (graph_input, _python_air, python_tools_sidecar) =
        prepare_graph_input(&input, config.as_deref())?;

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
        .run_graph_with_python_tools_sidecar(
            &graph_input,
            args,
            emitter_dyn,
            writer.as_ref().map(|w| w.session_dir()),
            python_tools_sidecar,
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

    if !json {
        print_execution_summary(&result.execution);
    }

    let metrics_json = build_metrics_json(
        &input,
        Some(opt_level),
        &result.execution,
        result.compiler_diagnostics.as_ref(),
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
    target: apxm_core::types::OptimizationTarget,
    config: Option<PathBuf>,
    json: bool,
    emit_metrics: Option<PathBuf>,
    emit_metrics_level: apxm_core::types::MetricsLevel,
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

    if apxm_config.backends.is_empty() && std::env::var(apxm_env::APXM_MOCK_BACKEND).is_err() {
        anyhow::bail!(
            "No backends configured.\n\n\
             Register a backend before running this artifact:\n\n\
             \x20 {}\n\
             \x20 {}\n\
             \x20 {}\n\n\
             Verify with: {}\n\
             If the backend requires authentication, provide it on the backend registration.",
            dekk_hints::BACKEND_ADD_OPENAI,
            dekk_hints::BACKEND_ADD_OLLAMA,
            dekk_hints::VLLM_ENABLE_SERVED_MODEL,
            dekk_hints::BACKEND_LIST
        );
    }

    let mut linker_config = LinkerConfig::from_apxm_config(apxm_config);
    linker_config.runtime_config.optimization_target = target;
    linker_config.runtime_config.metrics_level = emit_metrics_level;

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
        None,
        #[cfg(feature = "metrics")]
        None,
        #[cfg(not(feature = "metrics"))]
        None,
    );

    if !json {
        print_execution_summary(&result);
    }

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionSummaryStatus {
    Success,
    PartialFailure,
}

#[cfg(feature = "driver")]
mod execution_summary_labels {
    pub const PREFIX: &str = "APXM execution summary";
    pub const STATUS: &str = "status";
    pub const NODES: &str = "nodes";
    pub const DURATION_MS: &str = "duration_ms";
    pub const LLM_CALLS: &str = "llm_calls";
    pub const TOKENS: &str = "tokens";
    pub const INPUT_TOKENS: &str = "in";
    pub const OUTPUT_TOKENS: &str = "out";
    pub const CACHED_INPUT_TOKENS: &str = "cached_in";
    pub const REASONING_OUTPUT_TOKENS: &str = "reasoning_out";
    pub const BACKEND_GRAPHS: &str = "backend_graphs";
}

#[cfg(feature = "driver")]
impl ExecutionSummaryStatus {
    fn from_failed_nodes(failed_nodes: usize) -> Self {
        if failed_nodes == 0 {
            Self::Success
        } else {
            Self::PartialFailure
        }
    }

    fn label(self) -> &'static str {
        use apxm_core::constants::session::metrics_keys::execution_keys;
        match self {
            Self::Success => execution_keys::STATUS_SUCCESS,
            Self::PartialFailure => execution_keys::STATUS_PARTIAL_FAILURE,
        }
    }
}

#[cfg(feature = "driver")]
#[derive(Debug, Clone)]
struct ExecutionSummary {
    status: ExecutionSummaryStatus,
    nodes_executed: usize,
    nodes_failed: usize,
    duration_ms: u128,
    llm_calls: usize,
    input_tokens: usize,
    output_tokens: usize,
    total_tokens: usize,
    cached_input_tokens: usize,
    reasoning_output_tokens: usize,
    backend_graphs: usize,
}

#[cfg(feature = "driver")]
impl ExecutionSummary {
    fn from_result(result: &RuntimeExecutionResult) -> Self {
        let total = &result.token_snapshot.total;
        Self {
            status: ExecutionSummaryStatus::from_failed_nodes(result.stats.failed_nodes),
            nodes_executed: result.stats.executed_nodes,
            nodes_failed: result.stats.failed_nodes,
            duration_ms: result.stats.duration_ms,
            llm_calls: total.call_count,
            input_tokens: total.input_tokens,
            output_tokens: total.output_tokens,
            total_tokens: total.total_tokens,
            cached_input_tokens: total.cached_input_tokens,
            reasoning_output_tokens: total.reasoning_output_tokens,
            backend_graphs: result.graph_status_snapshots.len(),
        }
    }
}

#[cfg(feature = "driver")]
fn print_execution_summary(result: &RuntimeExecutionResult) {
    use execution_summary_labels as labels;

    let summary = ExecutionSummary::from_result(result);
    eprintln!(
        "{}: {}={}, {}={}/{}, {}={}, {}={}, {}={} ({}={}, {}={}, {}={}, {}={}), {}={}",
        labels::PREFIX,
        labels::STATUS,
        summary.status.label(),
        labels::NODES,
        summary.nodes_executed,
        summary.nodes_failed,
        labels::DURATION_MS,
        summary.duration_ms,
        labels::LLM_CALLS,
        summary.llm_calls,
        labels::TOKENS,
        summary.total_tokens,
        labels::INPUT_TOKENS,
        summary.input_tokens,
        labels::OUTPUT_TOKENS,
        summary.output_tokens,
        labels::CACHED_INPUT_TOKENS,
        summary.cached_input_tokens,
        labels::REASONING_OUTPUT_TOKENS,
        summary.reasoning_output_tokens,
        labels::BACKEND_GRAPHS,
        summary.backend_graphs
    );
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
fn llm_usage_json(_result: &RuntimeExecutionResult) -> serde_json::Value {
    use apxm_core::constants::session::metrics_keys::llm_keys;
    let mut map = serde_json::Map::new();
    #[cfg(feature = "metrics")]
    {
        map.insert(
            llm_keys::INPUT_TOKENS.to_owned(),
            _result.llm_metrics.total_input_tokens.into(),
        );
        map.insert(
            llm_keys::OUTPUT_TOKENS.to_owned(),
            _result.llm_metrics.total_output_tokens.into(),
        );
        map.insert(
            llm_keys::TOTAL_REQUESTS.to_owned(),
            _result.llm_metrics.total_requests.into(),
        );
    }
    #[cfg(not(feature = "metrics"))]
    {
        map.insert(llm_keys::INPUT_TOKENS.to_owned(), 0.into());
        map.insert(llm_keys::OUTPUT_TOKENS.to_owned(), 0.into());
        map.insert(llm_keys::TOTAL_REQUESTS.to_owned(), 0.into());
    }
    serde_json::Value::Object(map)
}

#[cfg(feature = "driver")]
fn build_execution_response(
    result: &RuntimeExecutionResult,
    execution_id: Option<String>,
    session_dir: Option<String>,
    metrics_path: Option<String>,
    profile_path: Option<String>,
) -> serde_json::Value {
    use apxm_core::constants::session::metrics_keys::{cli_response_keys as ck, execution_keys};

    let mut stats_map = serde_json::Map::new();
    stats_map.insert(
        ck::STATS_EXECUTED_NODES.to_owned(),
        result.stats.executed_nodes.into(),
    );
    stats_map.insert(
        ck::STATS_FAILED_NODES.to_owned(),
        result.stats.failed_nodes.into(),
    );
    stats_map.insert(
        execution_keys::DURATION_MS.to_owned(),
        (result.stats.duration_ms as u64).into(),
    );

    let mut response = serde_json::Map::new();
    response.insert(
        ck::CONTENT.to_owned(),
        best_result_content(&result.results)
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    );
    response.insert(
        ck::EXECUTION_ID.to_owned(),
        execution_id
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    );
    response.insert(
        ck::SESSION_DIR.to_owned(),
        session_dir
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    );
    response.insert(
        ck::METRICS_PATH.to_owned(),
        metrics_path
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    );
    response.insert(
        ck::PROFILE_PATH.to_owned(),
        profile_path
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    );
    response.insert(
        ck::RESULTS.to_owned(),
        serde_json::to_value(&result.results).unwrap_or(serde_json::Value::Null),
    );
    response.insert(ck::STATS.to_owned(), serde_json::Value::Object(stats_map));
    response.insert(ck::LLM_USAGE.to_owned(), llm_usage_json(result));
    serde_json::Value::Object(response)
}

/// Runtime metrics source for the unified `MetricsReport`.
#[cfg(feature = "driver")]
struct RuntimeMetricsSource<'a> {
    execution: &'a RuntimeExecutionResult,
    input: &'a std::path::Path,
    opt_level: Option<u8>,
    #[cfg(feature = "metrics")]
    link_metrics: Option<&'a apxm_driver::linker::LinkMetrics>,
}

#[cfg(feature = "driver")]
impl apxm_core::MetricsSource for RuntimeMetricsSource<'_> {
    fn section_name(&self) -> &'static str {
        apxm_core::constants::session::metrics_keys::SECTION_RUNTIME
    }

    fn collect(&self) -> serde_json::Value {
        use apxm_core::constants::session::metrics_keys;
        use metrics_keys::{execution_keys, runtime_meta_keys};
        #[cfg(feature = "metrics")]
        use metrics_keys::link_phase_keys;

        let mut map = serde_json::Map::new();

        let mut exec = serde_json::Map::new();
        exec.insert(
            execution_keys::NODES_EXECUTED.to_owned(),
            self.execution.stats.executed_nodes.into(),
        );
        exec.insert(
            execution_keys::NODES_FAILED.to_owned(),
            self.execution.stats.failed_nodes.into(),
        );
        exec.insert(
            execution_keys::DURATION_MS.to_owned(),
            (self.execution.stats.duration_ms as u64).into(),
        );
        let status =
            ExecutionSummaryStatus::from_failed_nodes(self.execution.stats.failed_nodes).label();
        exec.insert(
            execution_keys::STATUS.to_owned(),
            serde_json::Value::String(status.to_owned()),
        );
        map.insert(
            metrics_keys::RUNTIME_EXECUTION.to_owned(),
            serde_json::Value::Object(exec),
        );

        map.insert(
            metrics_keys::RUNTIME_SCHEDULER.to_owned(),
            self.execution.scheduler_metrics.to_json(),
        );
        let token_json = self.execution.token_snapshot.to_json();
        if let Some(obj) = token_json.get(metrics_keys::TOKEN_ACCOUNTING).cloned() {
            map.insert(metrics_keys::TOKEN_ACCOUNTING.to_owned(), obj);
        }
        let graph_metrics_json = self.execution.graph_metrics_snapshot.to_json();
        if let Some(obj) = graph_metrics_json
            .get(metrics_keys::RUNTIME_GRAPH_METRICS)
            .cloned()
        {
            map.insert(metrics_keys::RUNTIME_GRAPH_METRICS.to_owned(), obj);
        }
        if let Some(observed) = &self.execution.stats.observed_graph {
            map.insert(
                metrics_keys::RUNTIME_OBSERVED_GRAPH.to_owned(),
                serde_json::to_value(observed).unwrap_or(serde_json::Value::Null),
            );
        }
        #[cfg(feature = "metrics")]
        {
            map.insert(
                metrics_keys::RUNTIME_LLM.to_owned(),
                self.execution.llm_metrics.to_metrics_json(),
            );

            if let Some(link_metrics) = self.link_metrics {
                let mut link = serde_json::Map::new();
                link.insert(
                    link_phase_keys::COMPILE_MS.to_owned(),
                    (link_metrics.compile_time.as_secs_f64() * 1000.0).into(),
                );
                link.insert(
                    link_phase_keys::RUNTIME_MS.to_owned(),
                    (link_metrics.runtime_time.as_secs_f64() * 1000.0).into(),
                );
                map.insert(
                    link_phase_keys::LINK_PHASES.to_owned(),
                    serde_json::Value::Object(link),
                );
            }
        }
        map.insert(
            runtime_meta_keys::INPUT.to_owned(),
            serde_json::Value::String(self.input.display().to_string()),
        );
        if let Some(level) = self.opt_level {
            map.insert(
                runtime_meta_keys::OPTIMIZATION_LEVEL.to_owned(),
                serde_json::Value::String(format!("O{}", level)),
            );
        }
        serde_json::Value::Object(map)
    }
}

#[cfg(feature = "driver")]
fn build_metrics_json(
    input: &std::path::Path,
    opt_level: Option<u8>,
    result: &RuntimeExecutionResult,
    compiler_diagnostics: Option<&serde_json::Value>,
    #[cfg(feature = "metrics")] link_metrics: Option<&apxm_driver::linker::LinkMetrics>,
    #[cfg(not(feature = "metrics"))] _link_metrics: Option<()>,
) -> serde_json::Value {
    let mut report = apxm_core::MetricsReport::new();

    // Compiler diagnostics section (absent for artifact-only `run` path).
    if let Some(diag_json) = compiler_diagnostics {
        struct PrebuiltCompilerSource<'a>(&'a serde_json::Value);
        impl apxm_core::MetricsSource for PrebuiltCompilerSource<'_> {
            fn section_name(&self) -> &'static str {
                apxm_core::constants::session::metrics_keys::SECTION_COMPILER
            }
            fn collect(&self) -> serde_json::Value {
                self.0.clone()
            }
        }
        report.add_source(&PrebuiltCompilerSource(diag_json));
    }

    report.add_source(&RuntimeMetricsSource {
        execution: result,
        input,
        opt_level,
        #[cfg(feature = "metrics")]
        link_metrics,
    });

    #[cfg(feature = "metrics")]
    let backend_aggregate = result.llm_metrics.clone();
    #[cfg(not(feature = "metrics"))]
    let backend_aggregate = apxm_backends::AggregatedMetrics::default();
    if backend_aggregate.total_requests > 0 || !result.graph_status_snapshots.is_empty() {
        report.add_source(&apxm_backends::BackendMetricsSource {
            aggregate: backend_aggregate,
            per_backend: std::collections::HashMap::new(),
            graph_status_snapshots: result.graph_status_snapshots.clone(),
        });
    }

    report.to_json()
}

#[cfg(all(test, feature = "driver"))]
mod tests {
    use super::build_execution_response;
    use apxm_core::constants::session::metrics_keys;
    use apxm_core::types::{
        GraphMetricsSnapshot, GraphStatusSnapshot, execution::ExecutionStats, values::Value,
    };
    use apxm_runtime::{
        RuntimeExecutionResult, SchedulerMetrics,
        executor::token_accounting::{TokenAccountingSnapshot, TokenUsageSummary},
    };
    use std::collections::HashMap;
    use std::path::Path;

    const TEST_EXECUTION_ID: &str = "exec-123";
    const TEST_INPUT: &str = "demo.air";
    const TEST_SESSION_DIR: &str = "/tmp/session";
    const TEST_METRICS_PATH: &str = "/tmp/metrics.json";
    const TEST_PROFILE_PATH: &str = "/tmp/profile.json";

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
                observed_graph: None,
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
            graph_metrics_snapshot: GraphMetricsSnapshot::default(),
            graph_status_snapshots: vec![],
        }
    }

    #[test]
    fn execution_response_includes_machine_paths_and_ids() {
        let response = build_execution_response(
            &sample_result(),
            Some(TEST_EXECUTION_ID.to_string()),
            Some(TEST_SESSION_DIR.to_string()),
            Some(TEST_METRICS_PATH.to_string()),
            Some(TEST_PROFILE_PATH.to_string()),
        );

        use metrics_keys::cli_response_keys as response_keys;
        assert_eq!(response[response_keys::EXECUTION_ID], TEST_EXECUTION_ID);
        assert_eq!(response[response_keys::SESSION_DIR], TEST_SESSION_DIR);
        assert_eq!(response[response_keys::METRICS_PATH], TEST_METRICS_PATH);
        assert_eq!(response[response_keys::PROFILE_PATH], TEST_PROFILE_PATH);
        assert_eq!(response[response_keys::CONTENT], "done");
        assert_eq!(
            response[response_keys::STATS][response_keys::STATS_EXECUTED_NODES],
            3
        );
    }

    #[test]
    fn metrics_json_uses_unified_schema_v2() {
        let metrics =
            super::build_metrics_json(Path::new(TEST_INPUT), None, &sample_result(), None, None);
        assert_eq!(
            metrics[metrics_keys::SCHEMA_VERSION],
            metrics_keys::SCHEMA_VERSION_VALUE
        );
        let runtime = &metrics[metrics_keys::SECTION_RUNTIME];
        assert!(runtime.get(metrics_keys::RUNTIME_EXECUTION).is_some());
        assert!(runtime.get(metrics_keys::TOKEN_ACCOUNTING).is_some());
        assert!(runtime.get(metrics_keys::RUNTIME_OBSERVED_GRAPH).is_none());
        #[cfg(feature = "metrics")]
        assert!(runtime.get(metrics_keys::RUNTIME_LLM).is_some());
    }

    #[test]
    fn metrics_json_includes_compiler_section_when_diagnostics_present() {
        let compiler_diag = serde_json::json!({
            metrics_keys::COMPILER_PASSES: [],
            metrics_keys::COMPILER_SUMMARY: { metrics_keys::SUMMARY_TOTAL_PASSES: 0 }
        });
        let metrics = super::build_metrics_json(
            Path::new(TEST_INPUT),
            Some(1),
            &sample_result(),
            Some(&compiler_diag),
            None,
        );
        assert_eq!(
            metrics[metrics_keys::SCHEMA_VERSION],
            metrics_keys::SCHEMA_VERSION_VALUE
        );
        assert!(metrics.get(metrics_keys::SECTION_COMPILER).is_some());
        assert!(metrics.get(metrics_keys::SECTION_RUNTIME).is_some());
    }

    #[test]
    fn metrics_json_includes_backend_graph_snapshots() {
        let mut result = sample_result();
        result.graph_status_snapshots.push(
            GraphStatusSnapshot::graph_aware(TEST_EXECUTION_ID)
                .with_registered(true)
                .with_pin_counts(2, 16)
                .with_shape(Some(4), Some(3)),
        );

        let metrics =
            super::build_metrics_json(Path::new(TEST_INPUT), Some(2), &result, None, None);

        let graphs = metrics[metrics_keys::SECTION_BACKENDS][metrics_keys::BACKENDS_GRAPHS]
            .as_array()
            .expect("backend graph snapshots are emitted");
        assert_eq!(graphs.len(), 1);

        use metrics_keys::graph_status_keys as gsk;
        assert_eq!(
            graphs[0][gsk::BACKEND_KIND],
            apxm_core::constants::graph::backend_kind::GRAPH_AWARE
        );
        assert_eq!(graphs[0][gsk::GRAPH_ID], TEST_EXECUTION_ID);
        assert_eq!(graphs[0][gsk::PINNED_HANDLES], 2);
        assert_eq!(graphs[0][gsk::PINNED_BLOCKS], 16);
    }

    #[test]
    fn execution_summary_rolls_up_runtime_metrics() {
        let mut result = sample_result();
        result.token_snapshot.total.input_tokens = 11;
        result.token_snapshot.total.output_tokens = 7;
        result.token_snapshot.total.total_tokens = 18;
        result.token_snapshot.total.call_count = 2;
        result.token_snapshot.total.cached_input_tokens = 3;
        result.token_snapshot.total.reasoning_output_tokens = 5;
        result
            .graph_status_snapshots
            .push(GraphStatusSnapshot::graph_aware(TEST_EXECUTION_ID).with_registered(true));

        let summary = super::ExecutionSummary::from_result(&result);
        assert_eq!(summary.status, super::ExecutionSummaryStatus::Success);
        assert_eq!(summary.nodes_executed, 3);
        assert_eq!(summary.nodes_failed, 0);
        assert_eq!(summary.duration_ms, 42);
        assert_eq!(summary.llm_calls, 2);
        assert_eq!(summary.input_tokens, 11);
        assert_eq!(summary.output_tokens, 7);
        assert_eq!(summary.total_tokens, 18);
        assert_eq!(summary.cached_input_tokens, 3);
        assert_eq!(summary.reasoning_output_tokens, 5);
        assert_eq!(summary.backend_graphs, 1);
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn metrics_json_includes_backend_aggregate_when_llm_metrics_present() {
        let mut result = sample_result();
        result.llm_metrics = apxm_backends::AggregatedMetrics {
            total_requests: 2,
            successful_requests: 2,
            total_input_tokens: 10,
            total_output_tokens: 20,
            ..Default::default()
        };

        let metrics =
            super::build_metrics_json(Path::new(TEST_INPUT), Some(2), &result, None, None);

        assert_eq!(
            metrics[metrics_keys::SECTION_BACKENDS][metrics_keys::BACKENDS_AGGREGATE]
                [metrics_keys::llm_keys::TOTAL_REQUESTS],
            2
        );
    }
}
