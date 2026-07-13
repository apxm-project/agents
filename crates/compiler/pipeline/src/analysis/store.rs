//! Reusable, conservative compiler evidence for one execution-DAG snapshot.

use std::collections::{HashMap, HashSet};

use apxm_core::types::compiler::{
    BackendLegalityRequirements, CompilerAnalysisKind, CostSummary, EffectAuthoritySummary,
    OptimizationSummaryV1, PromptContractSummary, TransformationLegality,
};
use apxm_core::types::execution::ExecutionDag;

use super::{backend_legality, dag_use, effect_authority, profile_cost, prompt_contract, token_cost};

/// Every analysis kind supported by the compiler-private store.
pub(crate) const ALL_ANALYSES: &[CompilerAnalysisKind] = &[
    CompilerAnalysisKind::PromptContract,
    CompilerAnalysisKind::EffectAuthority,
    CompilerAnalysisKind::DagUse,
    CompilerAnalysisKind::TokenCost,
    CompilerAnalysisKind::ProfileCost,
    CompilerAnalysisKind::BackendLegality,
];

type NodeFacts<T> = Vec<Vec<T>>;

/// Typed dependency facts for every node in one DAG.
#[derive(Clone)]
struct DagUseFacts {
    data_edges: usize,
    effect_edges: usize,
    control_edges: usize,
    inputs: Vec<NodeInputs>,
}

/// Sorted producer identifiers grouped by dependency boundary.
#[derive(Clone)]
struct NodeInputs {
    data: Vec<u64>,
    effect: Vec<u64>,
    control: Vec<u64>,
}

/// Backend requirements and the legality derived from effect evidence.
#[derive(Clone)]
struct BackendLegalityFacts {
    requirements: NodeFacts<BackendLegalityRequirements>,
    transformations: NodeFacts<TransformationLegality>,
}

/// One cached analysis result keyed by [`CompilerAnalysisKind`].
enum CachedAnalysis {
    PromptContract(NodeFacts<PromptContractSummary>),
    EffectAuthority(NodeFacts<EffectAuthoritySummary>),
    DagUse(Vec<DagUseFacts>),
    TokenCost(NodeFacts<CostSummary>),
    ProfileCost(Vec<u64>),
    BackendLegality(BackendLegalityFacts),
}

/// Compiler-private cache for analyses derived from one execution-DAG snapshot.
///
/// The store never invents legality. A requested result is derived from the
/// current emitted DAG, and callers must discard every value a stage does not
/// explicitly preserve before using the store again.
pub(crate) struct AnalysisStore {
    dags: Vec<ExecutionDag>,
    cached: HashMap<CompilerAnalysisKind, CachedAnalysis>,
}

impl AnalysisStore {
    /// Start a store for one emitted execution-DAG snapshot.
    pub(crate) fn new(dags: Vec<ExecutionDag>) -> Self {
        Self {
            dags,
            cached: HashMap::new(),
        }
    }

    /// Replace the source snapshot while retaining only facts the caller proved preserved.
    pub(crate) fn rebase(&mut self, dags: Vec<ExecutionDag>) {
        if !same_node_layout(&self.dags, &dags) {
            self.cached.clear();
        }
        self.dags = dags;
    }

    /// Materialize every requested analysis exactly once for the current snapshot.
    pub(crate) fn materialize(&mut self, requested: &[CompilerAnalysisKind]) {
        for kind in requested {
            self.materialize_one(*kind);
        }
    }

    /// Discard all cached evidence not explicitly promised by the stage contract.
    pub(crate) fn retain_only(&mut self, preserved: &[CompilerAnalysisKind]) {
        let preserved: HashSet<_> = preserved.iter().copied().collect();
        self.cached.retain(|kind, _| preserved.contains(kind));
    }

    /// Whether a particular analysis is currently available for this snapshot.
    #[cfg(test)]
    pub(crate) fn is_materialized(&self, kind: CompilerAnalysisKind) -> bool {
        self.cached.contains_key(&kind)
    }

    /// Build the additive artifact projection from the same reusable evidence.
    pub(crate) fn summary(&mut self) -> OptimizationSummaryV1 {
        self.materialize(ALL_ANALYSES);

        let prompt = self.prompt_contracts();
        let effects = self.effect_authority();
        let dag_use = self.dag_use();
        let costs = self.token_costs();
        let profiles = self.profile_costs();
        let backend = self.backend_legality();

        let dags = self
            .dags
            .iter()
            .enumerate()
            .map(|(dag_index, dag)| {
                let use_facts = &dag_use[dag_index];
                let nodes = dag
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(node_index, node)| {
                        let inputs = &use_facts.inputs[node_index];
                        apxm_core::types::compiler::OperationOptimizationSummaryV1 {
                            node_id: node.id,
                            operation: node.op_type.mlir_mnemonic().to_string(),
                            effect_authority: effects[dag_index][node_index].clone(),
                            prompt: prompt[dag_index][node_index].clone(),
                            cost: costs[dag_index][node_index].clone(),
                            backend: backend.requirements[dag_index][node_index].clone(),
                            legality: backend.transformations[dag_index][node_index].clone(),
                            data_inputs: inputs.data.clone(),
                            effect_inputs: inputs.effect.clone(),
                            control_inputs: inputs.control.clone(),
                        }
                    })
                    .collect();

                apxm_core::types::compiler::DagOptimizationSummaryV1 {
                    name: dag.metadata.name.clone(),
                    data_edges: use_facts.data_edges,
                    effect_edges: use_facts.effect_edges,
                    control_edges: use_facts.control_edges,
                    weighted_critical_path_ms: profiles[dag_index],
                    nodes,
                }
            })
            .collect();

        OptimizationSummaryV1::new(dags)
    }

    fn materialize_one(&mut self, kind: CompilerAnalysisKind) {
        if self.cached.contains_key(&kind) {
            return;
        }

        let analysis = match kind {
            CompilerAnalysisKind::PromptContract => CachedAnalysis::PromptContract(
                self.dags
                    .iter()
                    .map(|dag| dag.nodes.iter().map(prompt_contract::summarize).collect())
                    .collect(),
            ),
            CompilerAnalysisKind::EffectAuthority => CachedAnalysis::EffectAuthority(
                self.dags
                    .iter()
                    .map(|dag| dag.nodes.iter().map(effect_authority::summarize).collect())
                    .collect(),
            ),
            CompilerAnalysisKind::DagUse => CachedAnalysis::DagUse(
                self.dags
                    .iter()
                    .map(|dag| {
                        let (data_edges, effect_edges, control_edges) = dag_use::edge_counts(dag);
                        let inputs = dag
                            .nodes
                            .iter()
                            .map(|node| {
                                let (data, effect, control) = dag_use::inputs(dag, node);
                                NodeInputs {
                                    data,
                                    effect,
                                    control,
                                }
                            })
                            .collect();
                        DagUseFacts {
                            data_edges,
                            effect_edges,
                            control_edges,
                            inputs,
                        }
                    })
                    .collect(),
            ),
            CompilerAnalysisKind::TokenCost => CachedAnalysis::TokenCost(
                self.dags
                    .iter()
                    .map(|dag| dag.nodes.iter().map(token_cost::summarize).collect())
                    .collect(),
            ),
            CompilerAnalysisKind::ProfileCost => CachedAnalysis::ProfileCost(
                self.dags
                    .iter()
                    .map(profile_cost::weighted_critical_path_ms)
                    .collect(),
            ),
            CompilerAnalysisKind::BackendLegality => {
                self.materialize_one(CompilerAnalysisKind::EffectAuthority);
                let effects = self.effect_authority();
                let requirements = self
                    .dags
                    .iter()
                    .map(|dag| dag.nodes.iter().map(backend_legality::requirements).collect())
                    .collect();
                let transformations = effects
                    .iter()
                    .map(|dag| {
                        dag.iter()
                            .map(backend_legality::transformation_legality)
                            .collect()
                    })
                    .collect();
                CachedAnalysis::BackendLegality(BackendLegalityFacts {
                    requirements,
                    transformations,
                })
            }
        };
        self.cached.insert(kind, analysis);
    }

    fn prompt_contracts(&self) -> &NodeFacts<PromptContractSummary> {
        let Some(CachedAnalysis::PromptContract(facts)) =
            self.cached.get(&CompilerAnalysisKind::PromptContract)
        else {
            unreachable!("prompt-contract analysis is materialized")
        };
        facts
    }

    fn effect_authority(&self) -> &NodeFacts<EffectAuthoritySummary> {
        let Some(CachedAnalysis::EffectAuthority(facts)) =
            self.cached.get(&CompilerAnalysisKind::EffectAuthority)
        else {
            unreachable!("effect-authority analysis is materialized")
        };
        facts
    }

    fn dag_use(&self) -> &Vec<DagUseFacts> {
        let Some(CachedAnalysis::DagUse(facts)) = self.cached.get(&CompilerAnalysisKind::DagUse)
        else {
            unreachable!("dag-use analysis is materialized")
        };
        facts
    }

    fn token_costs(&self) -> &NodeFacts<CostSummary> {
        let Some(CachedAnalysis::TokenCost(facts)) = self.cached.get(&CompilerAnalysisKind::TokenCost)
        else {
            unreachable!("token-cost analysis is materialized")
        };
        facts
    }

    fn profile_costs(&self) -> &Vec<u64> {
        let Some(CachedAnalysis::ProfileCost(facts)) =
            self.cached.get(&CompilerAnalysisKind::ProfileCost)
        else {
            unreachable!("profile-cost analysis is materialized")
        };
        facts
    }

    fn backend_legality(&self) -> &BackendLegalityFacts {
        let Some(CachedAnalysis::BackendLegality(facts)) =
            self.cached.get(&CompilerAnalysisKind::BackendLegality)
        else {
            unreachable!("backend-legality analysis is materialized")
        };
        facts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::{AISOperationType, DependencyType};
    use apxm_core::types::execution::{Edge, Node};

    #[test]
    fn invalidated_evidence_is_not_reused_for_a_rebased_snapshot() {
        let mut first = ExecutionDag::new();
        first.nodes.push(Node::new(1, AISOperationType::Nop));
        let mut store = AnalysisStore::new(vec![first]);
        store.materialize(&[
            CompilerAnalysisKind::DagUse,
            CompilerAnalysisKind::EffectAuthority,
        ]);
        assert!(store.is_materialized(CompilerAnalysisKind::DagUse));
        assert!(store.is_materialized(CompilerAnalysisKind::EffectAuthority));

        let mut second = ExecutionDag::new();
        second.nodes = vec![
            Node::new(1, AISOperationType::Nop),
            Node::new(2, AISOperationType::Nop),
        ];
        second
            .edges
            .push(Edge::new(1, 2, 1, DependencyType::Data));
        store.retain_only(&[CompilerAnalysisKind::EffectAuthority]);
        store.rebase(vec![second]);

        assert!(!store.is_materialized(CompilerAnalysisKind::DagUse));
        assert!(!store.is_materialized(CompilerAnalysisKind::EffectAuthority));
        store.materialize(&[CompilerAnalysisKind::DagUse]);
        assert_eq!(store.summary().dags[0].data_edges, 1);
    }
}

/// Whether two snapshots can safely retain per-node cached evidence.
fn same_node_layout(left: &[ExecutionDag], right: &[ExecutionDag]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.nodes.len() == right.nodes.len()
                && left
                    .nodes
                    .iter()
                    .zip(&right.nodes)
                    .all(|(left, right)| left.id == right.id && left.op_type == right.op_type)
        })
}
