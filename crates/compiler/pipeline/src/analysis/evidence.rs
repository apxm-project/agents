//! Typed external evidence consumed by conservative compiler analyses.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use apxm_backends::llm::BackendConfig;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::compiler::{BackendLegalityRequirements, EffectAuthoritySummary};
use apxm_core::types::execution::{ExecutionDag, Node, NodeId};
use apxm_core::types::{AISOperationType, PermissionOperation, Value};

/// Stable address of one node in a multi-DAG artifact analysis snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AnalysisNodeKey {
    /// Zero-based position of the DAG in the analyzed artifact.
    pub dag_index: usize,
    /// Stable node identifier within that DAG.
    pub node_id: NodeId,
}

/// Explicit capabilities for one configured backend/model route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackendCapabilityEvidence {
    /// The route accepts model-directed tool invocation.
    pub tools: bool,
    /// The route enforces structured-output schemas.
    pub structured_output: bool,
    /// The route supports backend-managed prompt-prefix reuse.
    pub prefix_reuse: bool,
    /// The route supports extended reasoning requests.
    pub thinking: bool,
    /// The route supports submitting independent requests as one batch.
    pub batching: bool,
    /// The route accepts an explicit temperature value.
    pub custom_temperature: bool,
    /// Maximum context window declared by the configured model.
    pub context_window: Option<u64>,
    /// Maximum output tokens declared by the configured model.
    pub max_output_tokens: Option<u64>,
    /// Explicit tokenizer used for compiler-side token accounting.
    pub tokenizer: TokenizerEvidence,
}

/// Tokenizer capability declared by one configured backend/model route.
///
/// Token accounting is unavailable until the owning route explicitly declares
/// one of the supported encodings. Model spelling is not tokenizer evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TokenizerEvidence {
    /// The configured route did not provide tokenizer capability evidence.
    #[default]
    Unknown,
    /// The route uses OpenAI's `cl100k_base` encoding.
    Cl100kBase,
    /// The route uses OpenAI's `o200k_base` encoding.
    O200kBase,
}

impl TokenizerEvidence {
    /// Return the declared encoding name when token accounting is available.
    pub fn name(self) -> Option<&'static str> {
        match self {
            Self::Unknown => None,
            Self::Cl100kBase => Some("cl100k_base"),
            Self::O200kBase => Some("o200k_base"),
        }
    }
}

/// Availability and capabilities for one configured backend/model route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredBackendEvidence {
    /// Backend identifier from the configured catalog.
    pub backend: String,
    /// Canonical configured model identifier.
    pub model: String,
    /// Configured aliases accepted for the canonical model.
    pub aliases: BTreeSet<String>,
    /// Whether the configured route is available to this compiler invocation.
    pub available: bool,
    /// Capability evidence supplied by the configuration or owning registry.
    pub capabilities: BackendCapabilityEvidence,
}

/// Sampled profile-cost evidence for one artifact node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileCostEvidence {
    /// Observed end-to-end latency in milliseconds.
    pub latency_ms: u64,
    /// Number of runtime samples backing the observation.
    pub sample_count: u64,
    /// Observed dynamic-token mean when the profile recorded one.
    pub dynamic_tokens: Option<u64>,
}

/// Typed active-grant projection for one operation readiness check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveGrantEvidence {
    /// Runtime-minted grant identity.
    pub grant_id: String,
    /// Capability authorized by this active grant.
    pub capability: String,
    /// Permission operations authorized by this active grant.
    pub operations: BTreeSet<PermissionOperation>,
}

/// Approval state supplied by the owner that evaluates the approval policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApprovalEvidence {
    /// No typed approval result was supplied.
    #[default]
    Unknown,
    /// The owning approval policy admitted the operation.
    Approved,
    /// The owning approval policy rejected or expired the operation.
    Rejected,
}

/// Runtime readiness facts required before an operation can execute.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionReadinessEvidence {
    /// Typed dependency tokens already made ready by their producers.
    pub ready_dependency_tokens: BTreeSet<u64>,
    /// Active grant selected for an authority-bound operation.
    pub active_grant: Option<ActiveGrantEvidence>,
    /// Typed approval result for an approval-bound operation.
    pub approval: ApprovalEvidence,
}

/// External, typed facts consumed by reusable compiler analyses.
///
/// The compiler never selects a backend, assumes a profile, or fabricates a
/// grant, approval, or ready dependency. Missing evidence keeps affected
/// optimizations unavailable instead of introducing a provider-specific fallback.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompilerAnalysisInputs {
    /// Configured backend/model routes available to this compiler invocation.
    pub configured_backends: Vec<ConfiguredBackendEvidence>,
    /// Sampled profile costs keyed by the emitted artifact node.
    pub profile_costs: BTreeMap<AnalysisNodeKey, ProfileCostEvidence>,
    /// Active grants, approvals, and dependency readiness keyed by node.
    pub execution_readiness: BTreeMap<AnalysisNodeKey, ExecutionReadinessEvidence>,
    /// Nodes whose owning execution surface proved required isolation controls.
    pub confined_nodes: BTreeSet<AnalysisNodeKey>,
}

impl CompilerAnalysisInputs {
    /// Derive route evidence from the configured backend catalog.
    ///
    /// The catalog does not describe backend-managed prefix reuse, so that
    /// capability remains false until an owning registry supplies it explicitly.
    pub fn from_backend_configs(backends: &[BackendConfig]) -> Self {
        let configured_backends = backends
            .iter()
            .flat_map(|backend| {
                backend.models.iter().map(move |model| {
                    let structured_output = model
                        .supports_structured_outputs
                        .or(backend.supports_structured_outputs)
                        .unwrap_or(false);
                    ConfiguredBackendEvidence {
                        backend: backend.name.clone(),
                        model: model.id.clone(),
                        aliases: model.aliases.iter().cloned().collect(),
                        available: true,
                        capabilities: BackendCapabilityEvidence {
                            tools: model.supports_functions,
                            structured_output,
                            prefix_reuse: false,
                            thinking: model.supports_thinking,
                            batching: false,
                            custom_temperature: model.supports_custom_temperature.unwrap_or(false),
                            context_window: positive_usize(model.context_window),
                            max_output_tokens: model.max_output_tokens.and_then(positive_usize),
                            tokenizer: TokenizerEvidence::Unknown,
                        },
                    }
                })
            })
            .collect();
        Self {
            configured_backends,
            ..Self::default()
        }
    }

    /// Return the sampled profile evidence for one emitted node.
    pub(super) fn profile_cost(&self, key: AnalysisNodeKey) -> Option<ProfileCostEvidence> {
        self.profile_costs
            .get(&key)
            .copied()
            .filter(|evidence| evidence.sample_count > 0)
    }

    /// Prove that the selected configured route supports the authored contract.
    pub(super) fn supports_backend_contract(
        &self,
        node: &Node,
        requirements: &BackendLegalityRequirements,
    ) -> bool {
        if !matches!(
            node.op_type,
            AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
        ) {
            return true;
        }

        let backend = string_attribute(node, graph_attrs::BACKEND);
        let model = string_attribute(node, graph_attrs::MODEL);
        let candidates: Vec<_> = self
            .configured_backends
            .iter()
            .filter(|candidate| {
                backend
                    .as_deref()
                    .is_none_or(|value| candidate.backend == value)
                    && model
                        .as_deref()
                        .is_none_or(|value| candidate.matches_model(value))
            })
            .collect();

        let Some(candidate) = candidates.first() else {
            return false;
        };
        if backend.is_none() && model.is_none() {
            return false;
        }
        if candidates
            .iter()
            .skip(1)
            .any(|other| other.backend != candidate.backend || other.model != candidate.model)
        {
            return false;
        }

        candidate.available
            && (!requirements.requires_tools || candidate.capabilities.tools)
            && (!requirements.requires_structured_output
                || candidate.capabilities.structured_output)
            && (!requirements.requires_prefix_reuse || candidate.capabilities.prefix_reuse)
            && (!requirements.requires_thinking || candidate.capabilities.thinking)
            && temperature_supported(node, candidate)
            && token_limits_supported(node, candidate)
    }

    /// Return tokenizer evidence for one unambiguous configured route.
    pub(crate) fn tokenizer_for_node(&self, node: &Node) -> Option<TokenizerEvidence> {
        let route = self.configured_backend_for(node)?;
        route
            .available
            .then_some(route.capabilities.tokenizer)
            .filter(|tokenizer| *tokenizer != TokenizerEvidence::Unknown)
    }

    /// Return tokenizer evidence for a pre-lowering node's explicit attributes.
    ///
    /// Air nodes and emitted execution nodes share the same route attributes.
    /// Keeping resolution on those attributes prevents either representation
    /// from deriving a tokenizer from provider or model spelling.
    pub(crate) fn tokenizer_for_attributes(
        &self,
        attributes: &HashMap<String, Value>,
    ) -> Option<TokenizerEvidence> {
        let route = self.configured_backend_for_attributes(attributes)?;
        route
            .available
            .then_some(route.capabilities.tokenizer)
            .filter(|tokenizer| *tokenizer != TokenizerEvidence::Unknown)
    }

    /// Whether both nodes resolve to the same configured backend/model route.
    pub(crate) fn same_configured_backend(&self, left: &Node, right: &Node) -> bool {
        let (Some(left), Some(right)) = (
            self.configured_backend_for(left),
            self.configured_backend_for(right),
        ) else {
            return false;
        };
        left.backend == right.backend && left.model == right.model
    }

    /// Whether the node resolves to an available route with explicit batching support.
    pub(super) fn supports_batching(&self, node: &Node) -> bool {
        self.configured_backend_for(node)
            .is_some_and(|route| route.available && route.capabilities.batching)
    }

    /// Stable configured route identity for compiler-emitted grouping metadata.
    pub(super) fn configured_route_identity(&self, node: &Node) -> Option<(&str, &str)> {
        let route = self.configured_backend_for(node)?;
        route
            .available
            .then_some((route.backend.as_str(), route.model.as_str()))
    }

    /// Whether required execution isolation was explicitly proven for the node.
    pub(super) fn confinement_satisfied(&self, key: AnalysisNodeKey, node: &Node) -> bool {
        !requires_confinement(node) || self.confined_nodes.contains(&key)
    }

    /// Resolve one configured route without inferring a backend or model.
    fn configured_backend_for(&self, node: &Node) -> Option<&ConfiguredBackendEvidence> {
        self.configured_backend_for_attributes(&node.attributes)
    }

    /// Resolve one configured route from explicit backend/model attributes.
    fn configured_backend_for_attributes(
        &self,
        attributes: &HashMap<String, Value>,
    ) -> Option<&ConfiguredBackendEvidence> {
        let backend = string_attribute_from(attributes, graph_attrs::BACKEND);
        let model = string_attribute_from(attributes, graph_attrs::MODEL);
        if backend.is_none() && model.is_none() {
            return None;
        }

        let candidates: Vec<_> = self
            .configured_backends
            .iter()
            .filter(|candidate| {
                backend
                    .as_deref()
                    .is_none_or(|value| candidate.backend == value)
                    && model
                        .as_deref()
                        .is_none_or(|value| candidate.matches_model(value))
            })
            .collect();
        let candidate = *candidates.first()?;
        candidates
            .iter()
            .skip(1)
            .all(|other| other.backend == candidate.backend && other.model == candidate.model)
            .then_some(candidate)
    }

    /// Prove that all typed runtime prerequisites for one node are ready.
    pub(super) fn execution_ready(
        &self,
        key: AnalysisNodeKey,
        dag: &ExecutionDag,
        node: &Node,
        effect: &EffectAuthoritySummary,
    ) -> bool {
        let incoming_tokens: Vec<_> = dag
            .edges
            .iter()
            .filter(|edge| edge.to == node.id)
            .map(|edge| edge.token_id)
            .collect();
        let requires_authority = !effect.permission_operations.is_empty();
        let requires_approval = effect.approval_required;
        if incoming_tokens.is_empty() && !requires_authority && !requires_approval {
            return true;
        }

        let Some(evidence) = self.execution_readiness.get(&key) else {
            return false;
        };
        if incoming_tokens
            .iter()
            .any(|token| !evidence.ready_dependency_tokens.contains(token))
        {
            return false;
        }
        if requires_authority {
            let Some(grant) = evidence.active_grant.as_ref() else {
                return false;
            };
            if grant.grant_id.is_empty()
                || !effect
                    .permission_operations
                    .iter()
                    .all(|operation| grant.operations.contains(operation))
            {
                return false;
            }
            if matches!(node.op_type, AISOperationType::InvCap)
                && string_attribute(node, graph_attrs::CAPABILITY)
                    .as_deref()
                    .is_none_or(|capability| capability != grant.capability)
            {
                return false;
            }
        }

        !requires_approval || evidence.approval == ApprovalEvidence::Approved
    }
}

/// Whether the operation can execute code or spawn an independently executing unit.
pub(super) fn requires_confinement(node: &Node) -> bool {
    matches!(
        node.op_type,
        AISOperationType::Exc
            | AISOperationType::WorkflowSpawn
            | AISOperationType::SpawnAgent
            | AISOperationType::Autonomous
    )
}

impl ConfiguredBackendEvidence {
    /// Whether this configured route resolves the authored model identifier.
    fn matches_model(&self, value: &str) -> bool {
        self.model == value || self.aliases.contains(value)
    }
}

/// Read one string-valued graph attribute without inventing a fallback.
fn string_attribute(node: &Node, attribute: &str) -> Option<String> {
    string_attribute_from(&node.attributes, attribute)
}

/// Read one string-valued attribute without introducing a fallback.
fn string_attribute_from(attributes: &HashMap<String, Value>, attribute: &str) -> Option<String> {
    attributes
        .get(attribute)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

/// Convert one positive platform-sized limit into stable compiler evidence.
fn positive_usize(value: usize) -> Option<u64> {
    if value == 0 {
        None
    } else {
        u64::try_from(value).ok()
    }
}

/// Whether an authored temperature value is accepted by the configured route.
fn temperature_supported(node: &Node, route: &ConfiguredBackendEvidence) -> bool {
    !node.attributes.contains_key(graph_attrs::TEMPERATURE) || route.capabilities.custom_temperature
}

/// Whether all authored token bounds fit the configured model limits.
fn token_limits_supported(node: &Node, route: &ConfiguredBackendEvidence) -> bool {
    let output_tokens = node
        .get_attribute(graph_attrs::TOKEN_BUDGET)
        .and_then(Value::as_u64);
    if let Some(output_tokens) = output_tokens
        && route
            .capabilities
            .max_output_tokens
            .is_none_or(|limit| output_tokens > limit)
    {
        return false;
    }

    let known_input_tokens = [
        graph_attrs::EST_TEMPLATE_TOKENS,
        graph_attrs::ESTIMATED_DYNAMIC_TOKENS,
    ]
    .into_iter()
    .filter_map(|attribute| node.get_attribute(attribute).and_then(Value::as_u64))
    .fold(0_u64, u64::saturating_add);
    let known_total = known_input_tokens.saturating_add(output_tokens.unwrap_or(0));
    known_total == 0
        || route
            .capabilities
            .context_window
            .is_some_and(|limit| known_total <= limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::execution::Edge;
    use apxm_core::types::{DependencyType, PermissionOperation};

    #[test]
    fn configured_backend_capabilities_gate_the_authored_contract() {
        let mut ask = Node::new(3, AISOperationType::Ask);
        ask.attributes.insert(
            graph_attrs::BACKEND.to_string(),
            Value::String("configured-backend".to_string()),
        );
        ask.attributes.insert(
            graph_attrs::MODEL.to_string(),
            Value::String("configured-model".to_string()),
        );
        ask.attributes.insert(
            graph_attrs::OUTPUT_SCHEMA.to_string(),
            Value::Object(Default::default()),
        );
        let requirements = BackendLegalityRequirements {
            requires_structured_output: true,
            ..Default::default()
        };
        let mut inputs = CompilerAnalysisInputs {
            configured_backends: vec![ConfiguredBackendEvidence {
                backend: "configured-backend".to_string(),
                model: "configured-model".to_string(),
                aliases: BTreeSet::new(),
                available: true,
                capabilities: BackendCapabilityEvidence::default(),
            }],
            ..Default::default()
        };

        assert!(!inputs.supports_backend_contract(&ask, &requirements));
        inputs.configured_backends[0].capabilities.structured_output = true;
        assert!(inputs.supports_backend_contract(&ask, &requirements));
    }

    #[test]
    fn readiness_requires_matching_active_grant_approval_and_dependency_tokens() {
        let mut invocation = Node::new(7, AISOperationType::InvCap);
        invocation.attributes.insert(
            graph_attrs::CAPABILITY.to_string(),
            Value::String("configured-capability".to_string()),
        );
        let dag = ExecutionDag {
            nodes: vec![Node::new(1, AISOperationType::Nop), invocation.clone()],
            edges: vec![Edge::new(1, 7, 11, DependencyType::Effect)],
            ..Default::default()
        };
        let effect = EffectAuthoritySummary {
            permission_operations: vec![PermissionOperation::Execute],
            approval_required: true,
            ..Default::default()
        };
        let key = AnalysisNodeKey {
            dag_index: 0,
            node_id: invocation.id,
        };
        let mut inputs = CompilerAnalysisInputs::default();
        assert!(!inputs.execution_ready(key, &dag, &invocation, &effect));

        inputs.execution_readiness.insert(
            key,
            ExecutionReadinessEvidence {
                ready_dependency_tokens: BTreeSet::new(),
                active_grant: Some(ActiveGrantEvidence {
                    grant_id: "grant-evidence".to_string(),
                    capability: "configured-capability".to_string(),
                    operations: BTreeSet::from([PermissionOperation::Execute]),
                }),
                approval: ApprovalEvidence::Approved,
            },
        );
        assert!(!inputs.execution_ready(key, &dag, &invocation, &effect));

        inputs
            .execution_readiness
            .get_mut(&key)
            .expect("readiness evidence")
            .ready_dependency_tokens
            .insert(11);
        assert!(inputs.execution_ready(key, &dag, &invocation, &effect));
    }

    #[test]
    fn backend_contract_checks_temperature_context_and_output_limits() {
        let mut ask = Node::new(5, AISOperationType::Ask);
        ask.attributes.insert(
            graph_attrs::BACKEND.to_string(),
            Value::String("configured-backend".to_string()),
        );
        ask.attributes.insert(
            graph_attrs::MODEL.to_string(),
            Value::String("configured-model".to_string()),
        );
        ask.attributes.insert(
            graph_attrs::TEMPERATURE.to_string(),
            Value::Number(0_i64.into()),
        );
        ask.attributes.insert(
            graph_attrs::EST_TEMPLATE_TOKENS.to_string(),
            Value::Number(20_i64.into()),
        );
        ask.attributes.insert(
            graph_attrs::TOKEN_BUDGET.to_string(),
            Value::Number(10_i64.into()),
        );
        let mut inputs = CompilerAnalysisInputs {
            configured_backends: vec![ConfiguredBackendEvidence {
                backend: "configured-backend".to_string(),
                model: "configured-model".to_string(),
                aliases: BTreeSet::new(),
                available: true,
                capabilities: BackendCapabilityEvidence {
                    custom_temperature: true,
                    context_window: Some(25),
                    max_output_tokens: Some(15),
                    ..Default::default()
                },
            }],
            ..Default::default()
        };

        assert!(!inputs.supports_backend_contract(&ask, &Default::default()));
        inputs.configured_backends[0].capabilities.context_window = Some(40);
        assert!(inputs.supports_backend_contract(&ask, &Default::default()));
        inputs.configured_backends[0].capabilities.max_output_tokens = Some(5);
        assert!(!inputs.supports_backend_contract(&ask, &Default::default()));
    }

    #[test]
    fn confinement_evidence_is_required_only_for_isolated_execution_ops() {
        let key = AnalysisNodeKey {
            dag_index: 0,
            node_id: 9,
        };
        let execution = Node::new(key.node_id, AISOperationType::Exc);
        let pure = Node::new(10, AISOperationType::Nop);
        let mut inputs = CompilerAnalysisInputs::default();

        assert!(!inputs.confinement_satisfied(key, &execution));
        assert!(inputs.confinement_satisfied(key, &pure));
        inputs.confined_nodes.insert(key);
        assert!(inputs.confinement_satisfied(key, &execution));
    }
}
