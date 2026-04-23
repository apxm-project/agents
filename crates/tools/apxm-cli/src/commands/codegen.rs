//! Codegen command (regenerate frontend / typescript artifacts).

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::cli::*;

pub fn codegen_command(action: CodegenAction, json_output: bool) -> Result<()> {
    match action {
        CodegenAction::Frontend { output_dir } => {
            let output_dir = output_dir.unwrap_or_else(default_frontend_codegen_dir);
            let rendered = crate::frontend::render_generated_python();
            let mut files: Vec<String> =
                rendered.iter().map(|(name, _)| name.to_string()).collect();
            files.sort();

            crate::frontend::codegen::write_generated_python(&output_dir)?;

            if json_output {
                let output = serde_json::json!({
                    "target": "frontend",
                    "output_dir": output_dir.display().to_string(),
                    "files": files,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Generated frontend bindings:");
                println!("  target: frontend");
                println!("  output: {}", output_dir.display());
                for file in files {
                    println!("  - {file}");
                }
            }

            Ok(())
        }
        CodegenAction::Typescript { output } => {
            let output_path = output.unwrap_or_else(default_typescript_codegen_path);
            crate::frontend::write_generated_typescript(&output_path)?;

            if json_output {
                let output = serde_json::json!({
                    "target": "typescript",
                    "output": output_path.display().to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Generated TypeScript types:");
                println!("  target: typescript");
                println!("  output: {}", output_path.display());
            }

            Ok(())
        }
    }
}

fn default_frontend_codegen_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../compiler/apxm-frontend/python/apxm/_generated")
}

fn default_typescript_codegen_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../apxm-gui/frontend/src/types/generated.ts")
}
