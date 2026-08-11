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

// Typed apxm-server HTTP client (folded in from the former standalone
// apxm-client crate, ). `goal` uses it unconditionally; `chat`/`watch`/
// `sse_permissions` use it under the `driver` feature only.
mod client;
mod commands;
mod frontend;

use anyhow::Result;
use commands::*;
use serde_json::json;
use std::process::ExitCode;

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
            "apxm={lvl},apxm_execution={lvl},apxm_kernel={lvl},apxm_core={lvl},apxm_backends={lvl},apxm_server={lvl}"
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
        Commands::CompileServiceCanonical { agent_dir } => {
            commands::compile_service_canonical::compile_service_canonical_command(
                agent_dir, cli.config,
            )
        }
        Commands::ExecuteCanonical {
            input,
            invocation_admission,
            release,
            provenance,
        } => {
            execute_canonical_command(input, invocation_admission, release, provenance, cli.json)
                .await
        }
        Commands::Doctor => doctor_command(cli.config, cli.json),
        Commands::Backend { action } => backend_command(action, cli.json).await,
        Commands::Tool { action } => tool_command(action, cli.json),
        Commands::Team { action } => team_command(action, cli.json),
        Commands::Agent { action } => agent_command(action, cli.json),
        Commands::Org { action } => org_command(action, cli.json),
        Commands::Ops { action } => ops_command(action, cli.json),
        Commands::Validate {
            input,
            no_check_resources,
        } => validate_command(input, cli.json, no_check_resources),
        Commands::Analyze { input } => analyze_command(input, cli.json),
        Commands::Template { action } => template_command(action, cli.json),
        Commands::Explain { target } => explain_command(&target, cli.json),
        Commands::Codegen { action } => codegen_command(action, cli.json),
        Commands::CanonicalAir { input } => canonical_air_command(input),
        Commands::Session { action } => session_command(action, cli.json),
        Commands::Process { action } => process_command(action, cli.json),
        Commands::Cache { action } => cache_command(action, cli.json),
        Commands::Tokenize { text, file, model } => tokenize_command(text, file, model, cli.json),
        Commands::Watch { thread_id, expand } => watch_command(thread_id, expand).await,
        Commands::Rollout { action } => rollout_action(action).await,
        Commands::Chat {
            agent,
            air,
            server,
            session_id,
            capability_grant_ids,
            import,
            backend,
            model,
            owner,
        } => {
            commands::chat::chat_command(commands::chat::ChatOptions {
                agent,
                air,
                server,
                session_id,
                capability_grant_ids,
                import,
                backend,
                model,
                owner,
            })
            .await
        }
    }
}

#[cfg(feature = "driver")]
async fn rollout_action(action: commands::RolloutAction) -> Result<()> {
    use commands::rollout::{
        RolloutArchiveOptions, RolloutCompactOptions, RolloutListOptions, RolloutReplayOptions,
        rollout_archive_command, rollout_compact_command, rollout_list_command,
        rollout_replay_command,
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
        commands::RolloutAction::Compact {
            max_age_days,
            blob_grace_hours,
        } => {
            rollout_compact_command(RolloutCompactOptions {
                home: None,
                max_age_days,
                blob_grace_hours,
            })
            .await?;
            Ok(())
        }
    }
}

#[cfg(not(feature = "driver"))]
async fn run_cli_no_driver(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Init { name } => init_command(&name),
        Commands::ExecuteCanonical {
            input,
            invocation_admission,
            release,
            provenance,
        } => {
            execute_canonical_command(input, invocation_admission, release, provenance, cli.json)
                .await
        }
        Commands::Doctor => doctor_command(cli.config, cli.json),
        Commands::Tool { action } => tool_command(action, cli.json),
        Commands::Team { action } => team_command(action, cli.json),
        Commands::Agent { action } => agent_command(action, cli.json),
        Commands::Org { action } => org_command(action, cli.json),
        Commands::Ops { action } => ops_command(action, cli.json),
        Commands::Validate {
            input,
            no_check_resources,
        } => validate_command(input, cli.json, no_check_resources),
        Commands::Analyze { input } => analyze_command(input, cli.json),
        Commands::Template { action } => template_command(action, cli.json),
        Commands::Explain { target } => explain_command(&target, cli.json),
        Commands::Codegen { action } => codegen_command(action, cli.json),
        Commands::CanonicalAir { input } => canonical_air_command(input),
        Commands::Session { action } => session_command(action, cli.json),
        Commands::Process { action } => process_command(action, cli.json),
        Commands::Cache { action } => cache_command(action, cli.json),
        Commands::Tokenize { text, file, model } => tokenize_command(text, file, model, cli.json),
        Commands::Watch { .. } | Commands::Rollout { .. } | Commands::Chat { .. } => {
            Err(anyhow::anyhow!(
                "apxm watch / apxm rollout / apxm chat require the `driver` feature. Rebuild through `{}`, then re-run the command.",
                commands::dekk_hints::BUILD
            ))
        }
        Commands::CompileServiceCanonical { .. } => Err(anyhow::anyhow!(
            "apxm compile-service-canonical requires the `driver` feature. Rebuild through `{}`, then re-run the command.",
            commands::dekk_hints::BUILD
        )),
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
