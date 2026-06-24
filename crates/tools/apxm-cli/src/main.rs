//! APXM command-line interface.
//!
//! Provides the `apxm` binary with subcommands for building, compiling,
//! running graphs, environment diagnostics, and
//! backend registration and management.

#![allow(
    clippy::assigning_clones,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::cast_possible_wrap,
    clippy::format_push_string,
    clippy::manual_let_else,
    clippy::match_same_arms,
    clippy::match_wildcard_for_single_variants,
    clippy::option_option,
    clippy::struct_field_names
)]

mod commands;
// Only the driver-gated `chat` command consumes this; gate it to match so the
// default (non-driver) build doesn't compile it as dead code.
#[cfg(feature = "driver")]
mod context_assembly;
mod frontend;

use anyhow::Result;
use commands::*;
use serde_json::json;
#[cfg(feature = "driver")]
use std::path::PathBuf;
use std::process::ExitCode;

/// Resolve the --emit-session / --no-emit-session flag pair.
///
/// Default: emit-session ON (auto-path). Explicit --no-emit-session disables.
#[cfg(feature = "driver")]
fn resolve_emit_session(
    emit_session: Option<Option<PathBuf>>,
    no_emit_session: bool,
) -> Option<Option<PathBuf>> {
    if no_emit_session {
        None
    } else {
        Some(emit_session.unwrap_or(None))
    }
}

/// Initialize the tracing subscriber based on the --trace flag or RUST_LOG env var.
/// If neither is provided, no subscriber is registered (zero overhead).
#[cfg(feature = "driver")]
fn initialize_tracing(level: &Option<String>, json_mode: bool) {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    if json_mode {
        return;
    }

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
async fn main() -> ExitCode {
    use clap::Parser;

    let cli = Cli::parse();
    let json_mode = cli.json;
    match run_cli(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            emit_cli_error(&err, json_mode);
            ExitCode::from(1)
        }
    }
}

#[cfg(not(feature = "driver"))]
fn main() -> ExitCode {
    use clap::Parser;

    let cli = Cli::parse();
    let json_mode = cli.json;
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            emit_cli_error(&anyhow::Error::new(err), json_mode);
            return ExitCode::from(1);
        }
    };
    match rt.block_on(run_cli_no_driver(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            emit_cli_error(&err, json_mode);
            ExitCode::from(1)
        }
    }
}

#[cfg(feature = "driver")]
async fn run_cli(cli: Cli) -> Result<()> {
    // Initialize tracing if --trace flag is provided
    initialize_tracing(&cli.trace, cli.json);

    match cli.command {
        Commands::Init { name } => init_command(&name),
        Commands::Compile {
            input,
            output,
            emit_diagnostics,
            emit_metrics,
            opt_level,
            target,
            no_cse_llm,
            profile,
            warn,
            disable_passes,
            pass_list_override,
            embed_manifest,
        } => compile_command(
            input,
            output,
            emit_diagnostics,
            emit_metrics,
            opt_level,
            target,
            no_cse_llm,
            profile,
            warn,
            disable_passes,
            pass_list_override,
            cli.config,
            embed_manifest,
        ),
        Commands::Decompile { artifact, output } => decompile_command(artifact, output),
        Commands::Execute {
            input,
            args,
            opt_level,
            target,
            emit_metrics,
            emit_metrics_level,
            emit_session,
            no_emit_session,
            emit_profile,
        } => {
            execute_command(
                input,
                args,
                opt_level,
                target,
                cli.config,
                cli.json,
                emit_metrics,
                emit_metrics_level,
                resolve_emit_session(emit_session, no_emit_session),
                emit_profile,
            )
            .await
        }
        Commands::Run {
            input,
            args,
            target,
            emit_metrics,
            emit_metrics_level,
            emit_session,
            no_emit_session,
            emit_profile,
        } => {
            run_command(
                input,
                args,
                target,
                cli.config,
                cli.json,
                emit_metrics,
                emit_metrics_level,
                resolve_emit_session(emit_session, no_emit_session),
                emit_profile,
            )
            .await
        }
        Commands::Doctor => doctor_command(cli.config, cli.json),
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
        Commands::Codegen { action } => codegen_command(action, cli.json),
        Commands::Replay { session } => replay_command(session),
        Commands::Session { action } => session_command(action, cli.json),
        Commands::Process { action } => process_command(action, cli.json),
        Commands::Workflow { action } => workflow_command(action, cli.config, cli.json).await,
        Commands::Cache { action } => cache_command(action, cli.json),
        Commands::Tokenize { text, file, model } => tokenize_command(text, file, model, cli.json),
        Commands::Watch { thread_id, expand } => watch_command(thread_id, expand).await,
        Commands::Rollout { action } => rollout_action(action).await,
        Commands::Goal(args) => goal_command(args, cli.json).await,
        Commands::Chat {
            air,
            server,
            session_id,
            delegated_capability_ids,
            import,
            tools,
            backend,
            model,
            agent,
            agent_mode,
            agent_model,
            tree,
            monitor_url,
            max_turns,
            max_events,
            tool_budget,
            tool_cap,
            tool_auth,
            owner,
            author,
        } => {
            commands::chat::chat_command(commands::chat::ChatOptions {
                air,
                server,
                session_id,
                delegated_capability_ids,
                import,
                tools,
                backend,
                model,
                agent,
                agent_mode,
                agent_model,
                tree,
                monitor_url,
                max_turns,
                max_events,
                tool_budget,
                tool_cap,
                tool_auth,
                owner,
                author,
            })
            .await
        }
    }
}

#[cfg(feature = "driver")]
async fn rollout_action(action: commands::RolloutAction) -> Result<()> {
    use commands::rollout::{
        RolloutArchiveOptions, RolloutListOptions, RolloutReplayOptions, rollout_archive_command,
        rollout_list_command, rollout_replay_command,
    };
    match action {
        commands::RolloutAction::List {
            session,
            since,
            agent_role,
            limit,
        } => {
            rollout_list_command(RolloutListOptions {
                session,
                since,
                agent_role,
                limit,
                home: None,
            })
            .await
        }
        commands::RolloutAction::Replay { thread_id } => {
            rollout_replay_command(RolloutReplayOptions {
                thread_id,
                home: None,
            })
            .await
        }
        commands::RolloutAction::Archive {
            thread_id,
            output,
            skill_dir,
        } => {
            let path = rollout_archive_command(RolloutArchiveOptions {
                thread_id,
                output,
                home: None,
                skill_dir,
            })
            .await?;
            println!("wrote archive: {}", path.display());
            Ok(())
        }
    }
}

#[cfg(not(feature = "driver"))]
async fn run_cli_no_driver(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Init { name } => init_command(&name),
        Commands::Doctor => doctor_command(cli.config, cli.json),
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
        Commands::Codegen { action } => codegen_command(action, cli.json),
        Commands::Replay { session } => replay_command(session),
        Commands::Session { action } => session_command(action, cli.json),
        Commands::Process { action } => process_command(action, cli.json),
        Commands::Workflow { action } => workflow_command_no_driver(action, cli.json),
        Commands::Cache { action } => cache_command(action, cli.json),
        Commands::Tokenize { text, file, model } => tokenize_command(text, file, model, cli.json),
        Commands::Goal(args) => goal_command(args, cli.json).await,
        Commands::Watch { .. } | Commands::Rollout { .. } | Commands::Chat { .. } => {
            Err(anyhow::anyhow!(
                "apxm watch / apxm rollout / apxm chat require the `driver` feature. Rebuild through `{}`, then re-run the command.",
                commands::dekk_hints::BUILD
            ))
        }
        _ => Err(anyhow::anyhow!(
            "Command requires the `driver` feature. Rebuild through `{}`, then re-run the command.",
            commands::dekk_hints::BUILD
        )),
    }
}

fn emit_cli_error(err: &anyhow::Error, json_mode: bool) {
    if err
        .downcast_ref::<commands::OutputAlreadyEmitted>()
        .is_some()
    {
        return;
    }

    if json_mode {
        let causes: Vec<String> = err.chain().skip(1).map(ToString::to_string).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "error": err.to_string(),
                "causes": causes,
            }))
            .expect("serialize cli error")
        );
    } else {
        eprintln!("{err}");
        for cause in err.chain().skip(1) {
            eprintln!("  caused by: {cause}");
        }
    }
}
