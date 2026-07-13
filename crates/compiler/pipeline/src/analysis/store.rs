//! Reusable, conservative compiler evidence for one execution-DAG snapshot.

use std::collections::{BTreeMap, HashMap, HashSet};

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::compiler::{
    BackendLegalityRequirements, CompilerAnalysisKind, CostSummary, EffectAuthoritySummary,
    OptimizationDecisionV1, OptimizationDisposition, OptimizationSummaryV1,
    OptimizationTransformKind, PromptContractSummary, TransformationLegality,
};
use apxm_core::types::execution::ExecutionDag;
use apxm_core::types::{AISOperationType, DependencyType, Number, PermissionOperation, Value};

use super::{
    backend_legality, dag_use, effect_authority,
    evidence::{AnalysisNodeKey, ApprovalEvidence, CompilerAnalysisInputs, TokenizerEvidence},
    profile_cost, prompt_contract, token_cost,
};

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
    inputs: CompilerAnalysisInputs,
    artifact_input_identity: ArtifactInputIdentity,
    cached: HashMap<CompilerAnalysisKind, CachedAnalysis>,
}

impl AnalysisStore {
    /// Start a store for one emitted execution-DAG snapshot.
    #[cfg(test)]
    pub(crate) fn new(dags: Vec<ExecutionDag>) -> Self {
        Self::with_inputs(dags, CompilerAnalysisInputs::default())
    }

    /// Start a store with typed external evidence for one execution-DAG snapshot.
    pub(crate) fn with_inputs(dags: Vec<ExecutionDag>, inputs: CompilerAnalysisInputs) -> Self {
        let artifact_input_identity = artifact_input_identity(&dags, &inputs);
        Self {
            dags,
            inputs,
            artifact_input_identity,
            cached: HashMap::new(),
        }
    }

    /// Replace the source snapshot while retaining only facts the caller proved preserved.
    #[cfg(test)]
    pub(crate) fn rebase(&mut self, dags: Vec<ExecutionDag>) {
        self.rebase_with_inputs(dags, self.inputs.clone());
    }

    /// Replace the source snapshot and external evidence with stable invalidation.
    pub(crate) fn rebase_with_inputs(
        &mut self,
        dags: Vec<ExecutionDag>,
        inputs: CompilerAnalysisInputs,
    ) {
        let artifact_input_identity = artifact_input_identity(&dags, &inputs);
        if self.artifact_input_identity != artifact_input_identity {
            self.cached.clear();
        }
        self.dags = dags;
        self.inputs = inputs;
        self.artifact_input_identity = artifact_input_identity;
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
                            decisions: optimization_decisions(
                                dag,
                                node,
                                &effects[dag_index][node_index],
                                &backend.requirements[dag_index][node_index],
                                &backend.transformations[dag_index][node_index],
                                &self.inputs,
                                AnalysisNodeKey {
                                    dag_index,
                                    node_id: node.id,
                                },
                            ),
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

    /// Stamp the runtime memoization guard from compiler-proven legality.
    ///
    /// An author may opt out with `memoizable=false`, but cannot opt an
    /// ineligible model call into reuse with `memoizable=true`.
    pub(crate) fn apply_memoization_guards(&mut self) {
        self.materialize(&[CompilerAnalysisKind::BackendLegality]);
        let legalities = self.backend_legality().transformations.clone();

        for (dag, dag_legalities) in self.dags.iter_mut().zip(legalities) {
            for (node, legality) in dag.nodes.iter_mut().zip(dag_legalities) {
                if !matches!(
                    node.op_type,
                    apxm_core::types::AISOperationType::Ask
                        | apxm_core::types::AISOperationType::Think
                        | apxm_core::types::AISOperationType::Reason
                ) {
                    continue;
                }
                let author_disabled = node
                    .get_attribute(apxm_core::constants::graph::attrs::MEMOIZABLE)
                    .and_then(apxm_core::types::Value::as_bool)
                    == Some(false);
                node.attributes.insert(
                    apxm_core::constants::graph::attrs::MEMOIZABLE.to_string(),
                    apxm_core::types::Value::Bool(legality.may_memoize && !author_disabled),
                );
            }
        }
    }

    /// Stamp only compiler-proven batch groups and remove unproven author hints.
    pub(crate) fn apply_batching_guards(&mut self) {
        self.materialize(&[CompilerAnalysisKind::BackendLegality]);
        let legalities = self.backend_legality().transformations.clone();

        for (dag_index, (dag, dag_legalities)) in self.dags.iter_mut().zip(legalities).enumerate() {
            for node in &mut dag.nodes {
                node.attributes.remove(graph_attrs::BATCH_GROUP);
            }

            let mut route_candidates: BTreeMap<(String, String, String), Vec<usize>> =
                BTreeMap::new();
            for (node_index, (node, legality)) in
                dag.nodes.iter().zip(dag_legalities.iter()).enumerate()
            {
                if !legality.may_batch {
                    continue;
                }
                let Some((backend, model)) = self.inputs.configured_route_identity(node) else {
                    continue;
                };
                route_candidates
                    .entry((
                        backend.to_string(),
                        model.to_string(),
                        node.op_type.mlir_mnemonic().to_string(),
                    ))
                    .or_default()
                    .push(node_index);
            }

            for ((backend, model, operation), mut candidates) in route_candidates {
                candidates.sort_unstable_by_key(|index| dag.nodes[*index].id);
                let mut groups: Vec<Vec<usize>> = Vec::new();
                for candidate in candidates {
                    if let Some(group) = groups.iter_mut().find(|group| {
                        group.iter().all(|member| {
                            backend_legality::dependency_independent(
                                dag,
                                dag.nodes[candidate].id,
                                dag.nodes[*member].id,
                            )
                        })
                    }) {
                        group.push(candidate);
                    } else {
                        groups.push(vec![candidate]);
                    }
                }

                for group in groups.into_iter().filter(|group| group.len() > 1) {
                    let group_id = stable_batch_group_id(
                        dag_index,
                        &backend,
                        &model,
                        &operation,
                        group.iter().map(|index| dag.nodes[*index].id),
                    );
                    for node_index in group {
                        dag.nodes[node_index].attributes.insert(
                            graph_attrs::BATCH_GROUP.to_string(),
                            Value::String(group_id.clone()),
                        );
                    }
                }
            }
        }
    }

    /// Consume the current snapshot after all execution guards are materialized.
    pub(crate) fn into_dags(self) -> Vec<ExecutionDag> {
        self.dags
    }

    fn materialize_one(&mut self, kind: CompilerAnalysisKind) {
        if self.cached.contains_key(&kind) {
            return;
        }

        let analysis = match kind {
            CompilerAnalysisKind::PromptContract => CachedAnalysis::PromptContract(
                self.dags
                    .iter()
                    .map(|dag| {
                        dag.nodes
                            .iter()
                            .map(|node| {
                                prompt_contract::summarize(
                                    node,
                                    self.inputs.tokenizer_for_node(node),
                                )
                            })
                            .collect()
                    })
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
                    .enumerate()
                    .map(|(dag_index, dag)| {
                        dag.nodes
                            .iter()
                            .map(|node| {
                                token_cost::summarize(
                                    node,
                                    self.inputs.profile_cost(AnalysisNodeKey {
                                        dag_index,
                                        node_id: node.id,
                                    }),
                                )
                            })
                            .collect()
                    })
                    .collect(),
            ),
            CompilerAnalysisKind::ProfileCost => {
                self.materialize_one(CompilerAnalysisKind::TokenCost);
                let costs = self.token_costs().clone();
                CachedAnalysis::ProfileCost(
                    self.dags
                        .iter()
                        .zip(costs.iter())
                        .map(|(dag, costs)| profile_cost::weighted_critical_path_ms(dag, costs))
                        .collect(),
                )
            }
            CompilerAnalysisKind::BackendLegality => {
                self.materialize_one(CompilerAnalysisKind::EffectAuthority);
                let effects = self.effect_authority();
                let requirements: NodeFacts<BackendLegalityRequirements> = self
                    .dags
                    .iter()
                    .map(|dag| {
                        dag.nodes
                            .iter()
                            .map(backend_legality::requirements)
                            .collect()
                    })
                    .collect();
                let transformations = self
                    .dags
                    .iter()
                    .zip(effects.iter())
                    .zip(requirements.iter())
                    .enumerate()
                    .map(|(dag_index, ((dag, dag_effects), dag_requirements))| {
                        backend_legality::transformation_legalities(
                            dag,
                            dag_effects,
                            dag_requirements,
                            &self.inputs,
                            dag_index,
                        )
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
        let Some(CachedAnalysis::TokenCost(facts)) =
            self.cached.get(&CompilerAnalysisKind::TokenCost)
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

/// Explain the compiler's production disposition for every optimization family.
fn optimization_decisions(
    dag: &ExecutionDag,
    node: &apxm_core::types::execution::Node,
    effect: &EffectAuthoritySummary,
    requirements: &BackendLegalityRequirements,
    legality: &TransformationLegality,
    inputs: &CompilerAnalysisInputs,
    key: AnalysisNodeKey,
) -> Vec<OptimizationDecisionV1> {
    vec![
        shared_prefix_decision(node),
        unproven_transform_decision(OptimizationTransformKind::TemplateSpecialization),
        unproven_transform_decision(OptimizationTransformKind::DeadContextElimination),
        unproven_transform_decision(OptimizationTransformKind::SchemaNarrowing),
        backend_selection_decision(node, requirements, inputs),
        scheduling_decision(node),
        checkpoint_decision(node, effect),
        memoization_decision(dag, node, effect, requirements, legality, inputs, key),
        batching_decision(dag, node, effect, requirements, legality, inputs, key),
        experimental_decision(
            OptimizationTransformKind::AskFusion,
            "ASK fusion remains disabled outside an explicit experiment surface",
        ),
        experimental_decision(
            OptimizationTransformKind::MemoryCondensation,
            "untyped memory condensation remains disabled outside an explicit experiment surface",
        ),
        experimental_decision(
            OptimizationTransformKind::SemanticCaching,
            "semantic caching remains disabled outside an explicit experiment surface",
        ),
        experimental_decision(
            OptimizationTransformKind::Speculation,
            if legality.may_speculate {
                "speculation is legal for this node but remains disabled in production"
            } else {
                "speculation remains disabled and its legality was not proven for this node"
            },
        ),
    ]
}

/// Report whether the artifact carries a reusable shared-prefix group.
fn shared_prefix_decision(node: &apxm_core::types::execution::Node) -> OptimizationDecisionV1 {
    if node.get_attribute(graph_attrs::REUSE_GROUP).is_some() {
        let mut reasons = vec![
            "artifact carries a compiler-produced backend-independent shared-prefix group"
                .to_string(),
        ];
        reasons.push(
            if node
                .get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
                .is_some()
            {
                "configured tokenizer evidence produced a shared-prefix token estimate"
            } else {
                "shared-prefix token estimation was withheld because tokenizer evidence was unavailable"
            }
            .to_string(),
        );
        decision(
            OptimizationTransformKind::SharedPrefixReuse,
            OptimizationDisposition::Applied,
            reasons,
        )
    } else {
        decision(
            OptimizationTransformKind::SharedPrefixReuse,
            OptimizationDisposition::Rejected,
            vec!["artifact contains no compiler-produced shared-prefix group".to_string()],
        )
    }
}

/// Reject an application claim when the final artifact has no per-node provenance.
fn unproven_transform_decision(transform: OptimizationTransformKind) -> OptimizationDecisionV1 {
    decision(
        transform,
        OptimizationDisposition::Rejected,
        vec![
            "final artifact carries no per-operation evidence that this transform changed the node"
                .to_string(),
        ],
    )
}

/// Report whether the authored route resolves to configured capability evidence.
fn backend_selection_decision(
    node: &apxm_core::types::execution::Node,
    requirements: &BackendLegalityRequirements,
    inputs: &CompilerAnalysisInputs,
) -> OptimizationDecisionV1 {
    if inputs.configured_route_identity(node).is_none() {
        return decision(
            OptimizationTransformKind::BackendSelection,
            OptimizationDisposition::Rejected,
            vec!["no unambiguous configured backend and model route was supplied".to_string()],
        );
    }
    if inputs.supports_backend_contract(node, requirements) {
        decision(
            OptimizationTransformKind::BackendSelection,
            OptimizationDisposition::Applied,
            vec!["configured route satisfies the authored backend capability contract".to_string()],
        )
    } else {
        decision(
            OptimizationTransformKind::BackendSelection,
            OptimizationDisposition::Rejected,
            vec![
                "configured route does not satisfy the authored backend capability contract"
                    .to_string(),
            ],
        )
    }
}

/// Report whether scheduling metadata survived into the executable node.
fn scheduling_decision(node: &apxm_core::types::execution::Node) -> OptimizationDecisionV1 {
    if node.metadata.priority > 0 || node.get_attribute(graph_attrs::STAGE_INDEX).is_some() {
        decision(
            OptimizationTransformKind::Scheduling,
            OptimizationDisposition::Applied,
            vec!["artifact carries compiler-produced scheduling metadata".to_string()],
        )
    } else {
        decision(
            OptimizationTransformKind::Scheduling,
            OptimizationDisposition::Rejected,
            vec![
                "artifact carries no compiler-produced scheduling metadata for this node"
                    .to_string(),
            ],
        )
    }
}

/// Report explicit checkpoint boundaries without claiming an inferred insertion.
fn checkpoint_decision(
    node: &apxm_core::types::execution::Node,
    effect: &EffectAuthoritySummary,
) -> OptimizationDecisionV1 {
    if node.op_type == AISOperationType::Checkpoint {
        decision(
            OptimizationTransformKind::CheckpointPlacement,
            OptimizationDisposition::Applied,
            vec!["artifact contains an explicit checkpoint boundary at this node".to_string()],
        )
    } else if effect.replay_safety == apxm_core::types::compiler::ReplaySafety::RequiresCheckpoint {
        decision(
            OptimizationTransformKind::CheckpointPlacement,
            OptimizationDisposition::Rejected,
            vec![
                "effect evidence requires a checkpoint but none is placed at this node".to_string(),
            ],
        )
    } else {
        decision(
            OptimizationTransformKind::CheckpointPlacement,
            OptimizationDisposition::Rejected,
            vec!["checkpoint placement is not required or proven at this node".to_string()],
        )
    }
}

/// Report the exact-reuse guard stamped into a model operation.
fn memoization_decision(
    dag: &ExecutionDag,
    node: &apxm_core::types::execution::Node,
    effect: &EffectAuthoritySummary,
    requirements: &BackendLegalityRequirements,
    legality: &TransformationLegality,
    inputs: &CompilerAnalysisInputs,
    key: AnalysisNodeKey,
) -> OptimizationDecisionV1 {
    let enabled = node
        .get_attribute(graph_attrs::MEMOIZABLE)
        .and_then(Value::as_bool)
        == Some(true);
    if enabled {
        return decision(
            OptimizationTransformKind::Memoization,
            OptimizationDisposition::Applied,
            vec!["compiler proved exact reuse legal and stamped memoizable=true".to_string()],
        );
    }

    let mut reasons = legality_rejection_reasons(dag, node, effect, requirements, inputs, key);
    if node
        .get_attribute(graph_attrs::MEMOIZABLE)
        .and_then(Value::as_bool)
        == Some(false)
        && legality.may_memoize
    {
        reasons.push("author configuration disabled memoization".to_string());
    }
    if reasons.is_empty() {
        reasons.push("compiler did not prove exact request reuse legal".to_string());
    }
    decision(
        OptimizationTransformKind::Memoization,
        OptimizationDisposition::Rejected,
        reasons,
    )
}

/// Report whether the compiler emitted a legal same-route batch group.
fn batching_decision(
    dag: &ExecutionDag,
    node: &apxm_core::types::execution::Node,
    effect: &EffectAuthoritySummary,
    requirements: &BackendLegalityRequirements,
    legality: &TransformationLegality,
    inputs: &CompilerAnalysisInputs,
    key: AnalysisNodeKey,
) -> OptimizationDecisionV1 {
    if legality.may_batch && node.get_attribute(graph_attrs::BATCH_GROUP).is_some() {
        return decision(
            OptimizationTransformKind::Batching,
            OptimizationDisposition::Applied,
            vec![
                "compiler proved an independent same-route peer and emitted a stable batch group"
                    .to_string(),
            ],
        );
    }

    let mut reasons = legality_rejection_reasons(dag, node, effect, requirements, inputs, key);
    if !inputs.supports_batching(node) {
        reasons.push("configured backend route did not declare batching support".to_string());
    }
    if legality.may_batch {
        reasons.push("no pairwise-independent batch group remained after grouping".to_string());
    }
    if reasons.is_empty() {
        reasons.push(
            "no independent compatible peer was available on the same configured route".to_string(),
        );
    }
    decision(
        OptimizationTransformKind::Batching,
        OptimizationDisposition::Rejected,
        reasons,
    )
}

/// Explain conservative legality gates shared by memoization and batching.
fn legality_rejection_reasons(
    dag: &ExecutionDag,
    node: &apxm_core::types::execution::Node,
    effect: &EffectAuthoritySummary,
    requirements: &BackendLegalityRequirements,
    inputs: &CompilerAnalysisInputs,
    key: AnalysisNodeKey,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if !matches!(
        node.op_type,
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
    ) {
        reasons.push("optimization applies only to model operations".to_string());
    }
    if !effect.writes.is_empty() {
        reasons.push("operation has observable writes".to_string());
    }
    if !effect.permission_operations.is_empty() {
        reasons.push("operation requires runtime authority".to_string());
    }
    if effect.approval_required {
        reasons.push("operation requires approval-policy admission".to_string());
    }
    if effect.replay_safety == apxm_core::types::compiler::ReplaySafety::RequiresCheckpoint {
        reasons.push("operation crosses a checkpoint-required replay boundary".to_string());
    } else if effect.replay_safety != apxm_core::types::compiler::ReplaySafety::Safe {
        reasons.push("replay safety was not proven".to_string());
    }
    if effect.determinism != apxm_core::types::compiler::Determinism::Proven {
        reasons.push("determinism was not proven".to_string());
    }
    if !effect.idempotent {
        reasons.push("idempotency was not proven".to_string());
    }
    if requirements.requires_tools {
        reasons.push("model operation can enter the runtime tool loop".to_string());
    }
    if !inputs.supports_backend_contract(node, requirements) {
        reasons.push("configured backend capability evidence is insufficient".to_string());
    }
    if !inputs.confinement_satisfied(key, node) {
        reasons.push("required execution confinement was not proven".to_string());
    }
    if !inputs.execution_ready(key, dag, node, effect) {
        reasons.push("typed dependency, grant, or approval readiness was not proven".to_string());
    }
    reasons
}

/// Build one canonical decision with at least one explanation.
fn decision(
    transform: OptimizationTransformKind,
    disposition: OptimizationDisposition,
    reasons: Vec<String>,
) -> OptimizationDecisionV1 {
    debug_assert!(!reasons.is_empty());
    OptimizationDecisionV1 {
        transform,
        disposition,
        reasons,
    }
}

/// Mark one roadmap-defined optimization as unavailable in production.
fn experimental_decision(
    transform: OptimizationTransformKind,
    reason: &str,
) -> OptimizationDecisionV1 {
    decision(
        transform,
        OptimizationDisposition::Experimental,
        vec![reason.to_string()],
    )
}

/// Derive a deterministic group identity from configured route and member nodes.
fn stable_batch_group_id(
    dag_index: usize,
    backend: &str,
    model: &str,
    operation: &str,
    node_ids: impl IntoIterator<Item = u64>,
) -> String {
    let mut hasher = blake3::Hasher::new();
    update_bytes(&mut hasher, b"apxm.compiler.batch-group.v1");
    update_len(&mut hasher, dag_index);
    update_string(&mut hasher, backend);
    update_string(&mut hasher, model);
    update_string(&mut hasher, operation);
    for node_id in node_ids {
        update_u64(&mut hasher, node_id);
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{BackendCapabilityEvidence, ConfiguredBackendEvidence};
    use apxm_core::types::execution::Node;
    use apxm_core::types::{AISOperationType, Value};

    #[test]
    fn changed_artifact_input_invalidates_evidence_with_the_same_node_layout() {
        let mut first = ExecutionDag::new();
        let mut first_node = Node::new(1, AISOperationType::Nop);
        first_node
            .attributes
            .insert("marker".to_string(), Value::String("first".to_string()));
        first.nodes.push(first_node);
        let mut store = AnalysisStore::new(vec![first]);
        store.materialize(&[
            CompilerAnalysisKind::DagUse,
            CompilerAnalysisKind::EffectAuthority,
        ]);
        assert!(store.is_materialized(CompilerAnalysisKind::DagUse));
        assert!(store.is_materialized(CompilerAnalysisKind::EffectAuthority));

        let mut second = ExecutionDag::new();
        let mut second_node = Node::new(1, AISOperationType::Nop);
        second_node
            .attributes
            .insert("marker".to_string(), Value::String("second".to_string()));
        second.nodes.push(second_node);
        store.rebase(vec![second]);

        assert!(!store.is_materialized(CompilerAnalysisKind::DagUse));
        assert!(!store.is_materialized(CompilerAnalysisKind::EffectAuthority));
    }

    #[test]
    fn stable_identity_preserves_evidence_when_attribute_maps_reorder() {
        let mut first = ExecutionDag::new();
        let mut first_node = Node::new(1, AISOperationType::Nop);
        first_node
            .attributes
            .insert("alpha".to_string(), Value::String("a".to_string()));
        first_node
            .attributes
            .insert("beta".to_string(), Value::String("b".to_string()));
        first.nodes.push(first_node);
        let mut store = AnalysisStore::new(vec![first]);
        store.materialize(&[CompilerAnalysisKind::EffectAuthority]);

        let mut second = ExecutionDag::new();
        let mut second_node = Node::new(1, AISOperationType::Nop);
        second_node
            .attributes
            .insert("beta".to_string(), Value::String("b".to_string()));
        second_node
            .attributes
            .insert("alpha".to_string(), Value::String("a".to_string()));
        second.nodes.push(second_node);
        store.rebase(vec![second]);

        assert!(store.is_materialized(CompilerAnalysisKind::EffectAuthority));
    }

    #[test]
    fn changed_external_evidence_invalidates_cached_analyses() {
        let mut dag = ExecutionDag::new();
        dag.nodes.push(Node::new(1, AISOperationType::Ask));
        let inputs = CompilerAnalysisInputs {
            configured_backends: vec![ConfiguredBackendEvidence {
                backend: "configured-backend".to_string(),
                model: "configured-model".to_string(),
                aliases: Default::default(),
                available: true,
                capabilities: BackendCapabilityEvidence::default(),
            }],
            ..Default::default()
        };
        let mut store = AnalysisStore::with_inputs(vec![dag.clone()], inputs.clone());
        store.materialize(&[CompilerAnalysisKind::BackendLegality]);
        assert!(store.is_materialized(CompilerAnalysisKind::BackendLegality));

        let mut changed = inputs;
        changed.configured_backends[0].capabilities.batching = true;
        store.rebase_with_inputs(vec![dag], changed);

        assert!(!store.is_materialized(CompilerAnalysisKind::BackendLegality));
    }

    #[test]
    fn stable_identity_ignores_configured_backend_order() {
        let mut dag = ExecutionDag::new();
        dag.nodes.push(Node::new(1, AISOperationType::Ask));
        let first = ConfiguredBackendEvidence {
            backend: "first-backend".to_string(),
            model: "first-model".to_string(),
            aliases: Default::default(),
            available: true,
            capabilities: BackendCapabilityEvidence::default(),
        };
        let second = ConfiguredBackendEvidence {
            backend: "second-backend".to_string(),
            model: "second-model".to_string(),
            aliases: Default::default(),
            available: true,
            capabilities: BackendCapabilityEvidence::default(),
        };
        let inputs = CompilerAnalysisInputs {
            configured_backends: vec![first.clone(), second.clone()],
            ..Default::default()
        };
        let mut store = AnalysisStore::with_inputs(vec![dag.clone()], inputs);
        store.materialize(&[CompilerAnalysisKind::BackendLegality]);

        store.rebase_with_inputs(
            vec![dag],
            CompilerAnalysisInputs {
                configured_backends: vec![second, first],
                ..Default::default()
            },
        );

        assert!(store.is_materialized(CompilerAnalysisKind::BackendLegality));
    }
}

/// Stable identity for the complete artifact input consumed by compiler analyses.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ArtifactInputIdentity([u8; blake3::OUT_LEN]);

/// Hash every analysis-relevant artifact input with map keys in canonical order.
fn artifact_input_identity(
    dags: &[ExecutionDag],
    inputs: &CompilerAnalysisInputs,
) -> ArtifactInputIdentity {
    let mut hasher = blake3::Hasher::new();
    update_bytes(&mut hasher, b"apxm.compiler.analysis-input.v2");
    update_len(&mut hasher, dags.len());

    for dag in dags {
        update_optional_string(&mut hasher, dag.metadata.name.as_deref());
        update_bool(&mut hasher, dag.metadata.is_entry);
        update_len(&mut hasher, dag.metadata.parameters.len());
        for parameter in &dag.metadata.parameters {
            update_string(&mut hasher, &parameter.name);
            update_string(&mut hasher, &parameter.type_name);
        }

        update_len(&mut hasher, dag.nodes.len());
        for node in &dag.nodes {
            update_u64(&mut hasher, node.id);
            update_string(&mut hasher, node.op_type.mlir_mnemonic());
            update_len(&mut hasher, node.attributes.len());
            let mut attributes: Vec<_> = node.attributes.iter().collect();
            attributes.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
            for (name, value) in attributes {
                update_string(&mut hasher, name);
                update_value(&mut hasher, value);
            }
            update_u64_slice(&mut hasher, &node.input_tokens);
            update_u64_slice(&mut hasher, &node.output_tokens);
            update_u32(&mut hasher, node.metadata.priority);
            update_optional_u64(&mut hasher, node.metadata.estimated_latency);
        }

        update_len(&mut hasher, dag.edges.len());
        for edge in &dag.edges {
            update_u64(&mut hasher, edge.from);
            update_u64(&mut hasher, edge.to);
            update_u64(&mut hasher, edge.token_id);
            update_u8(
                &mut hasher,
                match edge.dependency_type {
                    DependencyType::Data => 0,
                    DependencyType::Effect => 1,
                    DependencyType::Control => 2,
                },
            );
        }
        update_u64_slice(&mut hasher, &dag.entry_nodes);
        update_u64_slice(&mut hasher, &dag.exit_nodes);
    }

    update_analysis_inputs(&mut hasher, inputs);

    ArtifactInputIdentity(*hasher.finalize().as_bytes())
}

/// Hash every external evidence source with canonical collection ordering.
fn update_analysis_inputs(hasher: &mut blake3::Hasher, inputs: &CompilerAnalysisInputs) {
    let mut backends: Vec<_> = inputs.configured_backends.iter().collect();
    backends.sort_by(|left, right| {
        left.backend
            .cmp(&right.backend)
            .then_with(|| left.model.cmp(&right.model))
            .then_with(|| left.aliases.cmp(&right.aliases))
            .then_with(|| left.available.cmp(&right.available))
            .then_with(|| left.capabilities.tools.cmp(&right.capabilities.tools))
            .then_with(|| {
                left.capabilities
                    .structured_output
                    .cmp(&right.capabilities.structured_output)
            })
            .then_with(|| {
                left.capabilities
                    .prefix_reuse
                    .cmp(&right.capabilities.prefix_reuse)
            })
            .then_with(|| left.capabilities.thinking.cmp(&right.capabilities.thinking))
            .then_with(|| left.capabilities.batching.cmp(&right.capabilities.batching))
            .then_with(|| {
                left.capabilities
                    .custom_temperature
                    .cmp(&right.capabilities.custom_temperature)
            })
            .then_with(|| {
                left.capabilities
                    .context_window
                    .cmp(&right.capabilities.context_window)
            })
            .then_with(|| {
                left.capabilities
                    .max_output_tokens
                    .cmp(&right.capabilities.max_output_tokens)
            })
            .then_with(|| {
                tokenizer_tag(left.capabilities.tokenizer)
                    .cmp(&tokenizer_tag(right.capabilities.tokenizer))
            })
    });
    update_len(hasher, backends.len());
    for backend in backends {
        update_string(hasher, &backend.backend);
        update_string(hasher, &backend.model);
        update_len(hasher, backend.aliases.len());
        for alias in &backend.aliases {
            update_string(hasher, alias);
        }
        update_bool(hasher, backend.available);
        update_bool(hasher, backend.capabilities.tools);
        update_bool(hasher, backend.capabilities.structured_output);
        update_bool(hasher, backend.capabilities.prefix_reuse);
        update_bool(hasher, backend.capabilities.thinking);
        update_bool(hasher, backend.capabilities.batching);
        update_bool(hasher, backend.capabilities.custom_temperature);
        update_optional_u64(hasher, backend.capabilities.context_window);
        update_optional_u64(hasher, backend.capabilities.max_output_tokens);
        update_u8(hasher, tokenizer_tag(backend.capabilities.tokenizer));
    }

    update_len(hasher, inputs.profile_costs.len());
    for (key, profile) in &inputs.profile_costs {
        update_analysis_node_key(hasher, *key);
        update_u64(hasher, profile.latency_ms);
        update_u64(hasher, profile.sample_count);
        update_optional_u64(hasher, profile.dynamic_tokens);
    }

    update_len(hasher, inputs.execution_readiness.len());
    for (key, readiness) in &inputs.execution_readiness {
        update_analysis_node_key(hasher, *key);
        update_len(hasher, readiness.ready_dependency_tokens.len());
        for token in &readiness.ready_dependency_tokens {
            update_u64(hasher, *token);
        }
        match &readiness.active_grant {
            Some(grant) => {
                update_u8(hasher, 1);
                update_string(hasher, &grant.grant_id);
                update_string(hasher, &grant.capability);
                update_len(hasher, grant.operations.len());
                for operation in &grant.operations {
                    update_u8(hasher, permission_operation_tag(*operation));
                }
            }
            None => update_u8(hasher, 0),
        }
        update_u8(
            hasher,
            match readiness.approval {
                ApprovalEvidence::Unknown => 0,
                ApprovalEvidence::Approved => 1,
                ApprovalEvidence::Rejected => 2,
            },
        );
    }

    update_len(hasher, inputs.confined_nodes.len());
    for key in &inputs.confined_nodes {
        update_analysis_node_key(hasher, *key);
    }
}

/// Stable hash tag for tokenizer evidence.
fn tokenizer_tag(tokenizer: TokenizerEvidence) -> u8 {
    match tokenizer {
        TokenizerEvidence::Unknown => 0,
        TokenizerEvidence::Cl100kBase => 1,
        TokenizerEvidence::O200kBase => 2,
    }
}

/// Hash one stable node address.
fn update_analysis_node_key(hasher: &mut blake3::Hasher, key: AnalysisNodeKey) {
    update_len(hasher, key.dag_index);
    update_u64(hasher, key.node_id);
}

/// Stable hash tag for one permission operation.
fn permission_operation_tag(operation: PermissionOperation) -> u8 {
    match operation {
        PermissionOperation::Read => 0,
        PermissionOperation::List => 1,
        PermissionOperation::Search => 2,
        PermissionOperation::Create => 3,
        PermissionOperation::Write => 4,
        PermissionOperation::Append => 5,
        PermissionOperation::Update => 6,
        PermissionOperation::Delete => 7,
        PermissionOperation::Execute => 8,
        PermissionOperation::Send => 9,
        PermissionOperation::Approve => 10,
        PermissionOperation::Publish => 11,
    }
}

/// Update a hash with one typed artifact value.
fn update_value(hasher: &mut blake3::Hasher, value: &Value) {
    match value {
        Value::Null => update_u8(hasher, 0),
        Value::Bool(value) => {
            update_u8(hasher, 1);
            update_bool(hasher, *value);
        }
        Value::Number(Number::Integer(value)) => {
            update_u8(hasher, 2);
            hasher.update(&value.to_le_bytes());
        }
        Value::Number(Number::Float(value)) => {
            update_u8(hasher, 3);
            hasher.update(&value.to_bits().to_le_bytes());
        }
        Value::String(value) => {
            update_u8(hasher, 4);
            update_string(hasher, value);
        }
        Value::Array(values) => {
            update_u8(hasher, 5);
            update_len(hasher, values.len());
            for value in values {
                update_value(hasher, value);
            }
        }
        Value::Object(values) => {
            update_u8(hasher, 6);
            update_len(hasher, values.len());
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
            for (name, value) in entries {
                update_string(hasher, name);
                update_value(hasher, value);
            }
        }
        Value::Token(value) => {
            update_u8(hasher, 7);
            update_u64(hasher, *value);
        }
    }
}

/// Update a hash with a length-delimited byte slice.
fn update_bytes(hasher: &mut blake3::Hasher, value: &[u8]) {
    update_len(hasher, value.len());
    hasher.update(value);
}

/// Update a hash with one UTF-8 string.
fn update_string(hasher: &mut blake3::Hasher, value: &str) {
    update_bytes(hasher, value.as_bytes());
}

/// Update a hash with an optional string.
fn update_optional_string(hasher: &mut blake3::Hasher, value: Option<&str>) {
    match value {
        Some(value) => {
            update_u8(hasher, 1);
            update_string(hasher, value);
        }
        None => update_u8(hasher, 0),
    }
}

/// Update a hash with an optional unsigned integer.
fn update_optional_u64(hasher: &mut blake3::Hasher, value: Option<u64>) {
    match value {
        Some(value) => {
            update_u8(hasher, 1);
            update_u64(hasher, value);
        }
        None => update_u8(hasher, 0),
    }
}

/// Update a hash with an ordered token sequence.
fn update_u64_slice(hasher: &mut blake3::Hasher, values: &[u64]) {
    update_len(hasher, values.len());
    for value in values {
        update_u64(hasher, *value);
    }
}

/// Update a hash with a collection length.
fn update_len(hasher: &mut blake3::Hasher, value: usize) {
    update_u64(hasher, value as u64);
}

/// Update a hash with a boolean.
fn update_bool(hasher: &mut blake3::Hasher, value: bool) {
    update_u8(hasher, u8::from(value));
}

/// Update a hash with one byte.
fn update_u8(hasher: &mut blake3::Hasher, value: u8) {
    hasher.update(&[value]);
}

/// Update a hash with an unsigned 32-bit integer.
fn update_u32(hasher: &mut blake3::Hasher, value: u32) {
    hasher.update(&value.to_le_bytes());
}

/// Update a hash with an unsigned 64-bit integer.
fn update_u64(hasher: &mut blake3::Hasher, value: u64) {
    hasher.update(&value.to_le_bytes());
}
