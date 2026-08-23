//! APXM-owned graph facts and advisory intents.
//!
//! Provider mechanisms (pins, slots, queue priority, provider extension
//! envelopes) do not belong here. An adapter *projects* this contract: it
//! decides, field by field, what it can carry, renders only those fields, and
//! reports the rest as an explicit omission. A field an adapter does not
//! support never reaches a provider request.

use crate::constants::llm::apxm::graph_hints as hint_keys;
use crate::types::values::Value;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value as Json};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::str::FromStr;

pub const GRAPH_HINTS_SCHEMA: &str = hint_keys::SCHEMA;

/// Longest accepted opaque reference, in bytes.
pub const MAX_OPAQUE_REF_LEN: usize = 512;
/// Most direct successors one node hint may enumerate.
pub const MAX_SUCCESSOR_REFS: usize = 128;
/// Largest accepted planner token estimate.
pub const MAX_ESTIMATED_TOKENS: u32 = 16_777_216;
/// Largest accepted remaining-path and stage coordinate.
pub const MAX_PATH_COORDINATE: u32 = 65_535;
/// Largest accepted reuse-benefit horizon (24 hours, in milliseconds).
pub const MAX_BENEFIT_HORIZON_MS: u32 = 86_400_000;
/// Largest accepted expected-uses estimate.
pub const MAX_EXPECTED_USES: u32 = 1_048_576;

/// Domain separator so a hint digest can never collide with another
/// canonical-JSON digest computed elsewhere in the machine.
const HINTS_DIGEST_DOMAIN: &[u8] = b"apxm.inference-graph-hints\0";
const CAPABILITY_DIGEST_DOMAIN: &[u8] = b"apxm.graph-hint-capabilities\0";
const PLAN_DIGEST_DOMAIN: &[u8] = b"apxm.graph-hint-plan\0";
const PROJECTED_REQUEST_DIGEST_DOMAIN: &[u8] = b"apxm.graph-hint-projected-request\0";
const LIFECYCLE_DIGEST_DOMAIN: &[u8] = b"apxm.graph-hint-lifecycle\0";

/// Canonical JSON: object keys sorted, arrays order-preserving, absent values
/// absent. Two semantically identical documents canonicalize byte-for-byte
/// identically regardless of how they were built.
fn canonicalize(value: Json) -> Json {
    match value {
        Json::Array(values) => Json::Array(values.into_iter().map(canonicalize).collect()),
        Json::Object(values) => {
            let mut entries: Vec<(String, Json)> = values.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut canonical = Map::new();
            for (key, value) in entries {
                canonical.insert(key, canonicalize(value));
            }
            Json::Object(canonical)
        }
        scalar => scalar,
    }
}

/// The exact bytes a digest is taken over.
fn canonical_bytes(value: &impl Serialize) -> Vec<u8> {
    let json = serde_json::to_value(value).unwrap_or(Json::Null);
    serde_json::to_vec(&canonicalize(json)).unwrap_or_default()
}

fn domain_digest(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn require_opaque_ref(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{name} must be a non-empty opaque reference"));
    }
    if value.len() > MAX_OPAQUE_REF_LEN {
        return Err(format!(
            "{name} exceeds the {MAX_OPAQUE_REF_LEN}-byte opaque reference limit"
        ));
    }
    Ok(())
}

fn require_digest(name: &str, value: &str) -> Result<(), String> {
    let valid = value.len() == 71
        && value.starts_with("sha256:")
        && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit());
    if valid {
        Ok(())
    } else {
        Err(format!("{name} must be a sha256 digest"))
    }
}

fn require_at_most(name: &str, value: Option<u32>, maximum: u32) -> Result<(), String> {
    match value {
        Some(value) if value > maximum => {
            Err(format!("{name} exceeds its published maximum of {maximum}"))
        }
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphHintScope {
    pub graph_ref: String,
    pub graph_execution_ref: String,
    pub node_ref: String,
    pub node_execution_ref: String,
}

impl GraphHintScope {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            (hint_keys::GRAPH_REF, &self.graph_ref),
            (hint_keys::GRAPH_EXECUTION_REF, &self.graph_execution_ref),
            (hint_keys::NODE_REF, &self.node_ref),
            (hint_keys::NODE_EXECUTION_REF, &self.node_execution_ref),
        ] {
            require_opaque_ref(name, value)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkClass {
    Short,
    Medium,
    Long,
}

impl WorkClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Short => hint_keys::WORK_SHORT,
            Self::Medium => hint_keys::WORK_MEDIUM,
            Self::Long => hint_keys::WORK_LONG,
        }
    }
}

impl fmt::Display for WorkClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WorkClass {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            hint_keys::WORK_SHORT => Ok(Self::Short),
            hint_keys::WORK_MEDIUM => Ok(Self::Medium),
            hint_keys::WORK_LONG => Ok(Self::Long),
            _ => Err(format!("unknown work class: {value}")),
        }
    }
}

impl Serialize for WorkClass {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WorkClass {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_str(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
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

impl NodeGraphFacts {
    /// Every numeric estimate is bounded and every reference list is capped.
    /// An out-of-range value is a rejection, never a clamped value.
    pub fn validate(&self) -> Result<(), String> {
        if self.successor_refs.len() > MAX_SUCCESSOR_REFS {
            return Err(format!(
                "{} exceeds the {MAX_SUCCESSOR_REFS}-entry limit",
                hint_keys::SUCCESSOR_REFS
            ));
        }
        for successor in &self.successor_refs {
            require_opaque_ref(hint_keys::SUCCESSOR_REFS, successor)?;
        }
        if let Some(group) = &self.coexecution_group_ref {
            require_opaque_ref(hint_keys::COEXECUTION_GROUP_REF, group)?;
        }
        require_at_most(
            hint_keys::REMAINING_PATH_LEN,
            self.remaining_path_len,
            MAX_PATH_COORDINATE,
        )?;
        require_at_most(
            hint_keys::STAGE_INDEX,
            self.stage_index,
            MAX_PATH_COORDINATE,
        )?;
        for (name, value) in [
            (
                hint_keys::ESTIMATED_INPUT_TOKENS,
                self.estimated_input_tokens,
            ),
            (
                hint_keys::ESTIMATED_OUTPUT_TOKENS,
                self.estimated_output_tokens,
            ),
            (
                hint_keys::EXPECTED_SHARED_PREFIX_TOKENS,
                self.expected_shared_prefix_tokens,
            ),
        ] {
            require_at_most(name, value, MAX_ESTIMATED_TOKENS)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptimizationObjective {
    MinimizeGraphCompletionTime,
    Balanced,
    MaximizeThroughput,
}

impl OptimizationObjective {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MinimizeGraphCompletionTime => hint_keys::MINIMIZE_GRAPH_COMPLETION_TIME,
            Self::Balanced => hint_keys::BALANCED,
            Self::MaximizeThroughput => hint_keys::MAXIMIZE_THROUGHPUT,
        }
    }
}

impl fmt::Display for OptimizationObjective {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OptimizationObjective {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            hint_keys::MINIMIZE_GRAPH_COMPLETION_TIME => Ok(Self::MinimizeGraphCompletionTime),
            hint_keys::BALANCED => Ok(Self::Balanced),
            hint_keys::MAXIMIZE_THROUGHPUT => Ok(Self::MaximizeThroughput),
            _ => Err(format!("unknown optimization objective: {value}")),
        }
    }
}

impl Serialize for OptimizationObjective {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for OptimizationObjective {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_str(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReusePreference {
    PreferWhenBeneficial,
}

impl ReusePreference {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PreferWhenBeneficial => hint_keys::PREFER_WHEN_BENEFICIAL,
        }
    }
}

impl fmt::Display for ReusePreference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ReusePreference {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            hint_keys::PREFER_WHEN_BENEFICIAL => Ok(Self::PreferWhenBeneficial),
            _ => Err(format!("unknown reuse preference: {value}")),
        }
    }
}

impl Serialize for ReusePreference {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ReusePreference {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_str(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// Why a projector approximated or withheld a field it otherwise understands.
///
/// The set is closed: a free-form sentence cannot be joined, compared across
/// attempts, or held against a capability table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReasonCode {
    /// The exact binding declares no mechanism with this semantic effect.
    NoEquivalentMechanism,
    /// A mechanism exists but only approximates the common semantics.
    ApproximatedByRelatedMechanism,
    /// The value is outside the range the provider mechanism accepts.
    ValueOutsideMechanismRange,
    /// The admitted runtime profile withholds an otherwise supported lowering.
    ProfileWithholdsMechanism,
    /// The admitted scheduler or server contract does not enable the mechanism.
    MechanismNotAdmitted,
}

impl ReasonCode {
    pub const ALL: &'static [Self] = &[
        Self::NoEquivalentMechanism,
        Self::ApproximatedByRelatedMechanism,
        Self::ValueOutsideMechanismRange,
        Self::ProfileWithholdsMechanism,
        Self::MechanismNotAdmitted,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoEquivalentMechanism => hint_keys::REASON_NO_EQUIVALENT_MECHANISM,
            Self::ApproximatedByRelatedMechanism => {
                hint_keys::REASON_APPROXIMATED_BY_RELATED_MECHANISM
            }
            Self::ValueOutsideMechanismRange => hint_keys::REASON_VALUE_OUTSIDE_MECHANISM_RANGE,
            Self::ProfileWithholdsMechanism => hint_keys::REASON_PROFILE_WITHHOLDS_MECHANISM,
            Self::MechanismNotAdmitted => hint_keys::REASON_MECHANISM_NOT_ADMITTED,
        }
    }
}

impl fmt::Display for ReasonCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ReasonCode {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|code| code.as_str() == value)
            .ok_or_else(|| format!("unknown projection reason code: {value}"))
    }
}

impl Serialize for ReasonCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ReasonCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_str(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// A closed, adapter-owned mechanism identifier such as `llama.cache_prompt`.
///
/// The *shape* is common; the *names* are owned by the adapter that implements
/// the mechanism, which is why no provider mechanism name is spelled in this
/// module. A mechanism reference is a label for evidence: it never carries a
/// secret, a slot coordinate, a filename, or a value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackendMechanismRef(String);

impl BackendMechanismRef {
    /// Accept `<adapter>.<mechanism>` in lowercase snake segments.
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        let mut segments = value.split('.');
        let (Some(adapter), Some(mechanism), None) =
            (segments.next(), segments.next(), segments.next())
        else {
            return Err(format!(
                "mechanism reference {value:?} must be exactly <adapter>.<mechanism>"
            ));
        };
        for segment in [adapter, mechanism] {
            let valid = !segment.is_empty()
                && segment.starts_with(|c: char| c.is_ascii_lowercase())
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            if !valid {
                return Err(format!(
                    "mechanism reference {value:?} segment {segment:?} is not lowercase snake case"
                ));
            }
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BackendMechanismRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for BackendMechanismRef {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for BackendMechanismRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for BackendMechanismRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReusableContextIntent {
    pub preference: ReusePreference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affinity_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benefit_horizon_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_uses: Option<u32>,
}

impl ReusableContextIntent {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(affinity_ref) = &self.affinity_ref {
            require_opaque_ref(hint_keys::AFFINITY_REF, affinity_ref)?;
        }
        require_at_most(
            hint_keys::BENEFIT_HORIZON_MS,
            self.benefit_horizon_ms,
            MAX_BENEFIT_HORIZON_MS,
        )?;
        require_at_most(
            hint_keys::EXPECTED_USES,
            self.expected_uses,
            MAX_EXPECTED_USES,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct GraphExecutionIntents {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<OptimizationObjective>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reusable_context: Option<ReusableContextIntent>,
}

impl GraphExecutionIntents {
    pub fn validate(&self) -> Result<(), String> {
        match &self.reusable_context {
            Some(intent) => intent.validate(),
            None => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApxmGraphHints {
    pub schema: String,
    pub scope: GraphHintScope,
    #[serde(default)]
    pub facts: NodeGraphFacts,
    #[serde(default)]
    pub intents: GraphExecutionIntents,
}

impl ApxmGraphHints {
    /// Admit the envelope or fail closed. Unknown fields are rejected by the
    /// decode path; this rejects unknown schemas, blank references, and every
    /// out-of-range estimate.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != GRAPH_HINTS_SCHEMA {
            return Err(format!("unknown graph-hints schema: {}", self.schema));
        }
        self.scope.validate()?;
        self.facts.validate()?;
        self.intents.validate()
    }

    /// The exact canonical bytes the hint digest commits to.
    pub fn canonical_json(&self) -> String {
        String::from_utf8(canonical_bytes(self)).unwrap_or_default()
    }

    /// A stable digest over the canonical form.
    ///
    /// Two semantically identical hint sets digest identically regardless of
    /// how they were constructed, and any change to a present field, an absent
    /// field, or a reference list changes the digest.
    pub fn digest(&self) -> String {
        domain_digest(HINTS_DIGEST_DOMAIN, &canonical_bytes(self))
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

    /// Validate the graph-execution-scoped part of a node-hint collection.
    ///
    /// A single node hint can be admitted independently, but an execution
    /// materializer must reject a collection that silently changes the
    /// objective or reusable-context policy from one node to the next.
    pub fn validate_execution_consistency(hints: &[Self]) -> Result<(), String> {
        let Some(first) = hints.first() else {
            return Ok(());
        };
        first.validate()?;
        let graph_ref = &first.scope.graph_ref;
        let execution_ref = &first.scope.graph_execution_ref;
        for hint in &hints[1..] {
            hint.validate()?;
            if &hint.scope.graph_ref != graph_ref
                || &hint.scope.graph_execution_ref != execution_ref
            {
                return Err(
                    "graph-hint collection mixes graph or graph-execution references".to_owned(),
                );
            }
            if hint.intents != first.intents {
                return Err(
                    "graph-execution-scoped graph-hint intents conflict across node hints"
                        .to_owned(),
                );
            }
        }
        Ok(())
    }

    /// Render exactly the fields a plan projects.
    ///
    /// This is the only place the common envelope becomes a document bound for
    /// a provider. A field whose outcome is an omission is absent from the
    /// result — not null, not defaulted, absent. `None` means the plan projects
    /// nothing and the provider request must stay unchanged.
    pub fn project_envelope(&self, plan: &GraphHintPlan) -> Option<Json> {
        let mut facts = Map::new();
        fn set(
            facts: &mut Map<String, Json>,
            plan: &GraphHintPlan,
            field: GraphHintField,
            key: &str,
            value: Option<Json>,
        ) {
            if let (true, Some(value)) = (plan.projects(field), value) {
                facts.insert(key.to_owned(), value);
            }
        }
        let mut set = |field, key: &str, value| set(&mut facts, plan, field, key, value);
        set(
            GraphHintField::CriticalPath,
            hint_keys::CRITICAL_PATH,
            self.facts.critical_path.map(Json::from),
        );
        set(
            GraphHintField::SuccessorRefs,
            hint_keys::SUCCESSOR_REFS,
            (!self.facts.successor_refs.is_empty())
                .then(|| Json::from(self.facts.successor_refs.clone())),
        );
        set(
            GraphHintField::RemainingPathLen,
            hint_keys::REMAINING_PATH_LEN,
            self.facts.remaining_path_len.map(Json::from),
        );
        set(
            GraphHintField::StageIndex,
            hint_keys::STAGE_INDEX,
            self.facts.stage_index.map(Json::from),
        );
        set(
            GraphHintField::WorkClass,
            hint_keys::WORK_CLASS,
            self.facts
                .work_class
                .map(|class| Json::from(class.as_str())),
        );
        set(
            GraphHintField::EstimatedInputTokens,
            hint_keys::ESTIMATED_INPUT_TOKENS,
            self.facts.estimated_input_tokens.map(Json::from),
        );
        set(
            GraphHintField::EstimatedOutputTokens,
            hint_keys::ESTIMATED_OUTPUT_TOKENS,
            self.facts.estimated_output_tokens.map(Json::from),
        );
        set(
            GraphHintField::ExpectedSharedPrefixTokens,
            hint_keys::EXPECTED_SHARED_PREFIX_TOKENS,
            self.facts.expected_shared_prefix_tokens.map(Json::from),
        );
        set(
            GraphHintField::PrefixWarmupEligible,
            hint_keys::PREFIX_WARMUP_ELIGIBLE,
            self.facts.prefix_warmup_eligible.map(Json::from),
        );
        set(
            GraphHintField::PipelineEligible,
            hint_keys::PIPELINE_ELIGIBLE,
            self.facts.pipeline_eligible.map(Json::from),
        );
        set(
            GraphHintField::CoexecutionGroupRef,
            hint_keys::COEXECUTION_GROUP_REF,
            self.facts.coexecution_group_ref.clone().map(Json::from),
        );

        let mut intents = Map::new();
        if plan.projects(GraphHintField::Objective)
            && let Some(objective) = self.intents.objective
        {
            intents.insert(
                hint_keys::OBJECTIVE.to_owned(),
                Json::from(objective.as_str()),
            );
        }
        // `preference` is the required member of a reusable-context intent, so
        // an adapter that cannot carry the preference carries no affinity,
        // horizon, or use estimate either.
        if plan.projects(GraphHintField::ReusePreference)
            && let Some(reuse) = &self.intents.reusable_context
        {
            let mut context = Map::new();
            context.insert(
                hint_keys::PREFERENCE.to_owned(),
                Json::from(reuse.preference.as_str()),
            );
            if plan.projects(GraphHintField::AffinityRef)
                && let Some(affinity_ref) = &reuse.affinity_ref
            {
                context.insert(
                    hint_keys::AFFINITY_REF.to_owned(),
                    Json::from(affinity_ref.clone()),
                );
            }
            if plan.projects(GraphHintField::BenefitHorizonMs)
                && let Some(horizon) = reuse.benefit_horizon_ms
            {
                context.insert(
                    hint_keys::BENEFIT_HORIZON_MS.to_owned(),
                    Json::from(horizon),
                );
            }
            if plan.projects(GraphHintField::ExpectedUses)
                && let Some(uses) = reuse.expected_uses
            {
                context.insert(hint_keys::EXPECTED_USES.to_owned(), Json::from(uses));
            }
            intents.insert(
                hint_keys::REUSABLE_CONTEXT.to_owned(),
                Json::Object(context),
            );
        }

        let scope = plan
            .projects(GraphHintField::Scope)
            .then(|| serde_json::to_value(&self.scope).unwrap_or(Json::Object(Map::new())));
        if scope.is_none() && facts.is_empty() && intents.is_empty() {
            return None;
        }

        let mut envelope = Map::new();
        envelope.insert(
            hint_keys::SCHEMA_FIELD.to_owned(),
            Json::from(GRAPH_HINTS_SCHEMA),
        );
        if let Some(scope) = scope {
            envelope.insert(hint_keys::SCOPE.to_owned(), scope);
        }
        if !facts.is_empty() {
            envelope.insert(hint_keys::FACTS.to_owned(), Json::Object(facts));
        }
        if !intents.is_empty() {
            envelope.insert(hint_keys::INTENTS.to_owned(), Json::Object(intents));
        }
        Some(canonicalize(Json::Object(envelope)))
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
            let ordinal = value.as_i64()?;
            Some(ordinal >= graph_meta::CRITICAL_PATH_ATTR_THRESHOLD)
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
                            .map(|raw| raw.to_string())
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

        let reusable_context = affinity_ref
            .as_ref()
            .map(|affinity_ref| ReusableContextIntent {
                preference: ReusePreference::PreferWhenBeneficial,
                affinity_ref: Some(affinity_ref.clone()),
                benefit_horizon_ms: None,
                expected_uses: None,
            });

        let hints = Self {
            schema: GRAPH_HINTS_SCHEMA.to_owned(),
            scope: GraphHintScope {
                graph_ref: graph_id.clone(),
                graph_execution_ref: graph_id,
                node_ref: node_label.clone(),
                node_execution_ref: node_label,
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

/// How a binding carries one field, when it carries it at all.
///
/// `Direct` means the adapter has a provider control with the same useful
/// semantic effect; `Derived` means it uses a documented heuristic or
/// combination of controls. Neither is a performance guarantee.
///
/// The only evidence any binding here produces is its own projection record —
/// the plan and the projection. A provider-reported acknowledgement or a
/// comparable outcome measurement would be separate evidence layers, and no
/// provider response this repository parses carries either, so a binding
/// cannot declare that it produces them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphHintFieldCapability {
    Direct,
    Derived,
    Unsupported,
}

impl GraphHintFieldCapability {
    pub const fn is_supported(&self) -> bool {
        !matches!(self, Self::Unsupported)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphLifecycleCapability {
    PrepareRelease,
    NotNeeded,
    Unsupported,
}

/// A fact-only graph description used by the exact adapter lifecycle seam.
/// It contains no provider cache, scheduler, slot, worker, or route data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApxmGraphDescriptor {
    pub graph_ref: String,
    pub graph_execution_ref: String,
    pub nodes: Vec<ApxmGraphDescriptorNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApxmGraphDescriptorNode {
    pub node_ref: String,
    pub facts: NodeGraphFacts,
}

impl ApxmGraphDescriptor {
    pub fn validate(&self) -> Result<(), String> {
        require_opaque_ref(hint_keys::GRAPH_REF, &self.graph_ref)?;
        require_opaque_ref(hint_keys::GRAPH_EXECUTION_REF, &self.graph_execution_ref)?;
        if self.nodes.is_empty() {
            return Err("graph descriptor must contain at least one node".to_owned());
        }
        let mut node_refs = std::collections::BTreeSet::new();
        for node in &self.nodes {
            require_opaque_ref(hint_keys::NODE_REF, &node.node_ref)?;
            if !node_refs.insert(&node.node_ref) {
                return Err(format!("graph descriptor repeats node {}", node.node_ref));
            }
            node.facts.validate()?;
        }
        Ok(())
    }

    pub fn digest(&self) -> String {
        domain_digest(LIFECYCLE_DIGEST_DOMAIN, &canonical_bytes(self))
    }

    /// Build the static descriptor only after all node hints have been
    /// admitted and their graph-execution-scoped intents agree.
    pub fn from_hints(hints: &[ApxmGraphHints]) -> Result<Self, String> {
        ApxmGraphHints::validate_execution_consistency(hints)?;
        let first = hints
            .first()
            .ok_or_else(|| "cannot describe an empty graph execution".to_owned())?;
        let descriptor = Self {
            graph_ref: first.scope.graph_ref.clone(),
            graph_execution_ref: first.scope.graph_execution_ref.clone(),
            nodes: hints
                .iter()
                .map(|hint| ApxmGraphDescriptorNode {
                    node_ref: hint.scope.node_ref.clone(),
                    facts: hint.facts.clone(),
                })
                .collect(),
        };
        descriptor.validate()?;
        Ok(descriptor)
    }
}

/// An adapter-local preparation identity. Raw provider handles and graph ids
/// never leave the adapter; release accepts this digest-bound reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphPreparationRef {
    pub descriptor_digest: String,
    pub preparation_digest: String,
}

impl GraphPreparationRef {
    pub fn mint(descriptor_digest: &str, adapter_material: &str) -> Result<Self, String> {
        require_digest("descriptor_digest", descriptor_digest)?;
        require_opaque_ref("adapter_material", adapter_material)?;
        let preparation_digest = domain_digest(
            LIFECYCLE_DIGEST_DOMAIN,
            &canonical_bytes(&serde_json::json!({
                "descriptor_digest": descriptor_digest,
                "adapter_material": adapter_material,
            })),
        );
        Ok(Self {
            descriptor_digest: descriptor_digest.to_owned(),
            preparation_digest,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        require_digest("descriptor_digest", &self.descriptor_digest)?;
        require_digest("preparation_digest", &self.preparation_digest)
    }
}

/// Closed reasons for lifecycle calls that fail before a provider request is
/// sent. An accepted response or an ambiguous transport error is represented
/// by `OutcomeUnknown` instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleReasonCode {
    InvalidDescriptor,
    MechanismNotAdmitted,
    ProfileWithholdsMechanism,
    StalePreparation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum GraphPrepareOutcome {
    Prepared {
        preparation: GraphPreparationRef,
        projection_digest: String,
    },
    NotNeeded,
    Unsupported,
    FailedBeforeSend {
        reason: LifecycleReasonCode,
    },
    OutcomeUnknown {
        reconciliation_ref: String,
    },
}

impl GraphPrepareOutcome {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Prepared {
                preparation,
                projection_digest,
            } => {
                preparation.validate()?;
                require_digest("projection_digest", projection_digest)
            }
            Self::OutcomeUnknown { reconciliation_ref } => {
                require_opaque_ref("reconciliation_ref", reconciliation_ref)
            }
            Self::NotNeeded | Self::Unsupported | Self::FailedBeforeSend { .. } => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum GraphReleaseOutcome {
    Released { preparation_digest: String },
    NotNeeded,
    Unsupported,
    FailedBeforeSend { reason: LifecycleReasonCode },
    OutcomeUnknown { reconciliation_ref: String },
}

impl GraphReleaseOutcome {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Released { preparation_digest } => {
                require_digest("preparation_digest", preparation_digest)
            }
            Self::OutcomeUnknown { reconciliation_ref } => {
                require_opaque_ref("reconciliation_ref", reconciliation_ref)
            }
            Self::NotNeeded | Self::Unsupported | Self::FailedBeforeSend { .. } => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphHintCapabilities {
    pub fields: BTreeMap<GraphHintField, GraphHintFieldCapability>,
    pub lifecycle: GraphLifecycleCapability,
}

impl GraphHintCapabilities {
    /// A conforming implementation that supports nothing. Supporting zero
    /// fields is honest; claiming support silently is not.
    pub fn none() -> Self {
        Self {
            fields: GraphHintField::ALL
                .iter()
                .map(|field| (*field, GraphHintFieldCapability::Unsupported))
                .collect(),
            lifecycle: GraphLifecycleCapability::NotNeeded,
        }
    }

    /// Content address of the declared capability surface. Binding evidence
    /// carries this digest so a health probe cannot widen support mid-effect.
    pub fn digest(&self) -> String {
        domain_digest(CAPABILITY_DIGEST_DOMAIN, &canonical_bytes(self))
    }

    pub fn supports(&self, field: GraphHintField) -> bool {
        self.fields
            .get(&field)
            .is_some_and(GraphHintFieldCapability::is_supported)
    }

    /// A capability declaration is complete and closed. An adapter may
    /// support no fields, but it may not leave a field unclassified.
    pub fn validate(&self) -> Result<(), String> {
        for field in GraphHintField::ALL {
            if !self.fields.contains_key(field) {
                return Err(format!("graph-hint capability declaration omits {field:?}"));
            }
        }
        if self.fields.len() != GraphHintField::ALL.len() {
            return Err("graph-hint capability declaration contains an unknown field".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionOutcome {
    Applied {
        mechanism_ref: BackendMechanismRef,
    },
    Approximated {
        mechanism_ref: BackendMechanismRef,
        reason: ReasonCode,
    },
    OmittedUnsupported,
    OmittedByProfile {
        reason: ReasonCode,
    },
}

impl ProjectionOutcome {
    /// Whether this outcome authorizes the field to reach the provider request.
    pub const fn is_projected(&self) -> bool {
        matches!(self, Self::Applied { .. } | Self::Approximated { .. })
    }

    pub const fn mechanism_ref(&self) -> Option<&BackendMechanismRef> {
        match self {
            Self::Applied { mechanism_ref } | Self::Approximated { mechanism_ref, .. } => {
                Some(mechanism_ref)
            }
            Self::OmittedUnsupported | Self::OmittedByProfile { .. } => None,
        }
    }
}

/// The per-dispatch field-by-field projection report.
///
/// A plan over present hints always covers the whole closed field set, so
/// "what happened to every field" is answerable without inspecting the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphHintPlan {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_hints_digest: Option<String>,
    pub capability_digest: String,
    pub outcomes: BTreeMap<GraphHintField, ProjectionOutcome>,
}

impl GraphHintPlan {
    /// The plan for a dispatch that carries no hints: no digest, no outcomes,
    /// and therefore nothing added to the provider request.
    pub fn absent(capabilities: &GraphHintCapabilities) -> Self {
        Self {
            graph_hints_digest: None,
            capability_digest: capabilities.digest(),
            outcomes: BTreeMap::new(),
        }
    }

    /// The complete explicit report of a zero-capability projector.
    pub fn omitted_unsupported(
        hints: &ApxmGraphHints,
        capabilities: &GraphHintCapabilities,
    ) -> Self {
        Self {
            graph_hints_digest: Some(hints.digest()),
            capability_digest: capabilities.digest(),
            outcomes: GraphHintField::ALL
                .iter()
                .map(|field| (*field, ProjectionOutcome::OmittedUnsupported))
                .collect(),
        }
    }

    /// Start from the complete omitted report and record the exceptions.
    pub fn with_outcome(mut self, field: GraphHintField, outcome: ProjectionOutcome) -> Self {
        self.outcomes.insert(field, outcome);
        self
    }

    pub fn projects(&self, field: GraphHintField) -> bool {
        self.outcomes
            .get(&field)
            .is_some_and(ProjectionOutcome::is_projected)
    }

    pub fn projects_any(&self) -> bool {
        self.outcomes.values().any(ProjectionOutcome::is_projected)
    }

    pub fn digest(&self) -> String {
        domain_digest(PLAN_DIGEST_DOMAIN, &canonical_bytes(self))
    }

    /// Hold the plan against the capability table it claims to come from.
    ///
    /// A projector cannot claim a mechanism for a field it declares
    /// unsupported, and a plan over present hints must cover every field.
    pub fn validate_against(&self, capabilities: &GraphHintCapabilities) -> Result<(), String> {
        capabilities.validate()?;
        if self.capability_digest != capabilities.digest() {
            return Err(
                "graph-hint capability digest changed between declaration and planning".to_owned(),
            );
        }
        if self.graph_hints_digest.is_none() {
            return if self.outcomes.is_empty() {
                Ok(())
            } else {
                Err("a plan without hints cannot carry projection outcomes".to_owned())
            };
        }
        for field in GraphHintField::ALL {
            let Some(outcome) = self.outcomes.get(field) else {
                return Err(format!(
                    "graph-hint plan omits an outcome for {}",
                    serde_json::to_string(field).unwrap_or_default()
                ));
            };
            match (capabilities.fields.get(field), outcome) {
                (
                    Some(GraphHintFieldCapability::Unsupported),
                    ProjectionOutcome::OmittedUnsupported,
                ) => {}
                (Some(GraphHintFieldCapability::Unsupported), _) => {
                    return Err(format!(
                        "graph-hint plan gives unsupported field {} a non-unsupported outcome",
                        serde_json::to_string(field).unwrap_or_default()
                    ));
                }
                (
                    Some(_),
                    ProjectionOutcome::Applied { mechanism_ref }
                    | ProjectionOutcome::Approximated { mechanism_ref, .. },
                ) => {
                    if mechanism_ref.as_str().is_empty() {
                        return Err(format!(
                            "graph-hint plan gives {} an empty mechanism reference",
                            serde_json::to_string(field).unwrap_or_default()
                        ));
                    }
                }
                (Some(_), _) => {}
                (None, _) => {
                    return Err(format!(
                        "graph-hint capability declaration has no entry for {}",
                        serde_json::to_string(field).unwrap_or_default()
                    ));
                }
            }
        }
        Ok(())
    }
}

/// One attempt's realized projection. It commits to the exact provider fields
/// emitted for that attempt by digest and never carries the fields themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphHintProjection {
    pub plan_digest: String,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mechanism_bindings: Vec<BackendMechanismRef>,
    pub projected_request_digest: String,
}

/// What a projector produced for one attempt: the report, the attempt-local
/// projection evidence, and the provider fields the plan authorized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphHintDispatchProjection {
    pub plan: GraphHintPlan,
    pub projection: GraphHintProjection,
    pub provider_fields: Map<String, Json>,
}

impl GraphHintDispatchProjection {
    /// Evidence for the runtime record: digests and closed vocabulary only,
    /// never the provider body.
    pub fn to_evidence_json(&self) -> Json {
        serde_json::json!({
            hint_keys::PLAN: self.plan,
            hint_keys::PROJECTION: self.projection,
        })
    }
}

/// The one seam between APXM graph semantics and a provider request.
///
/// An adapter declares what it can carry, plans field by field, and renders
/// only the fields its own plan authorized. Everything else the machine knows
/// stays in runtime evidence and never reaches the provider.
pub trait GraphHintProjector {
    /// Declared per exact driver/profile/binding, not per provider brand.
    fn graph_hint_capabilities(&self) -> GraphHintCapabilities {
        GraphHintCapabilities::none()
    }

    /// Pure, deterministic field-by-field planning. The default is the
    /// complete explicit zero-capability report.
    fn plan_graph_hints(&self, hints: Option<&ApxmGraphHints>) -> Result<GraphHintPlan, String> {
        let capabilities = self.graph_hint_capabilities();
        match hints {
            None => Ok(GraphHintPlan::absent(&capabilities)),
            Some(hints) => {
                hints.validate()?;
                Ok(GraphHintPlan::omitted_unsupported(hints, &capabilities))
            }
        }
    }

    /// Render exactly the provider request fields the plan authorized.
    ///
    /// The default renders nothing, so a backend with no graph capabilities
    /// receives an otherwise unchanged model request.
    fn render_graph_hint_fields(
        &self,
        _hints: &ApxmGraphHints,
        _plan: &GraphHintPlan,
    ) -> Result<Map<String, Json>, String> {
        Ok(Map::new())
    }

    /// Plan, check the plan against the declared capabilities, render, and
    /// seal the rendered fields by digest. This is the only path from graph
    /// hints to a provider request.
    fn project_graph_hints(
        &self,
        hints: Option<&ApxmGraphHints>,
        attempt: u32,
    ) -> Result<GraphHintDispatchProjection, String> {
        let capabilities = self.graph_hint_capabilities();
        capabilities.validate()?;
        if let Some(hints) = hints {
            // The seam owns admission as a defense in depth. An adapter's
            // custom planner must not be able to bypass common validation.
            hints.validate()?;
        }
        let plan = self.plan_graph_hints(hints)?;
        plan.validate_against(&capabilities)?;
        match hints {
            Some(hints) if plan.graph_hints_digest.as_deref() == Some(hints.digest().as_str()) => {}
            Some(_) => return Err("graph-hint plan commits to a different envelope".to_owned()),
            None if plan.graph_hints_digest.is_none() => {}
            None => return Err("a hint-free plan cannot carry a hint digest".to_owned()),
        }
        let provider_fields = match hints {
            Some(hints) if plan.projects_any() => self.render_graph_hint_fields(hints, &plan)?,
            _ => Map::new(),
        };
        let mut mechanism_bindings: Vec<BackendMechanismRef> = plan
            .outcomes
            .values()
            .filter_map(ProjectionOutcome::mechanism_ref)
            .cloned()
            .collect();
        mechanism_bindings.sort();
        mechanism_bindings.dedup();
        let projection = GraphHintProjection {
            plan_digest: plan.digest(),
            attempt,
            mechanism_bindings,
            projected_request_digest: domain_digest(
                PROJECTED_REQUEST_DIGEST_DOMAIN,
                &canonical_bytes(&Json::Object(provider_fields.clone())),
            ),
        };
        Ok(GraphHintDispatchProjection {
            plan,
            projection,
            provider_fields,
        })
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

/// Common lifecycle state plus whatever the adapter observed.
///
/// Resource units are adapter vocabulary: the common record keeps them in an
/// opaque observation map keyed by adapter-owned names.
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

    pub fn registered(graph_ref: impl Into<String>) -> Self {
        Self::new(graph_ref).with_registered(true)
    }

    pub fn with_registered(mut self, registered: bool) -> Self {
        self.registered = registered;
        self
    }

    /// Record one adapter-owned observation under its adapter-owned name.
    pub fn with_adapter_observation(mut self, name: impl Into<String>, value: u64) -> Self {
        self.adapter_observations.insert(name.into(), value);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scoped() -> ApxmGraphHints {
        ApxmGraphHints::critical_path("g", "gx", "n", "nx", vec!["s".into()])
    }

    #[test]
    fn empty_scope_refs_fail_closed() {
        let mut hints = scoped();
        hints.scope.graph_ref = " ".into();
        assert!(hints.validate().is_err());
    }

    #[test]
    fn out_of_range_estimates_fail_closed() {
        for (label, mutate) in [
            (
                "estimated_input_tokens",
                Box::new(|h: &mut ApxmGraphHints| {
                    h.facts.estimated_input_tokens = Some(MAX_ESTIMATED_TOKENS + 1)
                }) as Box<dyn Fn(&mut ApxmGraphHints)>,
            ),
            (
                "estimated_output_tokens",
                Box::new(|h: &mut ApxmGraphHints| {
                    h.facts.estimated_output_tokens = Some(MAX_ESTIMATED_TOKENS + 1)
                }),
            ),
            (
                "expected_shared_prefix_tokens",
                Box::new(|h: &mut ApxmGraphHints| {
                    h.facts.expected_shared_prefix_tokens = Some(MAX_ESTIMATED_TOKENS + 1)
                }),
            ),
            (
                "remaining_path_len",
                Box::new(|h: &mut ApxmGraphHints| {
                    h.facts.remaining_path_len = Some(MAX_PATH_COORDINATE + 1)
                }),
            ),
            (
                "stage_index",
                Box::new(|h: &mut ApxmGraphHints| {
                    h.facts.stage_index = Some(MAX_PATH_COORDINATE + 1)
                }),
            ),
            (
                "successor_refs",
                Box::new(|h: &mut ApxmGraphHints| {
                    h.facts.successor_refs =
                        (0..=MAX_SUCCESSOR_REFS).map(|i| format!("n{i}")).collect()
                }),
            ),
            (
                "benefit_horizon_ms",
                Box::new(|h: &mut ApxmGraphHints| {
                    if let Some(reuse) = h.intents.reusable_context.as_mut() {
                        reuse.benefit_horizon_ms = Some(MAX_BENEFIT_HORIZON_MS + 1);
                    }
                }),
            ),
            (
                "expected_uses",
                Box::new(|h: &mut ApxmGraphHints| {
                    if let Some(reuse) = h.intents.reusable_context.as_mut() {
                        reuse.expected_uses = Some(MAX_EXPECTED_USES + 1);
                    }
                }),
            ),
        ] {
            let mut hints = scoped();
            mutate(&mut hints);
            assert!(
                hints.validate().is_err(),
                "{label} above its maximum must fail closed"
            );
        }
    }

    #[test]
    fn boundary_values_are_admitted() {
        let mut hints = scoped();
        hints.facts.estimated_input_tokens = Some(MAX_ESTIMATED_TOKENS);
        hints.facts.remaining_path_len = Some(MAX_PATH_COORDINATE);
        hints.facts.successor_refs = (0..MAX_SUCCESSOR_REFS).map(|i| format!("n{i}")).collect();
        hints.validate().expect("published maxima are admitted");
    }

    #[test]
    fn unknown_wire_fields_are_rejected_before_send() {
        let mut value = serde_json::to_value(scoped()).expect("serialize");
        value["facts"]["pin_policy"] = serde_json::json!("prefix");
        assert!(serde_json::from_value::<ApxmGraphHints>(value).is_err());
    }

    /// Phase A exit gate: the construction path does not matter.
    #[test]
    fn independently_constructed_identical_hints_share_one_digest() {
        let built = scoped();
        let decoded: ApxmGraphHints = serde_json::from_str(
            r#"{
                "intents": {
                    "reusable_context": {"preference": "prefer_when_beneficial"},
                    "objective": "minimize_graph_completion_time"
                },
                "facts": {"successor_refs": ["s"], "critical_path": true},
                "scope": {
                    "node_execution_ref": "nx",
                    "graph_ref": "g",
                    "node_ref": "n",
                    "graph_execution_ref": "gx"
                },
                "schema": "apxm.inference-graph-hints"
            }"#,
        )
        .expect("decode a differently ordered document");
        assert_eq!(built, decoded);
        assert_eq!(built.digest(), decoded.digest());
        assert_eq!(built.canonical_json(), decoded.canonical_json());
        assert!(built.digest().starts_with("sha256:"));

        let mut changed = built.clone();
        changed.facts.stage_index = Some(1);
        assert_ne!(built.digest(), changed.digest());
    }

    #[test]
    fn zero_capability_plan_omits_every_field_and_renders_nothing() {
        struct Zero;
        impl GraphHintProjector for Zero {}
        let hints = scoped();
        let projected = Zero.project_graph_hints(Some(&hints), 0).expect("project");
        assert_eq!(projected.plan.outcomes.len(), GraphHintField::ALL.len());
        assert!(
            projected
                .plan
                .outcomes
                .values()
                .all(|outcome| matches!(outcome, ProjectionOutcome::OmittedUnsupported))
        );
        assert!(projected.provider_fields.is_empty());
        assert!(hints.project_envelope(&projected.plan).is_none());
        assert_eq!(
            projected.plan.graph_hints_digest.as_deref(),
            Some(hints.digest().as_str())
        );
    }

    #[test]
    fn a_plan_cannot_project_a_field_its_binding_declares_unsupported() {
        let capabilities = GraphHintCapabilities::none();
        let hints = scoped();
        let plan = GraphHintPlan::omitted_unsupported(&hints, &capabilities).with_outcome(
            GraphHintField::ReusePreference,
            ProjectionOutcome::Applied {
                mechanism_ref: BackendMechanismRef::new("test.reuse").expect("mechanism"),
            },
        );
        assert!(plan.validate_against(&capabilities).is_err());
    }

    #[test]
    fn a_plan_cannot_relabel_an_unsupported_field_as_profile_omitted() {
        let capabilities = GraphHintCapabilities::none();
        let hints = scoped();
        let plan = GraphHintPlan::omitted_unsupported(&hints, &capabilities).with_outcome(
            GraphHintField::Scope,
            ProjectionOutcome::OmittedByProfile {
                reason: ReasonCode::ProfileWithholdsMechanism,
            },
        );
        assert!(plan.validate_against(&capabilities).is_err());
    }

    #[test]
    fn execution_scoped_intents_must_match_across_node_hints() {
        let first = scoped();
        let mut second = first.clone();
        second.scope.node_ref = "n2".into();
        second.scope.node_execution_ref = "nx2".into();
        second.intents.objective = Some(OptimizationObjective::MaximizeThroughput);
        assert!(ApxmGraphHints::validate_execution_consistency(&[first, second]).is_err());
    }

    #[test]
    fn a_descriptor_is_fact_only_and_mints_a_digest_bound_preparation() {
        let descriptor = ApxmGraphDescriptor::from_hints(&[scoped()]).expect("descriptor");
        assert!(descriptor.validate().is_ok());
        let preparation = GraphPreparationRef::mint(&descriptor.digest(), "adapter-result.1")
            .expect("preparation identity");
        assert!(preparation.validate().is_ok());
        let rendered = serde_json::to_string(&preparation).expect("serialize");
        assert!(!rendered.contains("adapter-result.1"));
    }

    #[test]
    fn lifecycle_outcomes_reject_unverifiable_digest_claims() {
        let invalid = GraphPrepareOutcome::Prepared {
            preparation: GraphPreparationRef {
                descriptor_digest:
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                preparation_digest:
                    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            projection_digest: "not-a-digest".into(),
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn the_projector_seam_revalidates_custom_plans() {
        struct InvalidPlan;

        impl GraphHintProjector for InvalidPlan {
            fn plan_graph_hints(
                &self,
                hints: Option<&ApxmGraphHints>,
            ) -> Result<GraphHintPlan, String> {
                let hints = hints.expect("fixture supplies hints");
                Ok(
                    GraphHintPlan::omitted_unsupported(hints, &GraphHintCapabilities::none())
                        .with_outcome(
                            GraphHintField::Scope,
                            ProjectionOutcome::OmittedByProfile {
                                reason: ReasonCode::ProfileWithholdsMechanism,
                            },
                        ),
                )
            }
        }

        assert!(InvalidPlan.project_graph_hints(Some(&scoped()), 0).is_err());
    }

    #[test]
    fn projected_envelope_carries_only_the_projected_fields() {
        let mut capabilities = GraphHintCapabilities::none();
        capabilities.fields.insert(
            GraphHintField::ReusePreference,
            GraphHintFieldCapability::Direct,
        );
        let hints = scoped();
        let plan = GraphHintPlan::omitted_unsupported(&hints, &capabilities).with_outcome(
            GraphHintField::ReusePreference,
            ProjectionOutcome::Applied {
                mechanism_ref: BackendMechanismRef::new("test.reuse").expect("mechanism"),
            },
        );
        plan.validate_against(&capabilities).expect("honest plan");
        let envelope = hints.project_envelope(&plan).expect("a projected envelope");
        let rendered = envelope.to_string();
        assert!(rendered.contains(hint_keys::PREFER_WHEN_BENEFICIAL));
        for absent in [
            hint_keys::SUCCESSOR_REFS,
            hint_keys::CRITICAL_PATH,
            hint_keys::SCOPE,
            hint_keys::OBJECTIVE,
        ] {
            assert!(
                !rendered.contains(absent),
                "unsupported field {absent} reached the provider document"
            );
        }
    }

    #[test]
    fn mechanism_refs_are_adapter_namespaced() {
        assert!(BackendMechanismRef::new("llama.cache_prompt").is_ok());
        for invalid in ["cache_prompt", "Llama.CachePrompt", "vllm.", "a.b.c", ""] {
            assert!(
                BackendMechanismRef::new(invalid).is_err(),
                "{invalid:?} must not be a mechanism reference"
            );
        }
    }

    #[test]
    fn envelope_uses_owned_hint_keys() {
        let hints = scoped();
        let value = serde_json::to_value(&hints).expect("serialize");
        assert_eq!(value[hint_keys::SCHEMA_FIELD], hint_keys::SCHEMA);
        assert!(value.get(hint_keys::SCOPE).is_some());
        assert!(value.get(hint_keys::FACTS).is_some());
        assert!(value.get(hint_keys::INTENTS).is_some());
        assert_eq!(
            value[hint_keys::INTENTS][hint_keys::REUSABLE_CONTEXT][hint_keys::PREFERENCE],
            hint_keys::PREFER_WHEN_BENEFICIAL
        );
    }

    #[test]
    fn reason_codes_round_trip_through_their_closed_wire_names() {
        for code in ReasonCode::ALL {
            assert_eq!(ReasonCode::from_str(code.as_str()), Ok(*code));
        }
        assert!(ReasonCode::from_str("because").is_err());
    }
}
