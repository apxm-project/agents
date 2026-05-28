//! Codegen command (regenerate frontend / typescript artifacts).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::cli::*;

pub fn codegen_command(action: CodegenAction, json_output: bool) -> Result<()> {
    match action {
        CodegenAction::Frontend { output_dir, check } => {
            let output_dir = output_dir.unwrap_or_else(default_frontend_codegen_dir);
            let rendered = crate::frontend::render_generated_python();
            let mut files: Vec<String> =
                rendered.iter().map(|(name, _)| name.to_string()).collect();
            files.sort();

            if check {
                check_generated_python_dir(&output_dir, &rendered)?;
            } else {
                crate::frontend::codegen::write_generated_python(&output_dir)?;
            }

            if json_output {
                let output = serde_json::json!({
                    "target": "frontend",
                    "output_dir": output_dir.display().to_string(),
                    "files": files,
                    "check": check,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!(
                    "{} frontend bindings:",
                    if check { "Checked" } else { "Generated" }
                );
                println!("  target: frontend");
                println!("  output: {}", output_dir.display());
                for file in files {
                    println!("  - {file}");
                }
            }

            Ok(())
        }
        CodegenAction::Typescript { output, check } => {
            let output_path = output.unwrap_or_else(default_typescript_codegen_path);
            let rendered = crate::frontend::codegen_ts::render_generated_typescript();
            if check {
                check_generated_file(&output_path, &rendered, "typescript")?;
            } else {
                crate::frontend::write_generated_typescript(&output_path)?;
            }

            if json_output {
                let output = serde_json::json!({
                    "target": "typescript",
                    "output": output_path.display().to_string(),
                    "check": check,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!(
                    "{} TypeScript types:",
                    if check { "Checked" } else { "Generated" }
                );
                println!("  target: typescript");
                println!("  output: {}", output_path.display());
            }

            Ok(())
        }
        CodegenAction::EventKinds { output, check } => {
            let output_path = output.unwrap_or_else(default_event_kinds_codegen_path);
            let rendered = crate::frontend::codegen_event_kinds::render_generated_event_kinds();
            if check {
                check_generated_file(&output_path, &rendered, "event-kinds")?;
            } else {
                crate::frontend::write_generated_event_kinds(&output_path)?;
            }

            if json_output {
                let output = serde_json::json!({
                    "target": "event-kinds",
                    "output": output_path.display().to_string(),
                    "count": apxm_core::events::kind::CORE_EVENT_KINDS.len(),
                    "check": check,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!(
                    "{} TypeScript event kinds:",
                    if check { "Checked" } else { "Generated" }
                );
                println!("  target: event-kinds");
                println!("  output: {}", output_path.display());
            }

            Ok(())
        }
    }
}

fn check_generated_file(output_path: &Path, rendered: &str, target: &str) -> Result<()> {
    let current = fs::read_to_string(output_path)?;
    if current != rendered {
        bail!(
            "{target} generated output is stale: rerun `apxm codegen {target}` for {}",
            output_path.display()
        );
    }
    Ok(())
}

fn check_generated_python_dir(
    output_dir: &Path,
    rendered: &[(&'static str, String)],
) -> Result<()> {
    let expected = rendered
        .iter()
        .map(|(name, _)| *name)
        .collect::<BTreeSet<_>>();

    for (filename, content) in rendered {
        let output_path = output_dir.join(filename);
        let current = fs::read_to_string(&output_path).map_err(|error| {
            anyhow::anyhow!(
                "frontend generated output is missing: rerun `apxm codegen frontend` for {} ({error})",
                output_path.display()
            )
        })?;
        if current != *content {
            bail!(
                "frontend generated output is stale: rerun `apxm codegen frontend` for {}",
                output_path.display()
            );
        }
    }

    for entry in fs::read_dir(output_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.ends_with(".py") && !expected.contains(name.as_str()) {
            bail!(
                "frontend generated output has unexpected file: remove {} or rerun `apxm codegen frontend`",
                entry.path().display()
            );
        }
    }

    Ok(())
}

fn default_frontend_codegen_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../compiler/apxm-frontend/python/apxm/_generated")
}

fn default_typescript_codegen_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../apxm-gui/frontend/src/types/generated.ts")
}

fn default_event_kinds_codegen_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../apxm-gui/frontend/src/lib/generated/core-event-kinds.ts")
}
