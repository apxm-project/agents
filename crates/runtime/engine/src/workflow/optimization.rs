//! Conservative interprocedural workflow optimization planning.
//!
//! The planner consumes only additive artifact summaries. It emits
//! recommendations for callers to surface or explicitly apply; it never
//! inserts checkpoints, memoization, or batching behavior on its own.

use std::collections::BTreeMap;

use apxm_core::types::compiler::{OptimizationSummaryV1, ReplaySafety};

/// Artifact summaries available to one `.apxmw` workflow planner invocation.
#[derive(Debug, Clone, Default)]
pub struct WorkflowOptimizationInputs {
    /// Summary keyed by authored workflow step identifier.
    pub artifacts: BTreeMap<String, OptimizationSummaryV1>,
}

/// A checkpoint boundary justified by typed effect or replay evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointRecommendation {
    /// Workflow step that requires a durable resume boundary.
    pub step_id: String,
    /// Explanation derived from the additive summary.
    pub reason: CheckpointReason,
}

/// Typed reason for a checkpoint recommendation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointReason {
    /// An operation crosses an approval boundary.
    ApprovalRequired,
    /// An operation has a declared external write effect.
    ExternalWrite,
    /// Replaying the operation requires a checkpoint boundary.
    ReplaySafety,
}

/// Conservative workflow decisions that a caller may elect to apply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowOptimizationPlan {
    /// Steps before which callers should persist a replay boundary.
    pub checkpoints: Vec<CheckpointRecommendation>,
    /// Steps whose entire artifact is proven safe for exact memoization.
    pub exact_memoizable_steps: Vec<String>,
    /// No batching candidate is emitted unless every artifact operation proves
    /// batching legal. The current compiler summary fails closed by default.
    pub legally_batchable_steps: Vec<String>,
}

/// Derive workflow recommendations exclusively from emitted legality evidence.
pub fn plan_workflow_optimizations(
    inputs: &WorkflowOptimizationInputs,
) -> WorkflowOptimizationPlan {
    let mut plan = WorkflowOptimizationPlan::default();

    for (step_id, summary) in &inputs.artifacts {
        let operations = summary.dags.iter().flat_map(|dag| dag.nodes.iter());
        let operations = operations.collect::<Vec<_>>();
        if operations.is_empty() {
            continue;
        }

        if let Some(reason) = operations.iter().find_map(|operation| {
            operation
                .effect_authority
                .approval_required
                .then_some(CheckpointReason::ApprovalRequired)
                .or_else(|| {
                    (!operation.effect_authority.writes.is_empty())
                        .then_some(CheckpointReason::ExternalWrite)
                })
                .or_else(|| {
                    (operation.effect_authority.replay_safety == ReplaySafety::RequiresCheckpoint)
                        .then_some(CheckpointReason::ReplaySafety)
                })
        }) {
            plan.checkpoints.push(CheckpointRecommendation {
                step_id: step_id.clone(),
                reason,
            });
        }

        if operations
            .iter()
            .all(|operation| operation.legality.may_memoize)
        {
            plan.exact_memoizable_steps.push(step_id.clone());
        }
        if operations
            .iter()
            .all(|operation| operation.legality.may_batch)
        {
            plan.legally_batchable_steps.push(step_id.clone());
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::NodeId;
    use apxm_core::types::compiler::{
        DagOptimizationSummaryV1, EffectAuthoritySummary, OperationOptimizationSummaryV1,
    };

    fn operation() -> OperationOptimizationSummaryV1 {
        OperationOptimizationSummaryV1 {
            node_id: NodeId::default(),
            operation: "nop".to_string(),
            effect_authority: EffectAuthoritySummary::default(),
            prompt: Default::default(),
            cost: Default::default(),
            backend: Default::default(),
            legality: Default::default(),
            data_inputs: Vec::new(),
            effect_inputs: Vec::new(),
            control_inputs: Vec::new(),
        }
    }

    #[test]
    fn approval_and_writes_produce_checkpoint_recommendations() {
        let mut protected = operation();
        protected.effect_authority.approval_required = true;
        protected
            .effect_authority
            .writes
            .push("capability:write".to_string());
        let inputs = WorkflowOptimizationInputs {
            artifacts: BTreeMap::from([(
                "mutate".to_string(),
                OptimizationSummaryV1::new(vec![DagOptimizationSummaryV1 {
                    nodes: vec![protected],
                    ..Default::default()
                }]),
            )]),
        };

        let plan = plan_workflow_optimizations(&inputs);
        assert_eq!(plan.checkpoints.len(), 1);
        assert_eq!(plan.checkpoints[0].step_id, "mutate");
        assert_eq!(
            plan.checkpoints[0].reason,
            CheckpointReason::ApprovalRequired
        );
        assert!(plan.exact_memoizable_steps.is_empty());
        assert!(plan.legally_batchable_steps.is_empty());
    }
}
