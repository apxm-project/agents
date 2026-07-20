//! Emit canonical AIR from a canonical FrontendGraph through the native bridge.
//!
//! This reads a `apxm.frontend-graph.v1` document and lowers it in-process to
//! canonical `apxm.air.v1` JSON: no CLI subprocess and no network compile.

use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result};

pub fn canonical_air_command(input: Option<PathBuf>) -> Result<()> {
    let json = read_graph_json(input.as_ref())?;
    let air = apxm_program::lower_frontend_graph_json(&json).map_err(|e| anyhow::anyhow!(e))?;
    println!("{air}");
    Ok(())
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
