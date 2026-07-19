//! Emit canonical AIR from a canonical FrontendGraph through the native bridge.
//!
//! This reads a `apxm.frontend-graph.v1` document and lowers it in-process: no
//! CLI subprocess and no network compile. By default it prints canonical AIR
//! JSON; with `--mlir` it prints the deterministic MLIR produced and verified by
//! the real toolchain.

use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result};

pub fn canonical_air_command(input: Option<PathBuf>, mlir: bool) -> Result<()> {
    let json = read_graph_json(input.as_ref())?;

    if mlir {
        let graph: apxm_program::FrontendGraph =
            serde_json::from_str(&json).context("failed to parse canonical frontend graph JSON")?;
        let air = apxm_program::frontend_graph_to_air(&graph)
            .map_err(|verdict| anyhow::anyhow!(render_verdict(&verdict)))?;
        let text = apxm_compiler::lower_and_verify(&air)
            .map_err(|error| anyhow::anyhow!("canonical MLIR lowering failed: {error}"))?;
        print!("{text}");
    } else {
        let air = apxm_program::lower_frontend_graph_json(&json).map_err(|e| anyhow::anyhow!(e))?;
        println!("{air}");
    }
    Ok(())
}

fn render_verdict(verdict: &apxm_program::Verdict) -> String {
    let rendered: Vec<String> = verdict
        .diagnostics()
        .iter()
        .map(|d| format!("{}:{}", d.code.slug(), d.location))
        .collect();
    format!("frontend graph rejected: [{}]", rendered.join("; "))
}

fn read_graph_json(input: Option<&PathBuf>) -> Result<String> {
    if let Some(path) = input {
        return std::fs::read_to_string(path).with_context(|| {
            format!("failed to read frontend graph JSON from {}", path.display())
        });
    }

    let mut json = String::new();
    std::io::stdin()
        .read_to_string(&mut json)
        .context("failed to read frontend graph JSON from stdin")?;
    if json.trim().is_empty() {
        anyhow::bail!("expected canonical frontend graph JSON on stdin");
    }
    Ok(json)
}
