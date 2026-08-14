//! APXM-owned graph facts and advisory intents.
//!
//! Provider mechanisms (pins, slots, queue priority, `vllm_xargs`) do not
//! belong here. Adapters project this contract onto an `apxm` server branch.

use crate::types::values::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const GRAPH_HINTS_SCHEMA: &str = "apxm.inference-graph-hints";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphHintScope {
    pub graph_ref: String,
    pub graph_execution_ref: String,
    pub node_ref: String,
    pub node_execution_ref: String,
}

impl GraphHintScope {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("graph_ref", &self.graph_ref),
            ("graph_execution_ref", &self.graph_execution_ref),
            ("node_ref", &self.node_ref),
            ("node_execution_ref", &self.node_execution_ref),
        ] {
            if value.trim().is_empty() {
                return Err(format!("{name} must be a non-empty opaque reference"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkClass {
    Short,
    Medium,
    Long,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NodeGraphFacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical_path: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub successor_refs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_path_len: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_class: Option<WorkClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_input_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_shared_prefix_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix_warmup_eligible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline_eligible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coexecution_group_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationObjective {
    MinimizeGraphCompletionTime,
    Balanced,
    MaximizeThroughput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReusePreference {
    PreferWhenBeneficial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReusableContextIntent {
    pub preference: ReusePreference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affinity_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benefit_horizon_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_uses: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GraphExecutionIntents {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<OptimizationObjective>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reusable_context: Option<ReusableContextIntent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApxmGraphHints {
    pub schema: String,
    pub scope: GraphHintScope,
    #[serde(default)]
    pub facts: NodeGraphFacts,
    #[serde(default)]
    pub intents: GraphExecutionIntents,
}

impl Default for ApxmGraphHints {
    fn default() -> Self {
        Self {
            schema: GRAPH_HINTS_SCHEMA.to_owned(),
            scope: GraphHintScope {
                graph_ref: "graph:unspecified".into(),
                graph_execution_ref: "graph-execution:unspecified".into(),
                node_ref: "node:unspecified".into(),
                node_execution_ref: "node-execution:unspecified".into(),
            },
            facts: NodeGraphFacts::default(),
            intents: GraphExecutionIntents::default(),
        }
    }
}

impl ApxmGraphHints {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != GRAPH_HINTS_SCHEMA {
            return Err(format!("unknown graph-hints schema: {}", self.schema));
        }
        self.scope.validate()
    }

    pub fn has_graph_context(&self) -> bool {
        self.scope.validate().is_ok()
    }

    pub fn critical_path(
        graph_ref: impl Into<String>,
        graph_execution_ref: impl Into<String>,
        node_ref: impl Into<String>,
        node_execution_ref: impl Into<String>,
        successor_refs: Vec<String>,
    ) -> Self {
        Self {
            schema: GRAPH_HINTS_SCHEMA.to_owned(),
            scope: GraphHintScope {
                graph_ref: graph_ref.into(),
                graph_execution_ref: graph_execution_ref.into(),
                node_ref: node_ref.into(),
                node_execution_ref: node_execution_ref.into(),
            },
            facts: NodeGraphFacts {
                critical_path: Some(true),
                successor_refs,
                ..NodeGraphFacts::default()
            },
            intents: GraphExecutionIntents {
                objective: Some(OptimizationObjective::MinimizeGraphCompletionTime),
                reusable_context: Some(ReusableContextIntent {
                    preference: ReusePreference::PreferWhenBeneficial,
                    affinity_ref: None,
                    benefit_horizon_ms: None,
                    expected_uses: None,
                }),
            },
        }
    }

    pub fn prefers_reuse(&self) -> bool {
        matches!(
            self.intents.reusable_context.as_ref().map(|c| c.preference),
            Some(ReusePreference::PreferWhenBeneficial)
        )
    }

    pub fn from_node_attrs(
        graph_id: String,
        node_label: String,
        attrs_map: &HashMap<String, Value>,
    ) -> Self {
        Self::try_from_node_attrs(graph_id, node_label, attrs_map)
            .expect("invalid compiler-derived graph hint attributes")
    }

    pub fn try_from_node_attrs(
        graph_id: String,
        node_label: String,
        attrs_map: &HashMap<String, Value>,
    ) -> Result<Self, String> {
        use crate::constants::graph::attrs;
        use crate::constants::graph::metadata as graph_meta;

        let critical_path = attrs_map.get(attrs::PRIORITY).and_then(|value| {
            let priority = value.as_i64()?;
            Some(priority >= graph_meta::CRITICAL_PATH_PRIORITY_THRESHOLD)
        });

        let successor_refs = attrs_map
            .get(attrs::DOWNSTREAM_NODES)
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| {
                        value
                            .as_u64()
                            .map(|raw| format!("node:{raw}"))
                            .or_else(|| value.as_string().map(ToOwned::to_owned))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let affinity_ref = attrs_map
            .get(attrs::REUSE_GROUP)
            .and_then(|value| value.as_string())
            .map(ToOwned::to_owned);

        let expected_shared_prefix_tokens = attrs_map
            .get(attrs::SHARED_PREFIX_EST_TOKENS)
            .and_then(|value| value.as_u64())
            .map(|value| value as u32);

        let prefix_warmup_eligible = attrs_map
            .get(attrs::WARMUP_CANDIDATE)
            .and_then(|value| value.as_bool());

        let reusable_context = affinity_ref.as_ref().map(|affinity_ref| ReusableContextIntent {
            preference: ReusePreference::PreferWhenBeneficial,
            affinity_ref: Some(affinity_ref.clone()),
            benefit_horizon_ms: None,
            expected_uses: None,
        });

        let hints = Self {
            schema: GRAPH_HINTS_SCHEMA.to_owned(),
            scope: GraphHintScope {
                graph_ref: graph_id,
                graph_execution_ref: "graph-execution:materialize".into(),
                node_ref: node_label.clone(),
                node_execution_ref: format!("node-execution:{node_label}"),
            },
            facts: NodeGraphFacts {
                critical_path,
                successor_refs,
                expected_shared_prefix_tokens,
                prefix_warmup_eligible,
                ..NodeGraphFacts::default()
            },
            intents: GraphExecutionIntents {
                objective: critical_path.and_then(|critical| {
                    critical.then_some(OptimizationObjective::MinimizeGraphCompletionTime)
                }),
                reusable_context,
            },
        };
        hints.validate()?;
        Ok(hints)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphHintField {
    Scope,
    CriticalPath,
    SuccessorRefs,
    RemainingPathLen,
    StageIndex,
    WorkClass,
    EstimatedInputTokens,
    EstimatedOutputTokens,
    ExpectedSharedPrefixTokens,
    PrefixWarmupEligible,
    PipelineEligible,
    CoexecutionGroupRef,
    Objective,
    ReusePreference,
    AffinityRef,
    BenefitHorizonMs,
    ExpectedUses,
}

impl GraphHintField {
    pub const ALL: &'static [Self] = &[
        Self::Scope,
        Self::CriticalPath,
        Self::SuccessorRefs,
        Self::RemainingPathLen,
        Self::StageIndex,
        Self::WorkClass,
        Self::EstimatedInputTokens,
        Self::EstimatedOutputTokens,
        Self::ExpectedSharedPrefixTokens,
        Self::PrefixWarmupEligible,
        Self::PipelineEligible,
        Self::CoexecutionGroupRef,
        Self::Objective,
        Self::ReusePreference,
        Self::AffinityRef,
        Self::BenefitHorizonMs,
        Self::ExpectedUses,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    AdapterProjection,
    BackendAcknowledgement,
    OutcomeMeasurement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphHintFieldCapability {
    Direct { evidence: BTreeSet<EvidenceKind> },
    Derived { evidence: BTreeSet<EvidenceKind> },
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphLifecycleCapability {
    PrepareRelease,
    NotNeeded,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphHintCapabilities {
    pub fields: BTreeMap<GraphHintField, GraphHintFieldCapability>,
    pub lifecycle: GraphLifecycleCapability,
}

impl GraphHintCapabilities {
    pub fn none() -> Self {
        Self {
            fields: GraphHintField::ALL
                .iter()
                .map(|field| (*field, GraphHintFieldCapability::Unsupported))
                .collect(),
            lifecycle: GraphLifecycleCapability::NotNeeded,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionOutcome {
    Applied { mechanism_ref: String },
    Approximated { mechanism_ref: String, reason: String },
    OmittedUnsupported,
    OmittedByProfile { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphHintPlan {
    pub outcomes: BTreeMap<GraphHintField, ProjectionOutcome>,
}

impl GraphHintPlan {
    pub fn omitted_unsupported() -> Self {
        Self {
            outcomes: GraphHintField::ALL
                .iter()
                .map(|field| (*field, ProjectionOutcome::OmittedUnsupported))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Acknowledgement {
    BackendAcknowledged,
    BackendRejected,
    NotReported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldRealization {
    pub projected: bool,
    pub acknowledgement: Acknowledgement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphHintRealization {
    pub fields: BTreeMap<GraphHintField, FieldRealization>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measurements: Vec<(String, i64)>,
}

pub trait GraphHintProjector {
    fn graph_hint_capabilities(&self) -> GraphHintCapabilities {
        GraphHintCapabilities::none()
    }

    fn plan_graph_hints(&self, hints: Option<&ApxmGraphHints>) -> Result<GraphHintPlan, String> {
        match hints {
            None => Ok(GraphHintPlan {
                outcomes: BTreeMap::new(),
            }),
            Some(hints) => {
                hints.validate()?;
                Ok(GraphHintPlan::omitted_unsupported())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphLifecycleOutcome {
    Prepared,
    Released,
    NotNeeded,
    Unsupported,
    FailedBeforeSend,
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BackendGraphCapabilities {
    pub supports_graph_registration: bool,
    pub supports_request_hints: bool,
    pub supports_priority: bool,
    pub supports_prefix_cohorts: bool,
    pub supports_pin_release: bool,
    pub supports_structured_outputs: bool,
    pub supports_backend_queue_state: bool,
    pub supports_backend_cache_state: bool,
    pub supports_cancel_groups: bool,
    pub supports_dispatch_ir_v1_internal: bool,
    pub supports_admin_reset_prefix_cache: bool,
}

impl BackendGraphCapabilities {
    pub fn unsupported_dispatch_fields<'a>(
        &self,
        fields_sent: impl IntoIterator<Item = &'a str>,
    ) -> Vec<String> {
        fields_sent
            .into_iter()
            .filter(|field| !self.field_supported(field))
            .map(ToOwned::to_owned)
            .collect()
    }

    pub fn dispatch_fields_capability_supported<'a>(
        &self,
        fields_sent: impl IntoIterator<Item = &'a str>,
    ) -> Vec<String> {
        fields_sent
            .into_iter()
            .filter(|field| self.field_supported(field))
            .map(ToOwned::to_owned)
            .collect()
    }

    fn field_supported(&self, field: &str) -> bool {
        use crate::constants::llm::apxm::dispatch_fields as df;
        match field {
            df::GRAPH_REGISTRATION => self.supports_graph_registration,
            df::REQUEST_HINTS => self.supports_request_hints,
            df::PRIORITY => self.supports_priority,
            df::PREFIX_COHORTS => self.supports_prefix_cohorts,
            df::PIN_RELEASE => self.supports_pin_release,
            df::STRUCTURED_OUTPUTS => self.supports_structured_outputs,
            df::BACKEND_QUEUE_STATE => self.supports_backend_queue_state,
            df::BACKEND_CACHE_STATE => self.supports_backend_cache_state,
            df::CANCEL_GROUPS => self.supports_cancel_groups,
            df::DISPATCH_IR_V1_INTERNAL => self.supports_dispatch_ir_v1_internal,
            df::ADMIN_RESET_PREFIX_CACHE => self.supports_admin_reset_prefix_cache,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSpec {
    #[serde(alias = "node_id", alias = "node_name")]
    pub node_ref: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub successor_refs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_input_tokens: Option<u32>,
    pub is_critical_path: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMetadata {
    #[serde(rename = "graph_id", alias = "graph_ref")]
    pub graph_ref: String,
    #[serde(rename = "execution_id", alias = "graph_execution_ref")]
    pub graph_execution_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical_path_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<u32>,
    pub nodes: Vec<NodeSpec>,
}

impl GraphMetadata {
    pub fn new(graph_ref: impl Into<String>, graph_execution_ref: impl Into<String>) -> Self {
        Self {
            graph_ref: graph_ref.into(),
            graph_execution_ref: Some(graph_execution_ref.into()),
            critical_path_length: None,
            node_count: None,
            max_parallelism: None,
            nodes: Vec::new(),
        }
    }

    pub fn with_critical_path_length(mut self, len: u32) -> Self {
        self.critical_path_length = Some(len);
        self
    }

    pub fn with_nodes(mut self, nodes: Vec<NodeSpec>) -> Self {
        self.node_count = Some(nodes.len() as u32);
        self.nodes = nodes;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphStatusSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_name: Option<String>,
    pub graph_ref: String,
    pub registered: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_observations: BTreeMap<String, u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical_path_length: Option<u64>,
}

impl GraphStatusSnapshot {
    pub fn new(graph_ref: impl Into<String>) -> Self {
        Self {
            backend_name: None,
            graph_ref: graph_ref.into(),
            registered: false,
            adapter_observations: BTreeMap::new(),
            node_count: None,
            critical_path_length: None,
        }
    }

    pub fn graph_aware(graph_id: impl Into<String>) -> Self {
        Self::new(graph_id).with_registered(true)
    }

    pub fn pinned_handles(&self) -> u64 {
        self.adapter_observations
            .get("vllm.pinned_handles")
            .copied()
            .unwrap_or(0)
    }

    pub fn pinned_blocks(&self) -> u64 {
        self.adapter_observations
            .get("vllm.pinned_blocks")
            .copied()
            .unwrap_or(0)
    }

    pub fn with_registered(mut self, registered: bool) -> Self {
        self.registered = registered;
        self
    }

    pub fn with_backend_name(mut self, backend_name: impl Into<String>) -> Self {
        self.backend_name = Some(backend_name.into());
        self
    }

    pub fn with_pin_counts(mut self, handles: u64, blocks: u64) -> Self {
        self.adapter_observations
            .insert("vllm.pinned_handles".into(), handles);
        self.adapter_observations
            .insert("vllm.pinned_blocks".into(), blocks);
        self
    }

    pub fn with_pin_peaks(mut self, handles_peak: u64, blocks_peak: u64) -> Self {
        self.adapter_observations
            .insert("vllm.pinned_handles_peak".into(), handles_peak);
        self.adapter_observations
            .insert("vllm.pinned_blocks_peak".into(), blocks_peak);
        self
    }

    pub fn with_shape(
        mut self,
        node_count: Option<u64>,
        critical_path_length: Option<u64>,
    ) -> Self {
        self.node_count = node_count;
        self.critical_path_length = critical_path_length;
        self
    }

    pub fn to_metrics_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_scope_refs_fail_closed() {
        let hints = ApxmGraphHints {
            schema: GRAPH_HINTS_SCHEMA.into(),
            scope: GraphHintScope {
                graph_ref: " ".into(),
                graph_execution_ref: "gex".into(),
                node_ref: "n".into(),
                node_execution_ref: "nex".into(),
            },
            facts: NodeGraphFacts::default(),
            intents: GraphExecutionIntents::default(),
        };
        assert!(hints.validate().is_err());
    }

    #[test]
    fn zero_capability_plan_omits_every_field() {
        struct Zero;
        impl GraphHintProjector for Zero {}
        let hints = ApxmGraphHints::default();
        let plan = Zero.plan_graph_hints(Some(&hints)).expect("plan");
        assert_eq!(plan.outcomes.len(), GraphHintField::ALL.len());
        assert!(plan
            .outcomes
            .values()
            .all(|outcome| matches!(outcome, ProjectionOutcome::OmittedUnsupported)));
    }
}
