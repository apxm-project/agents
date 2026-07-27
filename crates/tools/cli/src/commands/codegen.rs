//! Codegen command (regenerate frontend / typescript artifacts).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::cli::*;

pub fn codegen_command(action: CodegenAction, json_output: bool) -> Result<()> {
    match action {
        CodegenAction::Frontend { check } => {
            let evidence_path = default_python_runtime_evidence_codegen_path();
            let output_dir = evidence_path
                .parent()
                .expect("runtime evidence output has a parent")
                .to_path_buf();
            let evidence_init_path = evidence_path.with_file_name("__init__.py");
            let evidence = crate::frontend::codegen::render_runtime_evidence_python();
            let evidence_init = "from .runtime_evidence import *\n";
            let files = vec![
                "apxm_program/_generated/__init__.py".to_string(),
                "apxm_program/_generated/runtime_evidence.py".to_string(),
            ];

            if check {
                check_generated_file(&evidence_path, &evidence, "python runtime evidence")?;
                check_generated_file(
                    &evidence_init_path,
                    evidence_init,
                    "python runtime evidence init",
                )?;
            } else {
                fs::create_dir_all(&output_dir)?;
                fs::write(&evidence_path, evidence)?;
                fs::write(&evidence_init_path, evidence_init)?;
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
        CodegenAction::TypescriptFrontend { output_dir, check } => {
            let output_dir = output_dir.unwrap_or_else(default_typescript_frontend_codegen_dir);
            let rendered = crate::frontend::codegen_ts::render_typescript_frontend_files();
            let mut files: Vec<String> =
                rendered.iter().map(|(name, _)| name.to_string()).collect();
            files.sort();

            if check {
                check_generated_named_files(&output_dir, &rendered, "typescript-frontend")?;
            } else {
                crate::frontend::codegen_ts::write_typescript_frontend_generated(&output_dir)?;
            }

            if json_output {
                let output = serde_json::json!({
                    "target": "typescript-frontend",
                    "output_dir": output_dir.display().to_string(),
                    "files": files,
                    "check": check,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!(
                    "{} TypeScript frontend bindings:",
                    if check { "Checked" } else { "Generated" }
                );
                println!("  target: typescript-frontend");
                println!("  output: {}", output_dir.display());
                for file in files {
                    println!("  - {file}");
                }
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
        CodegenAction::OpSpec { output_dir, check } => {
            let output_dir = output_dir.unwrap_or_else(default_op_spec_codegen_dir);
            let rendered = apxm_ais::render_op_spec_files();
            let semantic_tablegen_path = default_semantic_tablegen_declarations_path();
            let semantic_tablegen = apxm_ais::generate_semantic_tablegen_declarations();
            let structural_tablegen_path = default_structural_tablegen_declarations_path();
            let structural_tablegen = apxm_ais::generate_structural_tablegen_declarations();
            let mut files: Vec<String> = rendered
                .iter()
                .map(|(name, _)| (*name).to_string())
                .collect();
            files.push(apxm_ais::SEMANTIC_TABLEGEN_DECLARATIONS_FILE.to_string());
            files.push(apxm_ais::STRUCTURAL_TABLEGEN_DECLARATIONS_FILE.to_string());
            files.sort();

            if check {
                check_generated_json_dir(&output_dir, &rendered, "op-spec")?;
                check_generated_file(
                    &semantic_tablegen_path,
                    &semantic_tablegen,
                    "semantic TableGen declarations",
                )?;
                check_generated_file(
                    &structural_tablegen_path,
                    &structural_tablegen,
                    "structural TableGen declarations",
                )?;
            } else {
                fs::create_dir_all(&output_dir)?;
                for (name, content) in &rendered {
                    fs::write(output_dir.join(name), content)?;
                }
                fs::write(&semantic_tablegen_path, semantic_tablegen)?;
                fs::write(&structural_tablegen_path, structural_tablegen)?;
            }

            if json_output {
                let output = serde_json::json!({
                    "target": "op-spec",
                    "output_dir": output_dir.display().to_string(),
                    "files": files,
                    "check": check,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!(
                    "{} op-spec catalog:",
                    if check { "Checked" } else { "Generated" }
                );
                println!("  target: op-spec");
                println!("  output: {}", output_dir.display());
                for file in files {
                    println!("  - {file}");
                }
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

/// Check a fixed, known set of generated filenames (rather than an
/// extension-filtered directory scan). Used by `op-spec`, whose two files
/// (`op-spec.v1.json`, `op-spec.vectors.v1.json`) are always both present or
/// the directory is stale/missing.
fn check_generated_json_dir(
    output_dir: &Path,
    rendered: &[(&'static str, String)],
    target: &str,
) -> Result<()> {
    for (filename, content) in rendered {
        let output_path = output_dir.join(filename);
        let current = fs::read_to_string(&output_path).map_err(|error| {
            anyhow::anyhow!(
                "{target} generated output is missing: rerun `apxm codegen {target}` for {} ({error})",
                output_path.display()
            )
        })?;
        if current != *content {
            bail!(
                "{target} generated output is stale: rerun `apxm codegen {target}` for {}",
                output_path.display()
            );
        }
    }
    Ok(())
}

fn check_generated_named_files(
    output_dir: &Path,
    rendered: &[(&'static str, String)],
    target: &str,
) -> Result<()> {
    let expected = rendered
        .iter()
        .map(|(filename, _)| *filename)
        .collect::<BTreeSet<_>>();
    for (filename, content) in rendered {
        let output_path = output_dir.join(filename);
        let current = fs::read_to_string(&output_path).map_err(|error| {
            anyhow::anyhow!(
                "{target} generated output is missing: rerun `apxm codegen {target}` for {} ({error})",
                output_path.display()
            )
        })?;
        if current != *content {
            bail!(
                "{target} generated output is stale: rerun `apxm codegen {target}` for {}",
                output_path.display()
            );
        }
    }
    for entry in fs::read_dir(output_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("ts")
            && entry
                .file_name()
                .to_str()
                .is_some_and(|filename| !expected.contains(filename))
        {
            bail!(
                "{target} generated output has unexpected file: remove {} or rerun `apxm codegen {target}`",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn default_python_runtime_evidence_codegen_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../compiler/frontend/python/apxm_program/_generated/runtime_evidence.py")
}

fn default_op_spec_codegen_dir() -> PathBuf {
    // Colocated with apxm-ais, the op-spec source of truth, so the
    // in-crate drift-gate tests (`crates/machine/ais/src/operations/op_spec.rs`)
    // and this CLI command always point at the same committed files.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../machine/ais/generated")
}

fn default_semantic_tablegen_declarations_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../compiler/pipeline/mlir/include/ais/Dialect/AIS/IR")
        .join(apxm_ais::SEMANTIC_TABLEGEN_DECLARATIONS_FILE)
}

fn default_structural_tablegen_declarations_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../compiler/pipeline/mlir/include/ais/Dialect/AIS/IR")
        .join(apxm_ais::STRUCTURAL_TABLEGEN_DECLARATIONS_FILE)
}

fn default_typescript_codegen_path() -> PathBuf {
    // Generated TS lives in-repo so APXM is self-contained. Consumers
    // (apxm-studio) vendor/import it.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("generated/typescript/generated.ts")
}

fn default_typescript_frontend_codegen_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../compiler/frontend/typescript/src/generated")
}

fn default_event_kinds_codegen_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("generated/typescript/core-event-kinds.ts")
}
