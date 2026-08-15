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
mod tui;

use anyhow::Result;
use commands::*;
use serde_json::json;
use std::process::ExitCode;

/// Initialize the tracing subscriber based on the --trace flag or RUST_LOG env var.
/// If neither is provided, no subscriber is registered (zero overhead).
#[cfg(feature = "driver")]
fn initialize_tracing(level: Option<&String>, json_mode: bool) {
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

fn main() -> ExitCode {
    use clap::Parser;

    let cli = Cli::parse();
    let json_mode = cli.json;
    match run_cli(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            emit_cli_error(&err, json_mode);
            ExitCode::from(1)
        }
    }
}

fn run_cli(cli: Cli) -> Result<()> {
    #[cfg(feature = "driver")]
    initialize_tracing(cli.trace.as_ref(), cli.json);

    match cli.command {
        Commands::Build { agent_package } => commands::interaction::build_command(agent_package),
        Commands::Run {
            agent_package,
            artifact,
            tui,
        } => commands::interaction::run_command(agent_package, artifact, tui),
        Commands::Event { action } => commands::interaction::event_command(action),
        Commands::Runtime { action } => commands::interaction::runtime_command(action),
        Commands::Resume { last } => commands::interaction::resume_command(last),
        Commands::Interact {
            agent_package,
            artifact,
        } => commands::interaction::interact_command(agent_package, artifact),
        Commands::Doctor => doctor_command(cli.config, cli.json),
        Commands::Agent { action } => agent_command(action, cli.json),
        Commands::Org { action } => org_command(action, cli.json),
        Commands::Process { action } => process_command(action, cli.json),
    }
}

fn emit_cli_error(err: &anyhow::Error, json_mode: bool) {
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
