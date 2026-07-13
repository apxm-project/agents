//! Immutable dependency planning for `.apxmw` workflow steps.

use anyhow::{Result, bail};
use std::collections::{BTreeSet, HashMap};

use super::WorkflowStep;

/// An immutable dependency plan for a workflow definition.
#[derive(Debug, Clone)]
pub struct WorkflowPlan {
    steps: Vec<WorkflowPlanStep>,
    phases: Vec<Vec<usize>>,
    index_by_id: HashMap<String, usize>,
}

/// One planned workflow step and its meta-DAG relationships.
#[derive(Debug, Clone)]
pub struct WorkflowPlanStep {
    id: String,
    declaration_index: usize,
    dependencies: Vec<usize>,
    dependents: Vec<usize>,
    phase: usize,
}

impl WorkflowPlan {
    /// Build a deterministic meta-DAG plan from workflow steps.
    pub fn build(steps: &[WorkflowStep]) -> Result<Self> {
        let mut ids = HashMap::with_capacity(steps.len());
        for (index, step) in steps.iter().enumerate() {
            if ids.insert(step.id.clone(), index).is_some() {
                bail!("Duplicate workflow step id '{}'", step.id);
            }
        }

        let mut planned_steps = steps
            .iter()
            .enumerate()
            .map(|(index, step)| WorkflowPlanStep {
                id: step.id.clone(),
                declaration_index: index,
                dependencies: Vec::with_capacity(step.depends_on.len()),
                dependents: Vec::new(),
                phase: 0,
            })
            .collect::<Vec<_>>();

        for (index, step) in steps.iter().enumerate() {
            for dependency_id in &step.depends_on {
                let Some(&dependency_index) = ids.get(dependency_id) else {
                    bail!(
                        "Workflow step '{}' depends on unknown step '{}'",
                        step.id,
                        dependency_id
                    );
                };
                planned_steps[index].dependencies.push(dependency_index);
                planned_steps[dependency_index].dependents.push(index);
            }
        }

        let mut pending_dependencies = planned_steps
            .iter()
            .map(|step| step.dependencies.len())
            .collect::<Vec<_>>();
        let mut current_phase = pending_dependencies
            .iter()
            .enumerate()
            .filter_map(|(index, &count)| (count == 0).then_some(index))
            .collect::<BTreeSet<_>>();
        let mut phases = Vec::new();
        let mut processed = 0_usize;

        while !current_phase.is_empty() {
            let phase = current_phase.into_iter().collect::<Vec<_>>();
            let phase_index = phases.len();
            let mut next_phase = BTreeSet::new();

            for &step_index in &phase {
                planned_steps[step_index].phase = phase_index;
                processed += 1;
                for &dependent_index in &planned_steps[step_index].dependents {
                    let remaining = pending_dependencies
                        .get_mut(dependent_index)
                        .expect("planner dependency index is valid");
                    *remaining = remaining.saturating_sub(1);
                    if *remaining == 0 {
                        next_phase.insert(dependent_index);
                    }
                }
            }

            phases.push(phase);
            current_phase = next_phase;
        }

        if processed != planned_steps.len() {
            bail!(
                "Cycle detected in workflow: processed {processed} of {} steps",
                planned_steps.len()
            );
        }

        Ok(Self {
            steps: planned_steps,
            phases,
            index_by_id: ids,
        })
    }

    /// Return every planned step in declaration order.
    pub fn steps(&self) -> &[WorkflowPlanStep] {
        &self.steps
    }

    /// Return deterministic topological phases for analysis and baseline mode.
    pub fn phases(&self) -> &[Vec<usize>] {
        &self.phases
    }

    /// Return one planned step by declaration index.
    pub fn step(&self, index: usize) -> &WorkflowPlanStep {
        &self.steps[index]
    }

    /// Resolve an authored workflow-step identifier to its declaration index.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.index_by_id.get(id).copied()
    }
}

impl WorkflowPlanStep {
    /// Return the authored workflow-step identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Return the stable authored declaration index.
    pub fn declaration_index(&self) -> usize {
        self.declaration_index
    }

    /// Return the declaration indices of prerequisite workflow steps.
    pub fn dependencies(&self) -> &[usize] {
        &self.dependencies
    }

    /// Return the declaration indices of directly dependent workflow steps.
    pub fn dependents(&self) -> &[usize] {
        &self.dependents
    }

    /// Return this step's legacy topological phase.
    pub fn phase(&self) -> usize {
        self.phase
    }
}
