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

use anyhow::Result;
use apxm_cli_dev::commands::{
    DevCli, DevCommands, analyze_command, canonical_air_command, codegen_command, explain_command,
    ops_command, template_command, validate_command,
};
use apxm_cli_dev::commands::{
    OutputAlreadyEmitted, compile_service_canonical, execute_canonical_command,
};
use clap::Parser;
use serde_json::json;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = DevCli::parse();
    let json_mode = cli.json;
    match run_dev(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            emit_cli_error(&err, json_mode);
            ExitCode::from(1)
        }
    }
}

fn run_dev(cli: DevCli) -> Result<()> {
    match cli.command {
        DevCommands::CompileServiceCanonical { agent_dir } => {
            compile_service_canonical::compile_service_canonical_command(agent_dir, cli.config)
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
                .map(apxm_cli_dev::commands::canonical_execute::admitted_package_handlers)
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
    if err.downcast_ref::<OutputAlreadyEmitted>().is_some() {
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
