//! Task / graph composition commands.

use anyhow::Result;

use super::cli::*;

pub fn task_command(action: TaskAction, _json_output: bool) -> Result<()> {
    match action {
        TaskAction::Merge { .. } => Err(anyhow::anyhow!(
            "Graph merge is no longer supported. Compose workflows using the workflow system instead."
        )),
    }
}
