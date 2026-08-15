//! Dekk-owned compiler and runtime conformance binary.

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
#[cfg(feature = "dev")]
mod frontend;
mod tui;

use anyhow::Result;
use clap::Parser;
use commands::*;
use serde_json::json;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = DevCli::parse();
    let json_mode = cli.json;
    match run_dev(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            emit_cli_error(&err, json_mode);
            ExitCode::from(1)
        }
    }
}

async fn run_dev(cli: DevCli) -> Result<()> {
    match cli.command {
        DevCommands::CompileServiceCanonical { agent_dir } => {
            #[cfg(feature = "driver")]
            {
                commands::compile_service_canonical::compile_service_canonical_command(
                    agent_dir, cli.config,
                )
            }
            #[cfg(not(feature = "driver"))]
            {
                let _ = agent_dir;
                Err(anyhow::anyhow!(
                    "apxm-dev compile-service-canonical requires the `driver` feature. Rebuild through `{}`.",
                    commands::dekk_hints::BUILD
                ))
            }
        }
        DevCommands::ExecuteCanonical {
            input,
            invocation_admission,
            release,
            provenance,
            package,
        } => {
            let handlers = package
                .as_deref()
                .map(commands::agent::admitted_package_handlers)
                .transpose()?
                .flatten();
            execute_canonical_command(
                input,
                invocation_admission,
                release,
                provenance,
                handlers,
                package,
                cli.json,
            )
            .await
        }
        DevCommands::Codegen { action } => codegen_command(action, cli.json),
        DevCommands::CanonicalAir { input } => canonical_air_command(input),
        DevCommands::Validate {
            input,
            no_check_resources,
        } => validate_command(input, cli.json, no_check_resources),
        DevCommands::Analyze { input } => analyze_command(input, cli.json),
        DevCommands::Ops { action } => ops_command(action, cli.json),
        DevCommands::Template { action } => template_command(action, cli.json),
        DevCommands::Explain { target } => explain_command(&target, cli.json),
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
