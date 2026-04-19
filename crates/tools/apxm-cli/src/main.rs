//! APXM command-line interface.
//!
//! Provides the `apxm` binary with subcommands for building, compiling,
//! running agent workflows, environment setup (`install`, `doctor`), and
//! LLM credential management (`register`).

mod commands;
mod frontend;

use anyhow::Result;
use commands::*;

/// Initialize the tracing subscriber based on the --trace flag or RUST_LOG env var.
/// If neither is provided, no subscriber is registered (zero overhead).
#[cfg(feature = "driver")]
fn initialize_tracing(level: &Option<String>) {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let filter_str = match level {
        Some(lvl) => format!(
            "apxm={lvl},apxm_runtime={lvl},apxm_driver={lvl},apxm_core={lvl},apxm_acp={lvl},apxm_backends={lvl},apxm_server={lvl}"
        ),
        None => match std::env::var("RUST_LOG") {
            Ok(val) if !val.is_empty() => val,
            _ => return, // No subscriber = no overhead
        },
    };

    let filter = EnvFilter::try_new(&filter_str).unwrap_or_else(|_| EnvFilter::new("apxm=info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_thread_ids(true)
                .with_target(true)
                .with_ansi(true)
                .with_writer(std::io::stderr),
        )
        .init();
}

#[cfg(feature = "driver")]
#[tokio::main]
async fn main() -> Result<()> {
    run_cli().await
}

#[cfg(not(feature = "driver"))]
fn main() -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_cli_no_driver())
}

#[cfg(feature = "driver")]
async fn run_cli() -> Result<()> {
    use clap::Parser;

    let cli = Cli::parse();

    // Initialize tracing if --trace flag is provided
    initialize_tracing(&cli.trace);

    match cli.command {
        Commands::Init { name } => init_command(&name),
        Commands::Compile {
            input,
            output,
            emit_diagnostics,
            opt_level,
            target,
            no_cse_llm,
            profile,
        } => compile_command(
            input,
            output,
            emit_diagnostics,
            opt_level,
            target,
            no_cse_llm,
            profile,
        ),
        Commands::Decompile { artifact, output } => decompile_command(artifact, output),
        Commands::Execute {
            input,
            args,
            opt_level,
            emit_metrics,
            emit_session,
            emit_profile,
        } => {
            execute_command(
                input,
                args,
                opt_level,
                cli.config,
                emit_metrics,
                emit_session,
                emit_profile,
            )
            .await
        }
        Commands::Run {
            input,
            args,
            emit_metrics,
            emit_session,
            emit_profile,
        } => {
            run_command(
                input,
                args,
                cli.config,
                emit_metrics,
                emit_session,
                emit_profile,
            )
            .await
        }
        Commands::Doctor => doctor_command(cli.config, cli.json),
        Commands::Activate { shell } => activate_command(&shell),
        Commands::Install => install_command(),
        Commands::Backend { action } => backend_command(action, cli.json).await,
        Commands::Tool { action } => tool_command(action, cli.json),
        Commands::Agent { action } => agent_command(action, cli.json).await,
        Commands::Team { action } => team_command(action, cli.json),
        Commands::Ops { action } => ops_command(action, cli.json),
        Commands::Validate {
            input,
            no_check_resources,
        } => validate_command(input, cli.json, no_check_resources),
        Commands::Analyze { input } => analyze_command(input, cli.json),
        Commands::Template { action } => template_command(action, cli.json),
        Commands::Explain { target } => explain_command(&target, cli.json),
        Commands::Task { action } => task_command(action, cli.json),
        Commands::Codegen { action } => codegen_command(action, cli.json),
        Commands::Replay { session } => replay_command(session),
        Commands::Session { action } => session_command(action, cli.json),
        Commands::Workflow { action } => workflow_command(action, cli.json).await,
        Commands::Cache { action } => cache_command(action, cli.json),
        Commands::Gui { file, port, open } => gui_command(file, port, open),
        Commands::OpenClaw { action } => openclaw_command(action, cli.json),
    }
}

#[cfg(not(feature = "driver"))]
async fn run_cli_no_driver() -> Result<()> {
    use clap::Parser;

    let cli = Cli::parse();
    match cli.command {
        Commands::Init { name } => init_command(&name),
        Commands::Doctor => doctor_command(cli.config, cli.json),
        Commands::Activate { shell } => activate_command(&shell),
        Commands::Install => install_command(),
        Commands::Tool { action } => tool_command(action, cli.json),
        Commands::Agent { action } => agent_command(action, cli.json).await,
        Commands::Team { action } => team_command(action, cli.json),
        Commands::Ops { action } => ops_command(action, cli.json),
        Commands::Validate {
            input,
            no_check_resources,
        } => validate_command(input, cli.json, no_check_resources),
        Commands::Analyze { input } => analyze_command(input, cli.json),
        Commands::Template { action } => template_command(action, cli.json),
        Commands::Explain { target } => explain_command(&target, cli.json),
        Commands::Task { action } => task_command(action, cli.json),
        Commands::Codegen { action } => codegen_command(action, cli.json),
        Commands::Replay { session } => replay_command(session),
        Commands::Session { action } => session_command(action, cli.json),
        Commands::Workflow { action } => workflow_command_no_driver(action, cli.json),
        Commands::Cache { action } => cache_command(action, cli.json),
        Commands::Gui { file, port, open } => gui_command(file, port, open),
        Commands::OpenClaw { action } => openclaw_command(action, cli.json),
        _ => Err(anyhow::anyhow!(
            "Command requires the `driver` feature. Re-run with: cargo run -p apxm-cli --features driver -- <command>"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::OperationCategory;

    // ── category_str tests ──────────────────────────────────────────────────

    #[test]
    fn category_str_reasoning() {
        assert_eq!(category_str(OperationCategory::Memory), "memory");
    }

    #[test]
    fn category_str_tools() {
        use apxm_core::types::OperationCategory;
        assert_eq!(category_str(OperationCategory::Tools), "tools");
    }

    #[test]
    fn category_str_control_flow() {
        use apxm_core::types::OperationCategory;
        assert_eq!(category_str(OperationCategory::ControlFlow), "control_flow");
    }

    #[test]
    fn category_str_all_variants() {
        use apxm_core::types::OperationCategory;
        // Ensure every variant maps to a non-empty string
        let variants = [
            OperationCategory::Metadata,
            OperationCategory::Memory,
            OperationCategory::Reasoning,
            OperationCategory::Tools,
            OperationCategory::ControlFlow,
            OperationCategory::Synchronization,
            OperationCategory::ErrorHandling,
            OperationCategory::Communication,
            OperationCategory::Internal,
            OperationCategory::Coordination,
            OperationCategory::Identity,
        ];
        for cat in variants {
            let s = category_str(cat);
            assert!(!s.is_empty(), "category_str returned empty for {:?}", cat);
        }
    }

    // ── find_op_spec tests ──────────────────────────────────────────────────

    #[test]
    fn find_op_spec_ask() {
        let spec = find_op_spec("ASK");
        assert!(spec.is_some(), "ASK should be a valid operation");
        let spec = spec.unwrap();
        assert_eq!(spec.op_type.to_string(), "ASK");
    }

    #[test]
    fn find_op_spec_think() {
        let spec = find_op_spec("THINK");
        assert!(spec.is_some(), "THINK should be a valid operation");
    }

    #[test]
    fn find_op_spec_inv() {
        // The canonical name is INV_TOOL (to_string() on AISOperationType::InvTool).
        // "INV" is the legacy .fromStr alias but not the Display name.
        let spec = find_op_spec("INV_TOOL");
        assert!(spec.is_some(), "INV_TOOL should be a valid operation");
    }

    #[test]
    fn find_op_spec_nonexistent() {
        assert!(find_op_spec("DOES_NOT_EXIST").is_none());
        assert!(find_op_spec("").is_none());
        assert!(find_op_spec("ask").is_none()); // case-sensitive
    }

    #[test]
    fn find_op_spec_returns_correct_category() {
        use apxm_core::types::OperationCategory;
        let ask = find_op_spec("ASK").unwrap();
        assert_eq!(ask.category, OperationCategory::Reasoning);

        let inv = find_op_spec("INV_TOOL").unwrap();
        assert_eq!(inv.category, OperationCategory::Tools);
    }

    // ── GraphAnalysis tests ─────────────────────────────────────────────────

    fn pipeline_graph() -> apxm_compiler::AirModule {
        serde_json::from_value(serde_json::json!({
            "name": "test-pipeline",
            "nodes": [
                {"id": 1, "name": "step-a", "op": "ASK", "attributes": {"template_str": "a"}},
                {"id": 2, "name": "step-b", "op": "ASK", "attributes": {"template_str": "b"}},
                {"id": 3, "name": "step-c", "op": "ASK", "attributes": {"template_str": "c"}}
            ],
            "edges": [
                {"from": 1, "to": 2, "dependency": "Data"},
                {"from": 2, "to": 3, "dependency": "Data"}
            ],
            "parameters": [],
            "metadata": {}
        }))
        .unwrap()
    }

    fn fanout_graph() -> apxm_compiler::AirModule {
        serde_json::from_value(serde_json::json!({
            "name": "test-fanout",
            "nodes": [
                {"id": 1, "name": "a", "op": "ASK", "attributes": {"template_str": "a"}},
                {"id": 2, "name": "b", "op": "ASK", "attributes": {"template_str": "b"}},
                {"id": 3, "name": "c", "op": "ASK", "attributes": {"template_str": "c"}},
                {"id": 4, "name": "sync", "op": "WAIT_ALL", "attributes": {"tokens": []}}
            ],
            "edges": [
                {"from": 1, "to": 4, "dependency": "Data"},
                {"from": 2, "to": 4, "dependency": "Data"},
                {"from": 3, "to": 4, "dependency": "Data"}
            ],
            "parameters": [],
            "metadata": {}
        }))
        .unwrap()
    }

    fn single_node_graph() -> apxm_compiler::AirModule {
        serde_json::from_value(serde_json::json!({
            "name": "single",
            "nodes": [
                {"id": 1, "name": "only", "op": "ASK", "attributes": {"template_str": "hi"}}
            ],
            "edges": [],
            "parameters": [],
            "metadata": {}
        }))
        .unwrap()
    }

    #[test]
    fn graph_analysis_pipeline_basic_properties() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        assert_eq!(ga.graph.name, "test-pipeline");
        assert_eq!(ga.graph.nodes.len(), 3);
        assert_eq!(ga.edge_count, 2);
        assert_eq!(ga.node_ids.len(), 3);
    }

    #[test]
    fn graph_analysis_pipeline_entry_exit_nodes() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // In a pipeline A->B->C, entry is [1] and exit is [3]
        assert_eq!(ga.entry_nodes, vec![1]);
        assert_eq!(ga.exit_nodes, vec![3]);
    }

    #[test]
    fn graph_analysis_pipeline_phases() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // Sequential pipeline should have 3 phases, each with 1 node
        assert_eq!(ga.phases.len(), 3);
        assert_eq!(ga.phases[0], vec![1]);
        assert_eq!(ga.phases[1], vec![2]);
        assert_eq!(ga.phases[2], vec![3]);
    }

    #[test]
    fn graph_analysis_pipeline_max_parallelism() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // Sequential => max parallelism = 1
        assert_eq!(ga.max_parallelism(), 1);
    }

    #[test]
    fn graph_analysis_pipeline_speedup() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // Sequential => speedup ~1.0
        let speedup = ga.speedup();
        assert!(
            (speedup - 1.0).abs() < 0.01,
            "sequential pipeline speedup should be ~1.0, got {}",
            speedup,
        );
    }

    #[test]
    fn graph_analysis_pipeline_critical_path() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        let (path, ms) = ga.critical_path();
        // Critical path includes all 3 nodes in sequence
        assert_eq!(path.len(), 3);
        assert_eq!(path, vec![1, 2, 3]);
        assert!(ms > 0, "critical path ms should be positive");
    }

    #[test]
    fn graph_analysis_fanout_max_parallelism() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // Nodes 1,2,3 are all entry nodes with no predecessors => phase 1 has 3 nodes
        assert_eq!(ga.max_parallelism(), 3);
    }

    #[test]
    fn graph_analysis_fanout_phases() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // Phase 1: [1,2,3] parallel, Phase 2: [4] sync
        assert_eq!(ga.phases.len(), 2);
        assert_eq!(ga.phases[0].len(), 3);
        assert_eq!(ga.phases[1], vec![4]);
    }

    #[test]
    fn graph_analysis_fanout_speedup_greater_than_one() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        let speedup = ga.speedup();
        assert!(
            speedup > 1.0,
            "fan-out graph should have speedup > 1.0, got {}",
            speedup,
        );
    }

    #[test]
    fn graph_analysis_fanout_entry_exit() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // Entry: 1,2,3 (sorted)
        let mut entries = ga.entry_nodes.clone();
        entries.sort();
        assert_eq!(entries, vec![1, 2, 3]);

        // Exit: 4
        assert_eq!(ga.exit_nodes, vec![4]);
    }

    #[test]
    fn graph_analysis_single_node() {
        let raw = single_node_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        assert_eq!(ga.graph.name, "single");
        assert_eq!(ga.graph.nodes.len(), 1);
        assert_eq!(ga.edge_count, 0);
        assert_eq!(ga.entry_nodes, vec![1]);
        assert_eq!(ga.exit_nodes, vec![1]);
        assert_eq!(ga.phases.len(), 1);
        assert_eq!(ga.max_parallelism(), 1);
        assert!((ga.speedup() - 1.0).abs() < 0.01);
    }

    #[test]
    fn graph_analysis_node_accessors() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        assert_eq!(ga.node_op(1), "ASK");
        assert_eq!(ga.node_name(1), "step-a");
        assert_eq!(ga.node_name(2), "step-b");
        assert_eq!(ga.node_name(3), "step-c");

        // Non-existent node returns "?"
        assert_eq!(ga.node_op(999), "?");
        assert_eq!(ga.node_name(999), "?");
    }

    #[test]
    fn graph_analysis_critical_path_fanout() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        let (path, ms) = ga.critical_path();
        // Critical path goes through one of the parallel nodes + sync
        assert_eq!(path.len(), 2);
        // The last node on the critical path should be the sync node
        assert_eq!(*path.last().unwrap(), 4);
        assert!(ms > 0);
    }

    #[test]
    fn graph_analysis_sequential_vs_parallel_ms() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        let seq = ga.sequential_ms();
        let par = ga.parallel_ms();
        // Sequential sum should be >= parallel sum
        assert!(
            seq >= par,
            "sequential {}ms should be >= parallel {}ms",
            seq,
            par
        );
    }

    #[test]
    fn graph_analysis_no_nodes_errors() {
        let result = serde_json::from_value::<apxm_compiler::AirModule>(
            serde_json::json!({"name": "empty"}),
        );
        assert!(result.is_err());
    }
}
