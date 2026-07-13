//! Deterministic topological phases for workflow analysis and baseline scheduling.

use super::{WorkflowPlan, WorkflowStep};

/// Compute execution phases using Kahn's algorithm.
///
/// Each phase is a Vec of step IDs that can run in parallel.
/// Steps with no dependencies go in phase 0, steps whose dependencies
/// are all satisfied in previous phases go in subsequent phases.
///
/// Returns an error if a cycle is detected.
pub fn execution_phases(steps: &[WorkflowStep]) -> anyhow::Result<Vec<Vec<String>>> {
    let plan = WorkflowPlan::build(steps)?;
    Ok(plan
        .phases()
        .iter()
        .map(|phase| {
            phase
                .iter()
                .map(|&index| plan.step(index).id().to_string())
                .collect()
        })
        .collect())
}
