//! Additive, conservative optimization evidence derived from execution DAGs.

mod backend_legality;
mod dag_use;
mod effect_authority;
mod evidence;
mod profile_cost;
mod prompt_contract;
mod store;
mod token_cost;

use apxm_core::types::compiler::OptimizationSummaryV1;
use apxm_core::types::execution::ExecutionDag;

pub use evidence::{
    ActiveGrantEvidence, AnalysisNodeKey, ApprovalEvidence, BackendCapabilityEvidence,
    CompilerAnalysisInputs, ConfiguredBackendEvidence, ExecutionReadinessEvidence,
    ProfileCostEvidence, TokenizerEvidence,
};
pub(crate) use store::AnalysisStore;

/// Finalize execution guards and build the versioned summary for an artifact.
#[cfg(test)]
pub(crate) fn finalize_artifact(dags: &mut [ExecutionDag]) -> OptimizationSummaryV1 {
    finalize_artifact_with_inputs(dags, &CompilerAnalysisInputs::default())
}

/// Finalize execution guards using caller-supplied backend, profile, and readiness evidence.
pub(crate) fn finalize_artifact_with_inputs(
    dags: &mut [ExecutionDag],
    inputs: &CompilerAnalysisInputs,
) -> OptimizationSummaryV1 {
    let mut store = AnalysisStore::with_inputs(dags.to_vec(), inputs.clone());
    store.apply_memoization_guards();
    store.apply_batching_guards();
    let summary = store.summary();
    let finalized = store.into_dags();
    dags.clone_from_slice(&finalized);
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::constants::graph::attrs as graph_attrs;
    use apxm_core::types::execution::{Edge, Node};
    use apxm_core::types::{AISOperationType, DependencyType, Value};

    #[test]
    fn capability_invocation_fails_closed_for_reordering_and_memoization() {
        let mut dag = ExecutionDag::new();
        dag.nodes.push(Node::new(1, AISOperationType::InvCap));

        let mut dags = vec![dag];
        let summary = finalize_artifact(&mut dags);
        let legality = &summary.dags[0].nodes[0].legality;
        assert!(!legality.may_reorder);
        assert!(!legality.may_duplicate);
        assert!(!legality.may_memoize);
    }

    #[test]
    fn critical_path_uses_node_latency_and_typed_edges() {
        let mut first = Node::new(1, AISOperationType::Nop);
        first.metadata.estimated_latency = Some(2_000_000);
        let mut second = Node::new(2, AISOperationType::Nop);
        second.metadata.estimated_latency = Some(3_000_000);
        let mut dag = ExecutionDag::new();
        dag.nodes = vec![first, second];
        dag.edges.push(Edge::new(1, 2, 1, DependencyType::Effect));

        let mut dags = vec![dag];
        let summary = finalize_artifact(&mut dags);
        assert_eq!(summary.dags[0].effect_edges, 1);
        assert_eq!(summary.dags[0].weighted_critical_path_ms, 5);
    }

    #[test]
    fn deterministic_ask_requires_configured_backend_evidence_for_exact_memoization() {
        let mut ask = Node::new(1, AISOperationType::Ask);
        ask.attributes.insert(
            graph_attrs::TEMPERATURE.to_string(),
            Value::Number(0_i64.into()),
        );
        ask.attributes.insert(
            graph_attrs::BACKEND.to_string(),
            Value::String("configured-backend".to_string()),
        );
        ask.attributes.insert(
            graph_attrs::MODEL.to_string(),
            Value::String("configured-model".to_string()),
        );
        let mut dags = vec![ExecutionDag {
            nodes: vec![ask],
            ..Default::default()
        }];
        let mut without_backend_evidence = dags.clone();

        let summary = finalize_artifact(&mut without_backend_evidence);

        assert!(!summary.dags[0].nodes[0].legality.may_memoize);
        assert_eq!(
            without_backend_evidence[0].nodes[0]
                .get_attribute(graph_attrs::MEMOIZABLE)
                .and_then(Value::as_bool),
            Some(false)
        );

        let inputs = CompilerAnalysisInputs {
            configured_backends: vec![ConfiguredBackendEvidence {
                backend: "configured-backend".to_string(),
                model: "configured-model".to_string(),
                aliases: Default::default(),
                available: true,
                capabilities: BackendCapabilityEvidence {
                    custom_temperature: true,
                    ..Default::default()
                },
            }],
            ..Default::default()
        };
        let summary = finalize_artifact_with_inputs(&mut dags, &inputs);

        assert!(summary.dags[0].nodes[0].legality.may_memoize);
        assert_eq!(
            dags[0].nodes[0]
                .get_attribute(graph_attrs::MEMOIZABLE)
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn configured_independent_model_calls_receive_batch_groups_and_explanations() {
        let model_call = |node_id| {
            let mut node = Node::new(node_id, AISOperationType::Ask);
            node.attributes.insert(
                graph_attrs::TEMPERATURE.to_string(),
                Value::Number(0_i64.into()),
            );
            node.attributes.insert(
                graph_attrs::BACKEND.to_string(),
                Value::String("configured-backend".to_string()),
            );
            node.attributes.insert(
                graph_attrs::MODEL.to_string(),
                Value::String("configured-model".to_string()),
            );
            node
        };
        let mut dags = vec![ExecutionDag {
            nodes: vec![model_call(1), model_call(2)],
            ..Default::default()
        }];
        let inputs = CompilerAnalysisInputs {
            configured_backends: vec![ConfiguredBackendEvidence {
                backend: "configured-backend".to_string(),
                model: "configured-model".to_string(),
                aliases: Default::default(),
                available: true,
                capabilities: BackendCapabilityEvidence {
                    batching: true,
                    custom_temperature: true,
                    ..Default::default()
                },
            }],
            ..Default::default()
        };

        let summary = finalize_artifact_with_inputs(&mut dags, &inputs);
        let groups: Vec<_> = dags[0]
            .nodes
            .iter()
            .map(|node| {
                node.get_attribute(graph_attrs::BATCH_GROUP)
                    .and_then(Value::as_str)
                    .expect("compiler batch group")
            })
            .collect();
        assert_eq!(groups[0], groups[1]);

        for node in &summary.dags[0].nodes {
            assert!(node.legality.may_batch);
            assert!(node.legality.may_memoize);
            assert_eq!(
                node.decisions.len(),
                apxm_core::types::compiler::OptimizationTransformKind::ALL.len()
            );
            assert!(
                node.decisions
                    .iter()
                    .all(|decision| !decision.reasons.is_empty())
            );
            let batching = node
                .decisions
                .iter()
                .find(|decision| {
                    decision.transform
                        == apxm_core::types::compiler::OptimizationTransformKind::Batching
                })
                .expect("batching decision");
            assert_eq!(
                batching.disposition,
                apxm_core::types::compiler::OptimizationDisposition::Applied
            );
            let speculation = node
                .decisions
                .iter()
                .find(|decision| {
                    decision.transform
                        == apxm_core::types::compiler::OptimizationTransformKind::Speculation
                })
                .expect("speculation decision");
            assert_eq!(
                speculation.disposition,
                apxm_core::types::compiler::OptimizationDisposition::Experimental
            );
        }
    }

    #[test]
    fn profile_cost_is_available_only_with_sampled_typed_evidence() {
        let mut first = Node::new(1, AISOperationType::Nop);
        first.metadata.estimated_latency = Some(1_000_000);
        let mut second = Node::new(2, AISOperationType::Nop);
        second.metadata.estimated_latency = Some(1_000_000);
        let mut dags = vec![ExecutionDag {
            nodes: vec![first, second],
            edges: vec![Edge::new(1, 2, 11, DependencyType::Data)],
            ..Default::default()
        }];
        let inputs = CompilerAnalysisInputs {
            profile_costs: [
                (
                    AnalysisNodeKey {
                        dag_index: 0,
                        node_id: 1,
                    },
                    ProfileCostEvidence {
                        latency_ms: 7,
                        sample_count: 3,
                        dynamic_tokens: Some(5),
                    },
                ),
                (
                    AnalysisNodeKey {
                        dag_index: 0,
                        node_id: 2,
                    },
                    ProfileCostEvidence {
                        latency_ms: 11,
                        sample_count: 4,
                        dynamic_tokens: None,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        let summary = finalize_artifact_with_inputs(&mut dags, &inputs);

        assert_eq!(summary.dags[0].weighted_critical_path_ms, 18);
        assert_eq!(summary.dags[0].nodes[0].cost.sample_count, 3);
        assert_eq!(summary.dags[0].nodes[0].cost.estimated_dynamic_tokens, 5);
        assert_eq!(
            summary.dags[0].nodes[0].cost.latency_provenance,
            apxm_core::types::compiler::CostProvenance::Observed
        );
    }

    #[test]
    fn tool_or_effect_bound_ask_is_never_stamped_memoizable() {
        let mut tool_ask = Node::new(1, AISOperationType::Ask);
        tool_ask.attributes.insert(
            graph_attrs::TEMPERATURE.to_string(),
            Value::Number(0_i64.into()),
        );
        tool_ask
            .attributes
            .insert(graph_attrs::TOOLS_ENABLED.to_string(), Value::Bool(true));
        tool_ask
            .attributes
            .insert(graph_attrs::MEMOIZABLE.to_string(), Value::Bool(true));

        let mut effect = Node::new(2, AISOperationType::InvCap);
        effect.attributes.insert(
            graph_attrs::CAPABILITY.to_string(),
            Value::String("lookup".to_string()),
        );
        let mut dependent_ask = Node::new(3, AISOperationType::Ask);
        dependent_ask.attributes.insert(
            graph_attrs::TEMPERATURE.to_string(),
            Value::Number(0_i64.into()),
        );
        dependent_ask
            .attributes
            .insert(graph_attrs::MEMOIZABLE.to_string(), Value::Bool(true));

        let mut dag = ExecutionDag {
            nodes: vec![tool_ask, effect, dependent_ask],
            ..Default::default()
        };
        dag.edges.push(Edge::new(2, 3, 1, DependencyType::Data));
        let mut dags = vec![dag];

        let summary = finalize_artifact(&mut dags);

        assert!(!summary.dags[0].nodes[0].legality.may_memoize);
        assert!(!summary.dags[0].nodes[2].legality.may_memoize);
        for node in [0, 2] {
            assert_eq!(
                dags[0].nodes[node]
                    .get_attribute(graph_attrs::MEMOIZABLE)
                    .and_then(Value::as_bool),
                Some(false)
            );
        }
    }
}
