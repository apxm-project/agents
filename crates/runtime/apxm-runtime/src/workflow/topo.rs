//! Topological sort for workflow execution phases.

use super::def::WorkflowStep;
use std::collections::{HashMap, VecDeque};

/// Compute execution phases using Kahn's algorithm.
///
/// Each phase is a Vec of step IDs that can run in parallel.
/// Steps with no dependencies go in phase 0, steps whose dependencies
/// are all satisfied in previous phases go in subsequent phases.
///
/// Returns an error if a cycle is detected.
pub fn execution_phases(steps: &[WorkflowStep]) -> anyhow::Result<Vec<Vec<String>>> {
    if steps.is_empty() {
        return Ok(vec![]);
    }

    // Build adjacency list and in-degree map
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();

    for step in steps {
        in_degree.entry(step.id.clone()).or_insert(0);
        adjacency.entry(step.id.clone()).or_insert_with(Vec::new);
    }

    for step in steps {
        for dep in &step.depends_on {
            adjacency
                .entry(dep.clone())
                .or_insert_with(Vec::new)
                .push(step.id.clone());
            *in_degree.entry(step.id.clone()).or_insert(0) += 1;
        }
    }

    let mut phases: Vec<Vec<String>> = Vec::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut processed = 0;

    // Start with all nodes that have in-degree 0
    for (step_id, &degree) in &in_degree {
        if degree == 0 {
            queue.push_back(step_id.clone());
        }
    }

    while !queue.is_empty() {
        let mut current_phase = Vec::new();

        // Process all nodes at the current level
        let phase_size = queue.len();
        for _ in 0..phase_size {
            if let Some(step_id) = queue.pop_front() {
                current_phase.push(step_id.clone());
                processed += 1;

                // Reduce in-degree for dependents
                if let Some(dependents) = adjacency.get(&step_id) {
                    for dependent in dependents {
                        let degree = in_degree.get_mut(dependent).unwrap();
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(dependent.clone());
                        }
                    }
                }
            }
        }

        if !current_phase.is_empty() {
            phases.push(current_phase);
        }
    }

    // If we didn't process all nodes, there's a cycle
    if processed != steps.len() {
        anyhow::bail!(
            "Cycle detected in workflow: processed {} of {} steps",
            processed,
            steps.len()
        );
    }

    Ok(phases)
}
