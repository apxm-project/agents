// AUTO-GENERATED from APXM Wave 1 context contract schemas; DO NOT EDIT.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextLayerKind {
    #[serde(rename = "platform_policy")]
    PlatformPolicy,
    #[serde(rename = "agent_instructions")]
    AgentInstructions,
    #[serde(rename = "developer_instructions")]
    DeveloperInstructions,
    #[serde(rename = "selected_skill_instructions")]
    SelectedSkillInstructions,
    #[serde(rename = "session_and_memory_data")]
    SessionAndMemoryData,
    #[serde(rename = "current_input")]
    CurrentInput,
    #[serde(rename = "tool_results")]
    ToolResults,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextTrust {
    #[serde(rename = "platform_instruction")]
    PlatformInstruction,
    #[serde(rename = "agent_instruction")]
    AgentInstruction,
    #[serde(rename = "developer_instruction")]
    DeveloperInstruction,
    #[serde(rename = "selected_skill_instruction")]
    SelectedSkillInstruction,
    #[serde(rename = "trusted_policy_hook")]
    TrustedPolicyHook,
    #[serde(rename = "data")]
    Data,
    #[serde(rename = "tool_result")]
    ToolResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextRedaction {
    #[serde(rename = "public")]
    Public,
    #[serde(rename = "sensitive")]
    Sensitive,
    #[serde(rename = "secret_ref")]
    SecretRef,
    #[serde(rename = "redacted")]
    Redacted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextFrameDisposition {
    #[serde(rename = "prompt")]
    Prompt,
    #[serde(rename = "reference")]
    Reference,
}

pub const PRINCIPAL_SUBJECT_TYPE_USER: &str = "user";
pub const PRINCIPAL_SUBJECT_TYPE_SERVICE: &str = "service";
pub const PRINCIPAL_SUBJECT_TYPE_AGENT: &str = "agent";
pub const PRINCIPAL_SUBJECT_TYPE_HOST: &str = "host";
pub const MODEL_REQUIREMENTS_LOCALITY_ANY: &str = "any";
pub const MODEL_REQUIREMENTS_LOCALITY_LOCAL: &str = "local";
pub const MODEL_REQUIREMENTS_LOCALITY_REMOTE: &str = "remote";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub tenant_id: String,
    pub principal_id: String,
    pub subject_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetSet {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tool_calls: u64,
    pub memory_bytes: u64,
    pub concurrency: u64,
    pub effects: u64,
    pub wall_clock_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantReference {
    pub grant_id: String,
    pub capability_id: String,
    pub operations: Vec<String>,
    pub resources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialReference {
    pub credential_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cancellation {
    pub token_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceContext {
    pub trace_id: String,
    pub span_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequirements {
    pub tools: bool,
    pub structured_output: bool,
    pub thinking: bool,
    pub vision: bool,
    pub locality: String,
    pub minimum_context_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedSkillContext {
    pub id: String,
    pub source_ref: String,
    pub manifest_digest: String,
    pub instruction_digest: String,
    pub instruction_ref: String,
    pub scope: String,
    pub activation_turn: u64,
    pub selection_evidence_ref: String,
    pub token_cost: u64,
    pub dependency_digests: Vec<String>,
}

pub const ACTIVE_SKILL_CONTEXT_SCHEMA_VERSION: &str = "apxm.active-skill-context.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveSkillContext {
    pub schema_version: String,
    pub context_id: String,
    pub invocation_id: String,
    pub user_visible_directive: String,
    pub structured_ids: Vec<String>,
    pub selected: Vec<SelectedSkillContext>,
    pub sealed_digest: String,
    pub assembled_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramPackageReference {
    pub package_id: String,
    pub digest: String,
    pub entry_flow: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationLineage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_invocation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationInput {
    pub kind: String,
    pub input_ref: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostAttestation {
    pub host_id: String,
    pub attestation_ref: String,
    pub context_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingConstraints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_profile_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_profile_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

pub const AGENT_INVOCATION_ENVELOPE_SCHEMA_VERSION: &str = "apxm.agent-invocation-envelope.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInvocationEnvelope {
    pub schema_version: String,
    pub invocation_id: String,
    pub agent_id: String,
    pub program_package: ProgramPackageReference,
    pub principal: Principal,
    pub tenant_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage: Option<InvocationLineage>,
    pub input: InvocationInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_skill_context_ref: Option<String>,
    pub context_ref: String,
    pub host_attestation: HostAttestation,
    pub capability_grants: Vec<GrantReference>,
    pub credential_refs: Vec<CredentialReference>,
    pub budgets: BudgetSet,
    pub model_requirements: ModelRequirements,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_constraints: Option<RoutingConstraints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect_policy_ref: Option<String>,
    pub cancellation: Cancellation,
    pub trace: TraceContext,
}

pub const AUTHENTICATED_PRINCIPAL_CLAIMS_SCHEMA_VERSION: &str = "apxm.authenticated-principal-attestation.v1";
pub const AUTHENTICATED_PRINCIPAL_CLAIMS_AUDIENCE: &str = "apxm.server";
pub const AUTHENTICATED_PRINCIPAL_CLAIMS_KEY_ID: &str = "auth-server-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedPrincipalClaims {
    pub schema_version: String,
    pub audience: String,
    pub principal: Principal,
    pub request_nonce: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub key_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedPrincipalAttestation {
    pub claims: AuthenticatedPrincipalClaims,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildExecutionLineage {
    pub parent_execution_id: String,
    pub child_execution_id: String,
    pub parent_session_id: Option<String>,
    pub child_session_id: Option<String>,
    pub delegation_ref: String,
}

pub const SELECTED_SKILL_DELEGATION_SCOPE: &str = "descendants";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedSkillDelegation {
    pub skill_id: String,
    pub root_selection_evidence_ref: String,
    pub scope: String,
    pub manifest_digest: String,
    pub instruction_digest: String,
}

pub const CHILD_EXECUTION_ENVELOPE_SCHEMA_VERSION: &str = "apxm.child-execution-envelope.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildExecutionEnvelope {
    pub schema_version: String,
    pub lineage: ChildExecutionLineage,
    pub program_package: ProgramPackageReference,
    pub capability_grants: Vec<GrantReference>,
    pub credential_refs: Vec<CredentialReference>,
    pub selected_skill_delegations: Vec<SelectedSkillDelegation>,
    pub context_ref: String,
    pub context_digest: String,
    pub context_policy_ref: String,
    pub memory_policy_ref: String,
    pub budgets: BudgetSet,
    pub cancellation: Cancellation,
    pub effect_policy_ref: String,
    pub idempotency_key: String,
    pub trace: TraceContext,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextDataBudgets {
    pub session_state: u64,
    pub memory: u64,
    pub beliefs: u64,
    pub transcript: u64,
    pub retrieval: u64,
    pub upstream_outputs: u64,
    pub tool_results: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextCompactionPolicy {
    pub enabled: bool,
    pub deterministic: bool,
    pub preserve_provenance: bool,
    pub reinject_exact_instructions: bool,
}

pub const CONTEXT_HOOK_POLICY_ORDINARY_HOOK_TRUST: &str = "data";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextHookPolicy {
    pub ordinary_hook_trust: String,
    pub trusted_instruction_hook_policy_ref: String,
}

pub const CONTEXT_ASSEMBLY_POLICY_SCHEMA_VERSION: &str = "apxm.context-assembly-policy.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextAssemblyPolicy {
    pub schema_version: String,
    pub policy_id: String,
    pub policy_digest: String,
    pub ordered_layers: Vec<String>,
    pub mandatory_instruction_kinds: Vec<String>,
    pub trusted_instruction_sources: Vec<String>,
    pub data_budgets: ContextDataBudgets,
    pub compaction: ContextCompactionPolicy,
    pub hooks: ContextHookPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextLayer {
    pub kind: ContextLayerKind,
    pub position: u64,
    pub provenance_ref: String,
    pub trust: ContextTrust,
    pub redaction: ContextRedaction,
    pub token_budget: u64,
    pub token_cost: u64,
    pub frame_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextFrame {
    pub frame_id: String,
    pub layer: ContextLayerKind,
    pub role: String,
    pub source_ref: String,
    pub digest: String,
    pub trust: ContextTrust,
    pub redaction: ContextRedaction,
    pub scope: String,
    pub truncation: String,
    pub token_cost: u64,
    pub content_ref: String,
    pub disposition: ContextFrameDisposition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSchema {
    pub grant_id: String,
    pub capability_id: String,
    pub schema_ref: String,
}

pub const MODEL_CONTEXT_ENVELOPE_SCHEMA_VERSION: &str = "apxm.model-context-envelope.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelContextEnvelope {
    pub schema_version: String,
    pub context_id: String,
    pub invocation_id: String,
    pub policy_ref: String,
    pub layers: Vec<ContextLayer>,
    pub frames: Vec<ContextFrame>,
    pub tool_schemas: Vec<ToolSchema>,
    pub model_requirements: ModelRequirements,
    pub sealed_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextLifecycleEventPayload {
    pub lifecycle_kind: String,
    pub context_id: String,
    pub invocation_id: String,
    pub context_digest: String,
    pub policy_ref: String,
    pub frame_count: u64,
    pub token_count: u64,
    pub content_redacted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributor_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contribution_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contribution_trust: Option<String>,
}
