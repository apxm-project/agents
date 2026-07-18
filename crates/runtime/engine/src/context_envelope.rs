//! Generic sealed model-context transport and validation.
//!
//! The trusted host resolves its durable pin before execution and supplies this
//! transport as data. The runtime verifies the canonical policy, envelope,
//! references, and content digests before constructing model messages; it does
//! not resolve catalogues, activate skills, or grant authority.

use std::collections::{BTreeSet, HashMap};

use apxm_backends::llm::backends::request::{Message, Role};
use apxm_core::types::context_contracts::{
    ContextAssemblyPolicy, ContextFrame, ContextFrameDisposition, ContextLayer, ContextLayerKind,
    ContextRedaction, ContextTrust, ModelContextEnvelope,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA_VERSION: &str = "apxm.sealed-context-runtime-transport.v1";
pub const MODEL_CONTEXT_ENVELOPE_SCHEMA_VERSION: &str = "apxm.model-context-envelope.v1";
pub const CONTEXT_ASSEMBLY_POLICY_SCHEMA_VERSION: &str = "apxm.context-assembly-policy.v1";
pub const POLICY_REFERENCE_PREFIX: &str = "sealed-context-policy";

const ORDERED_LAYER_KINDS: &[ContextLayerKind] = &[
    ContextLayerKind::PlatformPolicy,
    ContextLayerKind::AgentInstructions,
    ContextLayerKind::DeveloperInstructions,
    ContextLayerKind::SelectedSkillInstructions,
    ContextLayerKind::SessionAndMemoryData,
    ContextLayerKind::CurrentInput,
    ContextLayerKind::ToolResults,
];

/// Exact pinned instruction bytes supplied by the host after durable-pin
/// verification. The runtime uses only the reference and digest relationship.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedContextInstructionContent {
    pub content_ref: String,
    pub instruction_digest: String,
    pub content: String,
}

/// Host-to-runtime transport for a previously verified sealed context pin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedContextRuntimeTransport {
    pub schema_version: String,
    pub pin_id: String,
    pub sealed_identity_digest: String,
    pub policy: ContextAssemblyPolicy,
    pub model_context: ModelContextEnvelope,
    pub instruction_contents: Vec<SealedContextInstructionContent>,
}

/// Generic model messages resolved from a sealed context transport.
#[derive(Debug, Clone)]
pub struct ResolvedModelContext {
    pub messages: Vec<Message>,
    pub policy_ref: String,
    pub sealed_digest: String,
}

/// Redacted identity and aggregate counts for one validated sealed context.
///
/// Runtime observability uses this projection when a trusted hook contributes
/// a future instruction frame. It deliberately excludes every frame's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextLifecycleMetadata {
    pub context_id: String,
    pub invocation_id: String,
    pub context_digest: String,
    pub policy_ref: String,
    pub frame_count: u64,
    pub token_count: u64,
    pub trusted_instruction_hook_policy_ref: String,
}

/// Typed validation failure for a host-provided sealed context transport.
#[derive(Debug, thiserror::Error)]
pub enum SealedContextTransportError {
    #[error("sealed context transport is malformed: {0}")]
    Malformed(String),
    #[error("sealed context transport failed integrity validation: {0}")]
    Integrity(String),
}

/// Build the canonical policy reference for a verified policy digest.
pub fn policy_reference(policy_digest: &str) -> String {
    format!("{POLICY_REFERENCE_PREFIX}:{policy_digest}")
}

/// Resolve a serialized host transport into generic provider messages.
pub fn resolve_serialized(
    serialized: &str,
) -> Result<ResolvedModelContext, SealedContextTransportError> {
    let transport: SealedContextRuntimeTransport = serde_json::from_str(serialized)
        .map_err(|error| SealedContextTransportError::Malformed(error.to_string()))?;
    resolve(&transport)
}

/// Read the redacted lifecycle projection from a validated host transport.
pub fn lifecycle_metadata_from_serialized(
    serialized: &str,
) -> Result<ContextLifecycleMetadata, SealedContextTransportError> {
    let transport: SealedContextRuntimeTransport = serde_json::from_str(serialized)
        .map_err(|error| SealedContextTransportError::Malformed(error.to_string()))?;
    validate_transport(&transport)?;
    Ok(ContextLifecycleMetadata {
        context_id: transport.model_context.context_id,
        invocation_id: transport.model_context.invocation_id,
        context_digest: transport.sealed_identity_digest,
        policy_ref: transport.model_context.policy_ref,
        frame_count: transport.model_context.frames.len() as u64,
        token_count: transport
            .model_context
            .frames
            .iter()
            .map(|frame| frame.token_cost)
            .sum(),
        trusted_instruction_hook_policy_ref: transport
            .policy
            .hooks
            .trusted_instruction_hook_policy_ref,
    })
}

/// Verify a host-sealed transport and construct only generic model messages.
pub fn resolve(
    transport: &SealedContextRuntimeTransport,
) -> Result<ResolvedModelContext, SealedContextTransportError> {
    validate_transport(transport)?;
    let content_by_ref = transport
        .instruction_contents
        .iter()
        .map(|content| (content.content_ref.as_str(), content))
        .collect::<HashMap<_, _>>();
    let mut messages = Vec::with_capacity(transport.instruction_contents.len());
    for frame in transport
        .model_context
        .frames
        .iter()
        .filter(|frame| frame.disposition == ContextFrameDisposition::Prompt)
    {
        let content = content_by_ref
            .get(frame.content_ref.as_str())
            .ok_or_else(|| {
                SealedContextTransportError::Integrity(format!(
                    "frame '{}' references missing content '{}'",
                    frame.frame_id, frame.content_ref
                ))
            })?;
        messages.push(Message::text(
            role_for_frame(&frame.role)?,
            content.content.clone(),
        ));
    }
    Ok(ResolvedModelContext {
        messages,
        policy_ref: transport.model_context.policy_ref.clone(),
        sealed_digest: transport.sealed_identity_digest.clone(),
    })
}

fn validate_transport(
    transport: &SealedContextRuntimeTransport,
) -> Result<(), SealedContextTransportError> {
    if transport.schema_version != SCHEMA_VERSION {
        return Err(SealedContextTransportError::Integrity(
            "unexpected transport schema version".to_string(),
        ));
    }
    if transport.pin_id.trim().is_empty() || !is_sha256_digest(&transport.sealed_identity_digest) {
        return Err(SealedContextTransportError::Integrity(
            "pin identity is incomplete".to_string(),
        ));
    }
    if transport.policy.schema_version != CONTEXT_ASSEMBLY_POLICY_SCHEMA_VERSION
        || transport.model_context.schema_version != MODEL_CONTEXT_ENVELOPE_SCHEMA_VERSION
        || transport.model_context.context_id != transport.pin_id
        || transport.model_context.invocation_id.trim().is_empty()
        || transport.model_context.sealed_digest != transport.sealed_identity_digest
        || !matches!(
            transport.model_context.model_requirements.locality.as_str(),
            "any" | "local"
        )
    {
        return Err(SealedContextTransportError::Integrity(
            "envelope identity does not match the sealed pin".to_string(),
        ));
    }
    validate_policy(&transport.policy)?;
    if transport.model_context.policy_ref != policy_reference(&transport.policy.policy_digest) {
        return Err(SealedContextTransportError::Integrity(
            "model envelope policy reference does not match its sealed policy".to_string(),
        ));
    }
    validate_layers_and_frames(transport)
}

fn validate_policy(policy: &ContextAssemblyPolicy) -> Result<(), SealedContextTransportError> {
    if policy.policy_id.trim().is_empty()
        || !is_sha256_digest(&policy.policy_digest)
        || !policy
            .ordered_layers
            .iter()
            .map(String::as_str)
            .eq(ORDERED_LAYER_KINDS.iter().map(layer_kind_name))
        || (policy.mandatory_instruction_kinds.is_empty()
            != policy.trusted_instruction_sources.is_empty())
        || !policy.compaction.deterministic
        || !policy.compaction.preserve_provenance
        || !policy.compaction.reinject_exact_instructions
        || policy.hooks.ordinary_hook_trust != "data"
        || policy
            .hooks
            .trusted_instruction_hook_policy_ref
            .trim()
            .is_empty()
    {
        return Err(SealedContextTransportError::Integrity(
            "context policy violates the canonical assembly invariants".to_string(),
        ));
    }
    let mut canonical = policy.clone();
    canonical.policy_digest.clear();
    let actual = sha256_digest(
        &serde_json::to_vec(&canonical)
            .map_err(|error| SealedContextTransportError::Malformed(error.to_string()))?,
    );
    if actual != policy.policy_digest {
        return Err(SealedContextTransportError::Integrity(
            "context policy digest does not match its contents".to_string(),
        ));
    }
    Ok(())
}

fn validate_layers_and_frames(
    transport: &SealedContextRuntimeTransport,
) -> Result<(), SealedContextTransportError> {
    validate_layers(&transport.model_context.layers)?;
    let mut content_by_ref = HashMap::new();
    for content in &transport.instruction_contents {
        if content.content_ref.trim().is_empty()
            || !is_sha256_digest(&content.instruction_digest)
            || content.content.trim().is_empty()
            || sha256_digest(content.content.as_bytes()) != content.instruction_digest
            || content_by_ref
                .insert(content.content_ref.as_str(), content)
                .is_some()
        {
            return Err(SealedContextTransportError::Integrity(
                "sealed content references or digests are invalid".to_string(),
            ));
        }
    }
    let mut frame_ids = BTreeSet::new();
    let mut prompt_content_refs = BTreeSet::<String>::new();
    let mut present_instruction_kinds = BTreeSet::<String>::new();
    for frame in &transport.model_context.frames {
        if frame.frame_id.trim().is_empty()
            || frame.source_ref.trim().is_empty()
            || !is_sha256_digest(&frame.digest)
            || frame.content_ref.trim().is_empty()
            || !matches!(
                frame.scope.as_str(),
                "turn" | "session" | "child" | "replay"
            )
            || !matches!(
                frame.truncation.as_str(),
                "forbidden" | "deterministic_budget" | "compact_with_provenance"
            )
            || !frame_ids.insert(frame.frame_id.as_str())
        {
            return Err(SealedContextTransportError::Integrity(
                "model frame identity is invalid".to_string(),
            ));
        }
        let layer = transport
            .model_context
            .layers
            .get(frame.layer as usize)
            .ok_or_else(|| {
                SealedContextTransportError::Integrity("frame layer is not canonical".to_string())
            })?;
        if layer.kind != frame.layer || !layer.frame_ids.iter().any(|id| id == &frame.frame_id) {
            return Err(SealedContextTransportError::Integrity(
                "frame is not owned by its declared layer".to_string(),
            ));
        }
        validate_frame_disposition(
            frame,
            layer,
            transport,
            &content_by_ref,
            &mut prompt_content_refs,
            &mut present_instruction_kinds,
        )?;
    }
    for layer in &transport.model_context.layers {
        let accounted = transport
            .model_context
            .frames
            .iter()
            .filter(|frame| frame.layer == layer.kind)
            .map(|frame| frame.token_cost)
            .sum::<u64>();
        if accounted != layer.token_cost {
            return Err(SealedContextTransportError::Integrity(
                "layer token accounting does not match its frames".to_string(),
            ));
        }
    }
    let layered_frame_ids = transport
        .model_context
        .layers
        .iter()
        .flat_map(|layer| layer.frame_ids.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    if layered_frame_ids != frame_ids
        || prompt_content_refs.len() != content_by_ref.len()
        || transport
            .policy
            .mandatory_instruction_kinds
            .iter()
            .any(|kind| !present_instruction_kinds.contains(kind))
    {
        return Err(SealedContextTransportError::Integrity(
            "sealed context omits a mandatory instruction frame or has unreferenced content"
                .to_string(),
        ));
    }
    let mut grants = BTreeSet::new();
    for tool in &transport.model_context.tool_schemas {
        if tool.grant_id.trim().is_empty()
            || tool.capability_id.trim().is_empty()
            || tool.schema_ref.trim().is_empty()
            || !grants.insert(tool.grant_id.as_str())
        {
            return Err(SealedContextTransportError::Integrity(
                "model context tool schemas are not uniquely admitted".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_layers(layers: &[ContextLayer]) -> Result<(), SealedContextTransportError> {
    if layers.len() != ORDERED_LAYER_KINDS.len() {
        return Err(SealedContextTransportError::Integrity(
            "model context must contain every canonical layer".to_string(),
        ));
    }
    for (position, (layer, expected_kind)) in layers.iter().zip(ORDERED_LAYER_KINDS).enumerate() {
        if layer.kind != *expected_kind
            || layer.position != position as u64
            || layer.provenance_ref.trim().is_empty()
            || layer.trust != layer_trust(*expected_kind)
            || layer.redaction != layer_redaction(*expected_kind)
            || (layer.token_budget != 0 && layer.token_cost > layer.token_budget)
            || layer.frame_ids.iter().collect::<BTreeSet<_>>().len() != layer.frame_ids.len()
        {
            return Err(SealedContextTransportError::Integrity(
                "model context layers are incomplete, unordered, or trust-invalid".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_frame_disposition(
    frame: &ContextFrame,
    layer: &ContextLayer,
    transport: &SealedContextRuntimeTransport,
    content_by_ref: &HashMap<&str, &SealedContextInstructionContent>,
    prompt_content_refs: &mut BTreeSet<String>,
    present_instruction_kinds: &mut BTreeSet<String>,
) -> Result<(), SealedContextTransportError> {
    if layer.token_cost < frame.token_cost
        || frame.trust != layer.trust
        || frame.redaction != layer.redaction
    {
        return Err(SealedContextTransportError::Integrity(
            "frame accounting or authority differs from its layer".to_string(),
        ));
    }
    match frame.disposition {
        ContextFrameDisposition::Prompt => {
            if !instruction_trust(frame.trust)
                || !matches!(frame.role.as_str(), "system" | "developer")
                || frame.truncation != "forbidden"
                || !prompt_content_refs.insert(frame.content_ref.clone())
            {
                return Err(SealedContextTransportError::Integrity(
                    "only exact trusted instruction frames may become model prompts".to_string(),
                ));
            }
            let content = content_by_ref
                .get(frame.content_ref.as_str())
                .ok_or_else(|| {
                    SealedContextTransportError::Integrity(format!(
                        "prompt frame '{}' references unknown sealed content",
                        frame.frame_id
                    ))
                })?;
            if content.instruction_digest != frame.digest {
                return Err(SealedContextTransportError::Integrity(format!(
                    "prompt frame '{}' digest does not match its content reference",
                    frame.frame_id
                )));
            }
            if frame.trust == ContextTrust::SelectedSkillInstruction
                && (!transport
                    .policy
                    .trusted_instruction_sources
                    .iter()
                    .any(|source| source == &frame.source_ref)
                    || frame.token_cost == 0)
            {
                return Err(SealedContextTransportError::Integrity(
                    "selected-skill prompt frame is not trusted by the sealed policy".to_string(),
                ));
            }
            present_instruction_kinds.insert(trust_name(frame.trust).to_string());
            Ok(())
        }
        ContextFrameDisposition::Reference => {
            if instruction_trust(frame.trust)
                || content_by_ref.contains_key(frame.content_ref.as_str())
                || !matches!(frame.role.as_str(), "user" | "assistant" | "tool")
            {
                return Err(SealedContextTransportError::Integrity(
                    "reference data cannot acquire prompt or instruction authority".to_string(),
                ));
            }
            Ok(())
        }
    }
}

fn instruction_trust(trust: ContextTrust) -> bool {
    matches!(
        trust,
        ContextTrust::PlatformInstruction
            | ContextTrust::AgentInstruction
            | ContextTrust::DeveloperInstruction
            | ContextTrust::SelectedSkillInstruction
            | ContextTrust::TrustedPolicyHook
    )
}

fn layer_kind_name(kind: &ContextLayerKind) -> &'static str {
    match kind {
        ContextLayerKind::PlatformPolicy => "platform_policy",
        ContextLayerKind::AgentInstructions => "agent_instructions",
        ContextLayerKind::DeveloperInstructions => "developer_instructions",
        ContextLayerKind::SelectedSkillInstructions => "selected_skill_instructions",
        ContextLayerKind::SessionAndMemoryData => "session_and_memory_data",
        ContextLayerKind::CurrentInput => "current_input",
        ContextLayerKind::ToolResults => "tool_results",
    }
}

fn layer_trust(kind: ContextLayerKind) -> ContextTrust {
    match kind {
        ContextLayerKind::PlatformPolicy => ContextTrust::PlatformInstruction,
        ContextLayerKind::AgentInstructions => ContextTrust::AgentInstruction,
        ContextLayerKind::DeveloperInstructions => ContextTrust::DeveloperInstruction,
        ContextLayerKind::SelectedSkillInstructions => ContextTrust::SelectedSkillInstruction,
        ContextLayerKind::SessionAndMemoryData | ContextLayerKind::CurrentInput => {
            ContextTrust::Data
        }
        ContextLayerKind::ToolResults => ContextTrust::ToolResult,
    }
}

fn layer_redaction(kind: ContextLayerKind) -> ContextRedaction {
    match kind {
        ContextLayerKind::PlatformPolicy => ContextRedaction::Public,
        _ => ContextRedaction::Sensitive,
    }
}

fn trust_name(trust: ContextTrust) -> &'static str {
    match trust {
        ContextTrust::PlatformInstruction => "platform_instruction",
        ContextTrust::AgentInstruction => "agent_instruction",
        ContextTrust::DeveloperInstruction => "developer_instruction",
        ContextTrust::SelectedSkillInstruction => "selected_skill_instruction",
        ContextTrust::TrustedPolicyHook => "trusted_policy_hook",
        ContextTrust::Data => "data",
        ContextTrust::ToolResult => "tool_result",
    }
}

fn role_for_frame(role: &str) -> Result<Role, SealedContextTransportError> {
    match role {
        "system" => Ok(Role::System),
        "developer" => Ok(Role::System),
        "user" => Ok(Role::User),
        "assistant" => Ok(Role::Assistant),
        "tool" => Ok(Role::Tool),
        _ => Err(SealedContextTransportError::Integrity(
            "model frame role is not recognized".to_string(),
        )),
    }
}

fn is_sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::context_contracts::{
        ContextCompactionPolicy, ContextDataBudgets, ContextFrame, ContextHookPolicy, ContextLayer,
        ModelRequirements,
    };

    fn layers(selected_frame_ids: Vec<String>, selected_token_cost: u64) -> Vec<ContextLayer> {
        vec![
            ContextLayer {
                kind: ContextLayerKind::PlatformPolicy,
                position: 0,
                provenance_ref: "policy://platform/base".to_string(),
                trust: ContextTrust::PlatformInstruction,
                redaction: ContextRedaction::Public,
                token_budget: 0,
                token_cost: 0,
                frame_ids: Vec::new(),
            },
            ContextLayer {
                kind: ContextLayerKind::AgentInstructions,
                position: 1,
                provenance_ref: "artifact://agent/program".to_string(),
                trust: ContextTrust::AgentInstruction,
                redaction: ContextRedaction::Sensitive,
                token_budget: 0,
                token_cost: 0,
                frame_ids: Vec::new(),
            },
            ContextLayer {
                kind: ContextLayerKind::DeveloperInstructions,
                position: 2,
                provenance_ref: "policy://developer/default".to_string(),
                trust: ContextTrust::DeveloperInstruction,
                redaction: ContextRedaction::Sensitive,
                token_budget: 0,
                token_cost: 0,
                frame_ids: Vec::new(),
            },
            ContextLayer {
                kind: ContextLayerKind::SelectedSkillInstructions,
                position: 3,
                provenance_ref: "selection://review".to_string(),
                trust: ContextTrust::SelectedSkillInstruction,
                redaction: ContextRedaction::Sensitive,
                token_budget: selected_token_cost,
                token_cost: selected_token_cost,
                frame_ids: selected_frame_ids,
            },
            ContextLayer {
                kind: ContextLayerKind::SessionAndMemoryData,
                position: 4,
                provenance_ref: "context://memory/none".to_string(),
                trust: ContextTrust::Data,
                redaction: ContextRedaction::Sensitive,
                token_budget: 0,
                token_cost: 0,
                frame_ids: Vec::new(),
            },
            ContextLayer {
                kind: ContextLayerKind::CurrentInput,
                position: 5,
                provenance_ref: "input://invocation-1".to_string(),
                trust: ContextTrust::Data,
                redaction: ContextRedaction::Sensitive,
                token_budget: 0,
                token_cost: 0,
                frame_ids: Vec::new(),
            },
            ContextLayer {
                kind: ContextLayerKind::ToolResults,
                position: 6,
                provenance_ref: "context://tool-results/none".to_string(),
                trust: ContextTrust::ToolResult,
                redaction: ContextRedaction::Sensitive,
                token_budget: 0,
                token_cost: 0,
                frame_ids: Vec::new(),
            },
        ]
    }

    fn transport() -> SealedContextRuntimeTransport {
        let content = "Use the supplied review process.".to_string();
        let instruction_digest = sha256_digest(content.as_bytes());
        let mut policy = ContextAssemblyPolicy {
            schema_version: CONTEXT_ASSEMBLY_POLICY_SCHEMA_VERSION.to_string(),
            policy_id: "selected-context-policy".to_string(),
            policy_digest: String::new(),
            ordered_layers: ORDERED_LAYER_KINDS
                .iter()
                .map(layer_kind_name)
                .map(str::to_string)
                .collect(),
            mandatory_instruction_kinds: vec!["selected_skill_instruction".to_string()],
            trusted_instruction_sources: vec!["host://context/review".to_string()],
            data_budgets: ContextDataBudgets {
                session_state: 0,
                memory: 0,
                beliefs: 0,
                transcript: 0,
                retrieval: 0,
                upstream_outputs: 0,
                tool_results: 0,
            },
            compaction: ContextCompactionPolicy {
                enabled: false,
                deterministic: true,
                preserve_provenance: true,
                reinject_exact_instructions: true,
            },
            hooks: ContextHookPolicy {
                ordinary_hook_trust: "data".to_string(),
                trusted_instruction_hook_policy_ref: "policy://trusted-hooks".to_string(),
            },
        };
        policy.policy_digest = sha256_digest(
            &serde_json::to_vec(&policy).expect("test policy serializes deterministically"),
        );
        let sealed_identity_digest = sha256_digest(b"sealed-context-identity");
        SealedContextRuntimeTransport {
            schema_version: SCHEMA_VERSION.to_string(),
            pin_id: "context-pin-1".to_string(),
            sealed_identity_digest: sealed_identity_digest.clone(),
            policy: policy.clone(),
            model_context: ModelContextEnvelope {
                schema_version: MODEL_CONTEXT_ENVELOPE_SCHEMA_VERSION.to_string(),
                context_id: "context-pin-1".to_string(),
                invocation_id: "invocation-1".to_string(),
                policy_ref: policy_reference(&policy.policy_digest),
                layers: layers(vec!["frame-1".to_string()], 7),
                frames: vec![ContextFrame {
                    frame_id: "frame-1".to_string(),
                    layer: ContextLayerKind::SelectedSkillInstructions,
                    role: "developer".to_string(),
                    source_ref: "host://context/review".to_string(),
                    digest: instruction_digest.clone(),
                    trust: ContextTrust::SelectedSkillInstruction,
                    redaction: ContextRedaction::Sensitive,
                    scope: "turn".to_string(),
                    truncation: "forbidden".to_string(),
                    token_cost: 7,
                    content_ref: "sealed-content-1".to_string(),
                    disposition: ContextFrameDisposition::Prompt,
                }],
                tool_schemas: Vec::new(),
                model_requirements: ModelRequirements {
                    tools: false,
                    structured_output: false,
                    thinking: false,
                    vision: false,
                    locality: "any".to_string(),
                    minimum_context_tokens: 0,
                },
                sealed_digest: sealed_identity_digest,
            },
            instruction_contents: vec![SealedContextInstructionContent {
                content_ref: "sealed-content-1".to_string(),
                instruction_digest,
                content,
            }],
        }
    }

    #[test]
    fn resolves_verified_generic_context_messages() {
        let resolved = resolve(&transport()).expect("valid sealed transport resolves");
        assert_eq!(resolved.messages.len(), 1);
        assert_eq!(resolved.messages[0].role, Role::System);
        assert_eq!(
            resolved.messages[0].text_content(),
            "Use the supplied review process."
        );
    }

    #[test]
    fn rejects_content_digest_drift_before_message_construction() {
        let mut invalid = transport();
        invalid.instruction_contents[0].content = "mutated after pinning".to_string();
        assert!(matches!(
            resolve(&invalid),
            Err(SealedContextTransportError::Integrity(_))
        ));
    }

    #[test]
    fn rejects_data_in_an_instruction_role() {
        let mut invalid = transport();
        invalid.model_context.frames[0].trust = ContextTrust::Data;
        assert!(matches!(
            resolve(&invalid),
            Err(SealedContextTransportError::Integrity(_))
        ));
    }

    #[test]
    fn resolves_no_selected_instruction_transport_without_messages() {
        let mut transport = transport();
        transport.policy.mandatory_instruction_kinds.clear();
        transport.policy.trusted_instruction_sources.clear();
        transport.policy.policy_digest.clear();
        transport.policy.policy_digest = sha256_digest(
            &serde_json::to_vec(&transport.policy).expect("policy serializes deterministically"),
        );
        transport.model_context.policy_ref = policy_reference(&transport.policy.policy_digest);
        transport.model_context.frames.clear();
        transport.model_context.layers = layers(Vec::new(), 0);
        transport.instruction_contents.clear();

        let resolved = resolve(&transport).expect("empty selected layer is a valid transport");
        assert!(resolved.messages.is_empty());
    }

    #[test]
    fn reference_only_data_frame_never_becomes_a_model_message() {
        let mut transport = transport();
        transport.policy.mandatory_instruction_kinds.clear();
        transport.policy.trusted_instruction_sources.clear();
        transport.policy.policy_digest.clear();
        transport.policy.policy_digest = sha256_digest(
            &serde_json::to_vec(&transport.policy).expect("policy serializes deterministically"),
        );
        transport.model_context.policy_ref = policy_reference(&transport.policy.policy_digest);
        transport.model_context.frames = vec![ContextFrame {
            frame_id: "frame-input".to_string(),
            layer: ContextLayerKind::CurrentInput,
            role: "user".to_string(),
            source_ref: "input://invocation-1".to_string(),
            digest: sha256_digest(b"durable-input-evidence"),
            trust: ContextTrust::Data,
            redaction: ContextRedaction::Sensitive,
            scope: "turn".to_string(),
            truncation: "deterministic_budget".to_string(),
            token_cost: 0,
            content_ref: "input://invocation-1".to_string(),
            disposition: ContextFrameDisposition::Reference,
        }];
        transport.model_context.layers = layers(Vec::new(), 0);
        transport.model_context.layers[5].frame_ids = vec!["frame-input".to_string()];
        transport.instruction_contents.clear();

        let resolved = resolve(&transport).expect("reference frame is admitted as provenance only");
        assert!(resolved.messages.is_empty());
    }

    #[test]
    fn rejects_reference_data_promoted_to_a_prompt() {
        let mut transport = transport();
        transport.model_context.frames[0].layer = ContextLayerKind::CurrentInput;
        transport.model_context.frames[0].trust = ContextTrust::Data;
        transport.model_context.frames[0].role = "user".to_string();
        transport.model_context.frames[0].disposition = ContextFrameDisposition::Prompt;
        transport.model_context.layers = layers(Vec::new(), 0);
        transport.model_context.layers[5].frame_ids = vec!["frame-1".to_string()];
        transport.model_context.layers[5].token_cost = 7;
        assert!(matches!(
            resolve(&transport),
            Err(SealedContextTransportError::Integrity(_))
        ));
    }

    #[test]
    fn rejects_layer_frame_order_drift() {
        let mut invalid = transport();
        invalid.model_context.layers.swap(2, 3);
        assert!(matches!(
            resolve(&invalid),
            Err(SealedContextTransportError::Integrity(_))
        ));
    }
}
