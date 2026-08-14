//! Codegen command (regenerate frontend / typescript artifacts).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::cli::*;

pub fn codegen_command(action: CodegenAction, json_output: bool) -> Result<()> {
    match action {
        CodegenAction::Frontend { check } => {
            use crate::frontend::codegen::{
                GENERATED_PACKAGE_PYTHON_FILE, GENERATED_PYTHON_FRONTEND_FILES,
                RUNTIME_EVIDENCE_PYTHON_FILE,
            };

            let output_dir = default_python_generated_codegen_dir();
            let rendered = vec![
                (
                    GENERATED_PACKAGE_PYTHON_FILE,
                    crate::frontend::codegen::render_generated_package_python(),
                ),
                (
                    RUNTIME_EVIDENCE_PYTHON_FILE,
                    crate::frontend::codegen::render_runtime_evidence_python(),
                ),
            ];
            let files: Vec<String> = GENERATED_PYTHON_FRONTEND_FILES
                .iter()
                .map(|file| format!("apxm_program/_generated/{file}"))
                .collect();

            if check {
                // The whole directory is checked, not only this arm's two files:
                // every generated Python module lands here and a stray one must
                // not survive a regeneration unnoticed.
                check_generated_named_files(
                    &output_dir,
                    &rendered,
                    GENERATED_PYTHON_FRONTEND_FILES,
                    "py",
                    "frontend",
                )?;
            } else {
                fs::create_dir_all(&output_dir)?;
                for (filename, content) in &rendered {
                    fs::write(output_dir.join(filename), content)?;
                }
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
            let mut files: Vec<String> = rendered
                .iter()
                .map(|(name, _)| (*name).to_string())
                .collect();
            files.sort();

            if check {
                check_generated_named_files(
                    &output_dir,
                    &rendered,
                    crate::frontend::codegen_ts::GENERATED_TYPESCRIPT_FRONTEND_FILES,
                    "ts",
                    "typescript-frontend",
                )?;
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
        CodegenAction::Capabilities { check } => {
            let python_path = default_python_capabilities_codegen_path();
            let typescript_path = default_typescript_capabilities_codegen_path();
            let python = crate::frontend::codegen_capabilities::render_capabilities_python();
            let typescript =
                crate::frontend::codegen_capabilities::render_capabilities_typescript();
            let files = write_or_check_pair(
                [(&python_path, &python), (&typescript_path, &typescript)],
                check,
                "capabilities",
            )?;
            report_pair(
                "capabilities",
                "capability catalogue",
                files,
                check,
                json_output,
            )
        }
        CodegenAction::Permissions { check } => {
            let python_path = default_python_permissions_codegen_path();
            let typescript_path = default_typescript_permissions_codegen_path();
            let python = crate::frontend::codegen_permissions::render_permissions_python();
            let typescript = crate::frontend::codegen_permissions::render_permissions_typescript();
            let files = write_or_check_pair(
                [(&python_path, &python), (&typescript_path, &typescript)],
                check,
                "permissions",
            )?;
            report_pair(
                "permissions",
                "permission decisions",
                files,
                check,
                json_output,
            )
        }
        CodegenAction::FrontendVocabulary { check } => {
            let python_path = default_python_generated_codegen_dir()
                .join(crate::frontend::codegen_frontend_vocabulary::PYTHON_FRONTEND_GRAPH_FILE);
            let typescript_path = default_typescript_frontend_codegen_dir()
                .join(crate::frontend::codegen_frontend_vocabulary::TYPESCRIPT_FRONTEND_GRAPH_FILE);
            let python =
                crate::frontend::codegen_frontend_vocabulary::render_frontend_graph_python();
            let typescript =
                crate::frontend::codegen_frontend_vocabulary::render_frontend_graph_typescript();

            let files = write_or_check_pair(
                [(&python_path, &python), (&typescript_path, &typescript)],
                check,
                "frontend-vocabulary",
            )?;
            report_pair(
                "frontend-vocabulary",
                "source graph vocabulary",
                files,
                check,
                json_output,
            )
        }
        CodegenAction::FrontendRecords { check } => {
            let python_path = default_python_generated_codegen_dir()
                .join(crate::frontend::codegen_frontend_records::PYTHON_FRONTEND_RECORDS_FILE);
            let typescript_path = default_typescript_frontend_codegen_dir()
                .join(crate::frontend::codegen_frontend_records::TYPESCRIPT_FRONTEND_RECORDS_FILE);
            let python =
                crate::frontend::codegen_frontend_records::render_frontend_records_python();
            let typescript =
                crate::frontend::codegen_frontend_records::render_frontend_records_typescript();

            let files = write_or_check_pair(
                [(&python_path, &python), (&typescript_path, &typescript)],
                check,
                "frontend-records",
            )?;
            report_pair(
                "frontend-records",
                "FrontendGraph contract record types",
                files,
                check,
                json_output,
            )
        }
        CodegenAction::FrontendSerializers { check } => {
            let python_path = default_python_generated_codegen_dir().join(
                crate::frontend::codegen_frontend_serializers::PYTHON_FRONTEND_SERIALIZERS_FILE,
            );
            let typescript_path = default_typescript_frontend_codegen_dir().join(
                crate::frontend::codegen_frontend_serializers::TYPESCRIPT_FRONTEND_SERIALIZERS_FILE,
            );
            let python =
                crate::frontend::codegen_frontend_serializers::render_frontend_serializers_python();
            let typescript = crate::frontend::codegen_frontend_serializers::render_frontend_serializers_typescript();

            let files = write_or_check_pair(
                [(&python_path, &python), (&typescript_path, &typescript)],
                check,
                "frontend-serializers",
            )?;
            report_pair(
                "frontend-serializers",
                "FrontendGraph contract record serializers",
                files,
                check,
                json_output,
            )
        }
        CodegenAction::FrontendConformance { check } => {
            use crate::frontend::codegen_frontend_conformance as conformance;

            // Four files, two pairs: the harness each authoring package carries,
            // and the test-runner invoker that runs it. The invokers are
            // generated too — a hand-written one could be deleted and that
            // language would silently stop being checked.
            let python_path =
                default_python_generated_codegen_dir().join(conformance::PYTHON_CONFORMANCE_FILE);
            let typescript_path = default_typescript_frontend_codegen_dir()
                .join(conformance::TYPESCRIPT_CONFORMANCE_FILE);
            let python_test_path = default_python_conformance_test_dir()
                .join(conformance::PYTHON_CONFORMANCE_TEST_FILE);
            let typescript_test_path = default_typescript_conformance_test_dir()
                .join(conformance::TYPESCRIPT_CONFORMANCE_TEST_FILE);

            let python = conformance::render_conformance_python();
            let typescript = conformance::render_conformance_typescript();
            let python_test = conformance::render_conformance_python_test();
            let typescript_test = conformance::render_conformance_typescript_test();

            let mut files = write_or_check_pair(
                [(&python_path, &python), (&typescript_path, &typescript)],
                check,
                "frontend-conformance",
            )?;
            files.extend(write_or_check_pair(
                [
                    (&python_test_path, &python_test),
                    (&typescript_test_path, &typescript_test),
                ],
                check,
                "frontend-conformance",
            )?);
            report_pair(
                "frontend-conformance",
                "shared frontend conformance corpus harness",
                files,
                check,
                json_output,
            )
        }
        CodegenAction::Diagnostics { check } => {
            let python_path = default_python_generated_codegen_dir()
                .join(crate::frontend::codegen_diagnostics::PYTHON_DIAGNOSTICS_FILE);
            let typescript_path = default_typescript_frontend_codegen_dir()
                .join(crate::frontend::codegen_diagnostics::TYPESCRIPT_DIAGNOSTICS_FILE);
            let python = crate::frontend::codegen_diagnostics::render_diagnostics_python();
            let typescript = crate::frontend::codegen_diagnostics::render_diagnostics_typescript();

            let files = write_or_check_pair(
                [(&python_path, &python), (&typescript_path, &typescript)],
                check,
                "diagnostics",
            )?;
            report_pair(
                "diagnostics",
                "authoring diagnostic codes",
                files,
                check,
                json_output,
            )
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

/// Write — or drift-check — one vocabulary's Python and TypeScript halves, and
/// name the files either way. Both halves are one vocabulary, so they are
/// always written together and always checked together.
fn write_or_check_pair(
    outputs: [(&PathBuf, &String); 2],
    check: bool,
    target: &str,
) -> Result<Vec<String>> {
    for (path, rendered) in outputs {
        if check {
            check_generated_file(path, rendered, target)?;
        } else {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, rendered)?;
        }
    }
    Ok(outputs
        .iter()
        .map(|(path, _)| path.display().to_string())
        .collect())
}

/// Report one paired-vocabulary codegen arm in whichever form was asked for.
fn report_pair(
    target: &str,
    description: &str,
    files: Vec<String>,
    check: bool,
    json_output: bool,
) -> Result<()> {
    if json_output {
        let output = serde_json::json!({
            "target": target,
            "files": files,
            "check": check,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!(
            "{} {description}:",
            if check { "Checked" } else { "Generated" }
        );
        println!("  target: {target}");
        for file in files {
            println!("  - {file}");
        }
    }
    Ok(())
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
/// (`op-spec.json`, `op-spec.vectors.json`) are always both present or
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

/// Check `rendered` against `output_dir`, then reject any `extension` file in
/// that directory outside `owned` — the full set of generated filenames the
/// directory may hold, which spans every codegen arm writing into it.
fn check_generated_named_files(
    output_dir: &Path,
    rendered: &[(&'static str, String)],
    owned: &[&str],
    extension: &str,
    target: &str,
) -> Result<()> {
    let expected = owned.iter().copied().collect::<BTreeSet<_>>();
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
            && entry.path().extension().and_then(|found| found.to_str()) == Some(extension)
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

/// The one directory every generated Python frontend module lands in.
fn default_python_generated_codegen_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../compiler/frontend/python/apxm_program/_generated")
}

/// The Python frontend's test directory, which pytest collects.
fn default_python_conformance_test_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../compiler/frontend/python/tests_program")
}

/// The TypeScript frontend's test directory, which vitest collects.
fn default_typescript_conformance_test_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../compiler/frontend/typescript/test")
}

fn default_python_capabilities_codegen_path() -> PathBuf {
    default_python_generated_codegen_dir()
        .join(crate::frontend::codegen_capabilities::PYTHON_CAPABILITIES_FILE)
}

fn default_typescript_capabilities_codegen_path() -> PathBuf {
    default_typescript_frontend_codegen_dir()
        .join(crate::frontend::codegen_capabilities::TYPESCRIPT_CAPABILITIES_FILE)
}

fn default_python_permissions_codegen_path() -> PathBuf {
    default_python_generated_codegen_dir()
        .join(crate::frontend::codegen_permissions::PYTHON_PERMISSIONS_FILE)
}

fn default_typescript_permissions_codegen_path() -> PathBuf {
    default_typescript_frontend_codegen_dir()
        .join(crate::frontend::codegen_permissions::TYPESCRIPT_PERMISSIONS_FILE)
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
    // Downstream tooling may vendor/import the generated asset.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("generated/typescript/generated.ts")
}

fn default_typescript_frontend_codegen_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../compiler/frontend/typescript/src/generated")
}

fn default_event_kinds_codegen_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("generated/typescript/core-event-kinds.ts")
}
