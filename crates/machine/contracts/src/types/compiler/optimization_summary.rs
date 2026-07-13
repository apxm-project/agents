//! Additive optimization evidence carried alongside an executable artifact.

use serde::{Deserialize, Serialize};

use crate::constants::graph::attrs::PromptInputRole;
use crate::types::{NodeId, PermissionOperation};

/// Versioned artifact section for compiler-produced optimization evidence.
pub const OPTIMIZATION_SUMMARY_ARTIFACT_SECTION: &str = "apxm.optimization-summary.v1";
/// Wire version for [`OptimizationSummaryV1`].
pub const OPTIMIZATION_SUMMARY_VERSION: u16 = 1;

/// Reusable compiler analyses whose preservation and invalidation are recorded
/// by pipeline stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompilerAnalysisKind {
    PromptContract,
    EffectAuthority,
    DagUse,
    TokenCost,
    ProfileCost,
    BackendLegality,
}

/// Conservative determinism classification for a node or subgraph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Determinism {
    Proven,
    #[default]
    Unknown,
    NonDeterministic,
}

impl Determinism {
    /// Every determinism value accepted by the optimization-summary contract.
    pub const ALL: [Self; 3] = [Self::Proven, Self::Unknown, Self::NonDeterministic];
}

/// Conservative replay classification for an observable computation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReplaySafety {
    Safe,
    RequiresCheckpoint,
    #[default]
    Unknown,
}

impl ReplaySafety {
    /// Every replay-safety value accepted by the optimization-summary contract.
    pub const ALL: [Self; 3] = [Self::Safe, Self::RequiresCheckpoint, Self::Unknown];
}

/// Whether a profile-derived value is observed or a deterministic fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CostProvenance {
    Observed,
    BackendTier,
    #[default]
    StaticDefault,
    OperationCount,
}

impl CostProvenance {
    /// Every cost-provenance value accepted by the optimization-summary contract.
    pub const ALL: [Self; 4] = [
        Self::Observed,
        Self::BackendTier,
        Self::StaticDefault,
        Self::OperationCount,
    ];
}

/// Stable optimization families reported by compiler artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationTransformKind {
    SharedPrefixReuse,
    TemplateSpecialization,
    DeadContextElimination,
    SchemaNarrowing,
    BackendSelection,
    Scheduling,
    CheckpointPlacement,
    Memoization,
    Batching,
    AskFusion,
    MemoryCondensation,
    SemanticCaching,
    Speculation,
}

impl OptimizationTransformKind {
    /// Every transform family accepted by the optimization-summary contract.
    pub const ALL: [Self; 13] = [
        Self::SharedPrefixReuse,
        Self::TemplateSpecialization,
        Self::DeadContextElimination,
        Self::SchemaNarrowing,
        Self::BackendSelection,
        Self::Scheduling,
        Self::CheckpointPlacement,
        Self::Memoization,
        Self::Batching,
        Self::AskFusion,
        Self::MemoryCondensation,
        Self::SemanticCaching,
        Self::Speculation,
    ];
}

/// Compiler disposition for one optimization family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationDisposition {
    Applied,
    Rejected,
    Experimental,
}

impl OptimizationDisposition {
    /// Every disposition accepted by the optimization-summary contract.
    pub const ALL: [Self; 3] = [Self::Applied, Self::Rejected, Self::Experimental];
}

/// Explainable compiler decision attached to one operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationDecisionV1 {
    pub transform: OptimizationTransformKind,
    pub disposition: OptimizationDisposition,
    #[serde(default)]
    pub reasons: Vec<String>,
}

/// One compiler-produced summary for an artifact.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OptimizationSummaryV1 {
    pub schema_version: u16,
    #[serde(default)]
    pub dags: Vec<DagOptimizationSummaryV1>,
}

impl OptimizationSummaryV1 {
    pub fn new(dags: Vec<DagOptimizationSummaryV1>) -> Self {
        Self {
            schema_version: OPTIMIZATION_SUMMARY_VERSION,
            dags,
        }
    }
}

/// Summary of one independently executable DAG.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DagOptimizationSummaryV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub data_edges: usize,
    pub effect_edges: usize,
    pub control_edges: usize,
    pub weighted_critical_path_ms: u64,
    #[serde(default)]
    pub nodes: Vec<OperationOptimizationSummaryV1>,
}

/// Legality result used to gate transformations before profitability chooses
/// among legal alternatives.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TransformationLegality {
    pub may_reorder: bool,
    pub may_duplicate: bool,
    pub may_memoize: bool,
    pub may_batch: bool,
    pub may_speculate: bool,
}

/// Effect, authority, and replay facts inferred conservatively from one node.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EffectAuthoritySummary {
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub writes: Vec<String>,
    #[serde(default)]
    pub permission_operations: Vec<PermissionOperation>,
    pub approval_required: bool,
    pub determinism: Determinism,
    pub idempotent: bool,
    pub replay_safety: ReplaySafety,
}

/// Prompt-channel facts used by context, token, prefix, and memoization work.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptContractSummary {
    #[serde(default)]
    pub input_roles: Vec<PromptInputRole>,
    pub protected_input_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokenizer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_prefix_est_tokens: Option<u32>,
}

/// Token and profile cost estimates with explicit provenance.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostSummary {
    pub static_prompt_tokens: u64,
    pub estimated_dynamic_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    pub estimated_latency_ms: u64,
    pub latency_provenance: CostProvenance,
    pub sample_count: u64,
}

/// Backend features required by a node's authored semantic contract.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackendLegalityRequirements {
    pub requires_tools: bool,
    pub requires_structured_output: bool,
    pub requires_prefix_reuse: bool,
    pub requires_thinking: bool,
}

/// Typed per-operation optimization evidence.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OperationOptimizationSummaryV1 {
    pub node_id: NodeId,
    pub operation: String,
    pub effect_authority: EffectAuthoritySummary,
    pub prompt: PromptContractSummary,
    pub cost: CostSummary,
    pub backend: BackendLegalityRequirements,
    pub legality: TransformationLegality,
    #[serde(default)]
    pub decisions: Vec<OptimizationDecisionV1>,
    #[serde(default)]
    pub data_inputs: Vec<NodeId>,
    #[serde(default)]
    pub effect_inputs: Vec<NodeId>,
    #[serde(default)]
    pub control_inputs: Vec<NodeId>,
}
