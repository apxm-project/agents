//! Compiler-backed legality evidence for workflow scheduling.
//!
//! Workflow execution consumes this evidence only at already-ready boundaries.
//! It does not mutate the artifact or execution graph.

use std::collections::BTreeMap;

use apxm_core::types::NodeId;
use apxm_core::types::compiler::{
    OPTIMIZATION_SUMMARY_VERSION, OptimizationSummaryV1, ReplaySafety,
};

/// Workflow-runner support for a compiler-required checkpoint boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowCheckpointCapability {
    /// The runner injects the compiler-owned barrier before the listed nodes.
    InjectBeforeSideEffects,
    /// The runner cannot create a checkpoint boundary that the artifact did
    /// not already contain.
    DoesNotInjectCompilerBarriers,
}

/// A checkpoint placement result for a workflow step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowCheckpointPlacement {
    /// Every operation is proven replay-safe without an additional boundary.
    NotRequired,
    /// The listed operations require a durable checkpoint before execution.
    BeforeNodes(Vec<NodeId>),
    /// The compiler requires a placement that this workflow runner cannot
    /// provide. Callers must surface this typed capability boundary before any
    /// workflow step dispatches.
    Unsupported {
        required_before_nodes: Vec<NodeId>,
        capability: WorkflowCheckpointCapability,
    },
    /// The artifact cannot prove a replay placement for this workflow step.
    Unknown,
}

/// A durable workflow boundary derived directly from versioned compiler
/// replay evidence.
///
/// The runner records this boundary before it hands the artifact to the
/// runtime. It carries node identities only; request or effect content stays
/// in the artifact/runtime-owned replay stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCheckpointBarrier {
    pub step_id: String,
    pub required_before_nodes: Vec<NodeId>,
}

/// Artifact summaries available to one `.apxmw` workflow scheduling invocation.
#[derive(Debug, Clone, Default)]
pub struct WorkflowCriticalPathEvidence {
    /// Summary keyed by authored workflow step identifier.
    pub artifacts: BTreeMap<String, OptimizationSummaryV1>,
}

impl WorkflowCriticalPathEvidence {
    /// Return a step's compiler-proven weighted critical-path duration.
    ///
    /// Priority can change the order of otherwise-ready workflow steps. It is
    /// therefore available only when every operation in every executable DAG
    /// is proven reorderable by the versioned compiler legality summary.
    pub fn weighted_critical_path_ms(&self, step_id: &str) -> Option<u64> {
        let dags = self.versioned_dags(step_id)?;

        let mut total = 0_u64;
        for dag in dags {
            if dag.nodes.is_empty()
                || dag.weighted_critical_path_ms == 0
                || !dag.nodes.iter().all(|node| node.legality.may_reorder)
            {
                return None;
            }
            total = total.saturating_add(dag.weighted_critical_path_ms);
        }

        Some(total)
    }

    /// Return the durable boundary required by replay evidence for one step.
    fn checkpoint_requirement(&self, step_id: &str) -> WorkflowCheckpointPlacement {
        let Some(dags) = self.versioned_dags(step_id) else {
            return WorkflowCheckpointPlacement::Unknown;
        };
        let mut required_before = Vec::new();

        for node in dags.iter().flat_map(|dag| dag.nodes.iter()) {
            match node.effect_authority.replay_safety {
                ReplaySafety::Safe => {}
                ReplaySafety::RequiresCheckpoint => required_before.push(node.node_id),
                ReplaySafety::Unknown => return WorkflowCheckpointPlacement::Unknown,
            }
        }

        if required_before.is_empty() {
            WorkflowCheckpointPlacement::NotRequired
        } else {
            WorkflowCheckpointPlacement::BeforeNodes(required_before)
        }
    }

    /// Resolve a compiler-required placement against one runner's explicit
    /// checkpoint capability.
    pub fn checkpoint_placement(
        &self,
        step_id: &str,
        capability: WorkflowCheckpointCapability,
    ) -> WorkflowCheckpointPlacement {
        match self.checkpoint_requirement(step_id) {
            WorkflowCheckpointPlacement::BeforeNodes(required_before_nodes)
                if capability != WorkflowCheckpointCapability::InjectBeforeSideEffects =>
            {
                WorkflowCheckpointPlacement::Unsupported {
                    required_before_nodes,
                    capability,
                }
            }
            placement => placement,
        }
    }

    /// Build the compiler-owned workflow barrier for one executable step.
    ///
    /// A workflow runner may use this only after it has declared the
    /// `InjectBeforeSideEffects` capability. Missing or stale evidence stays
    /// an error instead of being converted into a best-effort boundary.
    pub fn checkpoint_barrier(
        &self,
        step_id: &str,
    ) -> Result<Option<WorkflowCheckpointBarrier>, String> {
        match self.checkpoint_placement(
            step_id,
            WorkflowCheckpointCapability::InjectBeforeSideEffects,
        ) {
            WorkflowCheckpointPlacement::NotRequired => Ok(None),
            WorkflowCheckpointPlacement::BeforeNodes(required_before_nodes) => {
                Ok(Some(WorkflowCheckpointBarrier {
                    step_id: step_id.to_string(),
                    required_before_nodes,
                }))
            }
            WorkflowCheckpointPlacement::Unknown => Err(format!(
                "workflow step '{step_id}' has incomplete replay legality evidence; checkpoint placement is unknown"
            )),
            WorkflowCheckpointPlacement::Unsupported { .. } => Err(format!(
                "workflow step '{step_id}' has no compiler-owned checkpoint injection capability"
            )),
        }
    }

    /// Return an artifact's current versioned execution summaries.
    fn versioned_dags(
        &self,
        step_id: &str,
    ) -> Option<&[apxm_core::types::compiler::DagOptimizationSummaryV1]> {
        let summary = self.artifacts.get(step_id)?;
        (summary.schema_version == OPTIMIZATION_SUMMARY_VERSION && !summary.dags.is_empty())
            .then_some(summary.dags.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::NodeId;
    use apxm_core::types::compiler::{
        DagOptimizationSummaryV1, OperationOptimizationSummaryV1, ReplaySafety,
    };

    fn operation() -> OperationOptimizationSummaryV1 {
        OperationOptimizationSummaryV1 {
            node_id: NodeId::default(),
            operation: "nop".to_string(),
            effect_authority: Default::default(),
            prompt: Default::default(),
            cost: Default::default(),
            backend: Default::default(),
            legality: Default::default(),
            decisions: Vec::new(),
            data_inputs: Vec::new(),
            effect_inputs: Vec::new(),
            control_inputs: Vec::new(),
        }
    }

    #[test]
    fn accepts_only_versioned_reorderable_critical_path_evidence() {
        let mut reorderable = operation();
        reorderable.legality.may_reorder = true;
        let evidence = WorkflowCriticalPathEvidence {
            artifacts: BTreeMap::from([(
                "pure".to_string(),
                OptimizationSummaryV1::new(vec![DagOptimizationSummaryV1 {
                    weighted_critical_path_ms: 42,
                    nodes: vec![reorderable],
                    ..Default::default()
                }]),
            )]),
        };

        assert_eq!(evidence.weighted_critical_path_ms("pure"), Some(42));
    }

    #[test]
    fn rejects_incomplete_or_nonreorderable_critical_path_evidence() {
        let mut nonreorderable = operation();
        nonreorderable.legality.may_reorder = false;
        let mut stale = OptimizationSummaryV1::new(vec![DagOptimizationSummaryV1 {
            weighted_critical_path_ms: 10,
            nodes: vec![nonreorderable],
            ..Default::default()
        }]);
        stale.schema_version = 0;
        let evidence = WorkflowCriticalPathEvidence {
            artifacts: BTreeMap::from([("stale".to_string(), stale)]),
        };

        assert_eq!(evidence.weighted_critical_path_ms("stale"), None);
    }

    #[test]
    fn derives_checkpoint_boundaries_only_from_versioned_replay_evidence() {
        let mut replay_bound = operation();
        replay_bound.node_id = 41;
        replay_bound.effect_authority.replay_safety = ReplaySafety::RequiresCheckpoint;
        let mut unknown = operation();
        unknown.node_id = 42;
        let evidence = WorkflowCriticalPathEvidence {
            artifacts: BTreeMap::from([
                (
                    "replay_bound".to_string(),
                    OptimizationSummaryV1::new(vec![DagOptimizationSummaryV1 {
                        nodes: vec![replay_bound],
                        ..Default::default()
                    }]),
                ),
                (
                    "unknown".to_string(),
                    OptimizationSummaryV1::new(vec![DagOptimizationSummaryV1 {
                        nodes: vec![unknown],
                        ..Default::default()
                    }]),
                ),
            ]),
        };

        assert_eq!(
            evidence.checkpoint_placement(
                "replay_bound",
                WorkflowCheckpointCapability::InjectBeforeSideEffects,
            ),
            WorkflowCheckpointPlacement::BeforeNodes(vec![41])
        );
        assert_eq!(
            evidence.checkpoint_placement(
                "replay_bound",
                WorkflowCheckpointCapability::DoesNotInjectCompilerBarriers,
            ),
            WorkflowCheckpointPlacement::Unsupported {
                required_before_nodes: vec![41],
                capability: WorkflowCheckpointCapability::DoesNotInjectCompilerBarriers,
            }
        );
        assert_eq!(
            evidence.checkpoint_placement(
                "unknown",
                WorkflowCheckpointCapability::InjectBeforeSideEffects,
            ),
            WorkflowCheckpointPlacement::Unknown
        );

        assert_eq!(
            evidence.checkpoint_barrier("replay_bound"),
            Ok(Some(WorkflowCheckpointBarrier {
                step_id: "replay_bound".to_string(),
                required_before_nodes: vec![41],
            }))
        );
        assert!(evidence.checkpoint_barrier("unknown").is_err());
    }
}
