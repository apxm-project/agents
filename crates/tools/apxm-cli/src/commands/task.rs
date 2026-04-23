//! Task / graph composition commands.

use anyhow::Result;

use super::cli::*;

pub fn task_command(action: TaskAction, _json_output: bool) -> Result<()> {
    match action {
        TaskAction::Merge { .. } => Err(anyhow::anyhow!(
            "Graph merge is no longer supported. Execute a single graph directly with `dekk apxm execute`, or use the legacy `dekk apxm workflow ...` surface only when you specifically need `.apxmw` files."
        )),
    }
}
