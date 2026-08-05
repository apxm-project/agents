//! Execute canonical `apxm.air.v1` through the canonical runtime driver.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use apxm_execution::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionReceiver, CompositionRequest, EventAwait, EventOutcome, EventPort, ExecutionRequest,
    NodeOutcome, NoopStaticHookHandler, RuntimeProfile,
};
#[cfg(test)]
use apxm_execution::{ExecutionPortBundle, ExecutionPorts, RuntimeProfileError, execute};
use apxm_inference::{
    AttemptDisposition, InferenceTargetCommitment, ModelBindingAdmission, ModelCallPreparation,
    ModelCallRequest, ModelCallRequestMetadata, ModelCallRequestMetadataPort, ModelInferencePort,
    ModelOutcome, ResolvedModelBinding, TypedError, Usage,
};
use apxm_kernel::{
    AcpPromptOutcome, AcpPromptRequest, AdmittedConfinement, AdmittedPortBinding, AtomicWriteSet,
    ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult,
    ExternalAgentCapabilityPort, InvocationAdmission, PortImplementation, PortSlot,
    ProgramInstanceRef, ProgramInvocationRef, PromptEffectState, ResourceCeilings,
    RuntimeAdmission, VerifiedInvocationAdmission, digest_serializable,
    verify_invocation_admission,
};
use apxm_kernel::{ConfinementAttestation, ConfinementError, ConfinementPort, ConfinementRequest};
#[cfg(test)]
use apxm_kernel::{ExactPortBinding, PortBundle, PortBundleSpec};
use apxm_program::air::{AirModule, SemanticOpKind};
#[cfg(test)]
use apxm_program::artifact::SchemaDigestRef;
use apxm_program::external_agent::{AttributedEvent, AttributedEventKind, PeerUsage};

pub async fn execute_canonical_command(input: PathBuf, _json_output: bool) -> Result<()> {
    let _ = load_canonical_air(&input)?;
    anyhow::bail!("canonical execution requires an explicit APXM Invocation Admission authority");
}

/// One APXM-owned canonical runtime instance. The commit port is retained for
/// the instance lifetime so replay produces the existing idempotent/compare
/// conflict behavior instead of creating a second commit writer.
pub struct CanonicalRuntime {
    commit: Arc<DevCommit>,
}

impl CanonicalRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self {
            commit: Arc::new(DevCommit::default()),
        }
    }

    pub async fn execute(
        &self,
        air: AirModule,
        admission: &InvocationAdmission,
        release_bytes: &[u8],
        provenance_bytes: &[u8],
    ) -> Result<Value> {
        ensure_local_capability_authority_available(&air)?;
        let artifact_bytes = serde_json::to_vec(&air)?;
        let descriptor = reference_host_runtime_descriptor();
        let verified = verify_invocation_admission(
            admission,
            &artifact_bytes,
            release_bytes,
            provenance_bytes,
            &descriptor.port_bindings,
            descriptor.resource_ceilings.clone(),
            &descriptor.confinement,
        )
        .map_err(|error| anyhow::anyhow!(error))?;
        let model_binding_digest = descriptor
            .port_bindings
            .iter()
            .find(|binding| binding.slot == PortSlot::ModelInference.as_str())
            .map(|binding| binding.binding_digest.clone())
            .ok_or_else(|| anyhow::anyhow!("reference runtime model binding is absent"))?;
        let request = ExecutionRequest {
            model_admission: model_admission(&air, &model_binding_digest),
            air,
            hook_bindings: Vec::new(),
            capability_invocations: BTreeMap::new(),
            program_instance_ref: ProgramInstanceRef::new("reference-host.instance"),
            program_invocation_ref: ProgramInvocationRef::new(admission.invocation_id.clone()),
            commit_id: format!("reference-host.commit.{}", admission.invocation_id),
            write_set: reference_write_set(&admission.invocation_id),
        };
        let profile = runtime_profile_from_invocation(
            self.commit.clone(),
            Arc::new(UnavailableModelRequestMetadata),
            verified,
            "reference-host.execution",
        )
        .await?;
        let report = profile
            .execute(request, Value::Null)
            .await
            .map_err(|err| anyhow::anyhow!(err))?;

        Ok(json!({
            "schema_version": "apxm.local-execute-result.v1",
            "runtime": "apxm_execution",
            "status": "completed",
            "content": report.final_context,
            "results": {
                "node_outcomes": report.node_outcomes.iter().map(node_outcome_json).collect::<Vec<_>>(),
                "external_agent_evidence": report.external_agent_evidence,
            },
            "stats": {
                "executed_nodes": report.node_outcomes.len(),
                "failed_nodes": 0,
                "duration_ms": 0,
            },
            "llm_usage": {
                "input_tokens": report.native_usage.input_tokens,
                "output_tokens": report.native_usage.output_tokens,
                "total_requests": report
                    .node_outcomes
                    .iter()
                    .filter(|outcome| matches!(outcome, NodeOutcome::Model { .. }))
                    .count(),
            },
            "commit": commit_result_json(&report.commit),
        }))
    }
}

impl Default for CanonicalRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn load_canonical_air(input: &PathBuf) -> Result<AirModule> {
    let text = std::fs::read_to_string(input)
        .with_context(|| format!("failed to read canonical AIR from {}", input.display()))?;
    let air: AirModule = serde_json::from_str(&text).with_context(|| {
        format!(
            "{} must contain canonical apxm.air.v1 JSON",
            input.display()
        )
    })?;
    let verdict = air.verify();
    if !verdict.is_accepted() {
        let diagnostics = verdict
            .into_diagnostics()
            .into_iter()
            .map(|diagnostic| {
                json!({
                    "code": diagnostic.code.slug(),
                    "location": diagnostic.location,
                    "message": diagnostic.message,
                })
            })
            .collect::<Vec<_>>();
        anyhow::bail!(
            "canonical AIR verification failed: {}",
            serde_json::to_string(&diagnostics)?
        );
    }
    Ok(air)
}

fn model_targets(air: &AirModule) -> Vec<String> {
    let mut targets = Vec::new();
    for op in &air.semantic_operations {
        if op.op != SemanticOpKind::ModelCall {
            continue;
        }
        let Some(target) = op
            .operands
            .iter()
            .find(|operand| operand.slot == "model_ref")
            .map(|operand| operand.value_id.clone())
        else {
            continue;
        };
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    targets
}

fn ensure_local_capability_authority_available(air: &AirModule) -> Result<()> {
    for operation in &air.semantic_operations {
        if operation.op != SemanticOpKind::CapabilityInvoke {
            continue;
        }
        let capability_ref = operation
            .operands
            .iter()
            .find(|operand| operand.slot == "capability_ref")
            .map(|operand| operand.value_id.as_str())
            .unwrap_or("<missing capability_ref>");
        if !capability_ref.starts_with("external-agent:") {
            anyhow::bail!(
                "canonical local execution cannot invoke Capability {capability_ref} at node {}; explicit Invocation Admission authority is required",
                operation.node_id
            );
        }
    }
    Ok(())
}

fn model_admission(air: &AirModule, model_binding_digest: &str) -> ModelBindingAdmission {
    ModelBindingAdmission::for_invocation(
        model_targets(air)
            .into_iter()
            .map(|target| {
                ResolvedModelBinding::from_target_commitment(
                    InferenceTargetCommitment::commit(
                        target,
                        digest('b'),
                        "reference-host.model",
                        model_binding_digest,
                        digest('c'),
                        digest('d'),
                        1,
                    )
                    .expect("development target commitment"),
                )
            })
            .collect(),
    )
}

fn dev_write_set() -> AtomicWriteSet {
    AtomicWriteSet {
        next_program_state_digest: digest('1'),
        continuation_digest: digest('2'),
        checkpoint_effect_outcomes_digest: digest('3'),
        runtime_evidence_batch_digest: digest('4'),
        usage_facts_digest: digest('5'),
        session_output_refs_digest: digest('6'),
    }
}

fn reference_write_set(invocation_id: &str) -> AtomicWriteSet {
    let digest_for = |member: &str| digest_text(&format!("{invocation_id}\0{member}"));
    AtomicWriteSet {
        next_program_state_digest: digest_for("state_continuation"),
        continuation_digest: digest_for("continuation"),
        checkpoint_effect_outcomes_digest: digest_for("checkpoint_effect_outcomes"),
        runtime_evidence_batch_digest: digest_for("runtime_evidence"),
        usage_facts_digest: digest_for("usage_facts"),
        session_output_refs_digest: digest_for("session_output_refs"),
    }
}

/// Exact APXM-owned descriptors used by the reference-host composition root.
/// The transport digest must hash these descriptors; it is never copied onto
/// an unrelated implementation.
#[derive(Clone, Debug)]
pub struct ReferenceRuntimeDescriptor {
    pub port_bindings: Vec<AdmittedPortBinding>,
    pub resource_ceilings: ResourceCeilings,
    pub confinement: AdmittedConfinement,
}

pub fn reference_host_runtime_descriptor() -> ReferenceRuntimeDescriptor {
    let port_bindings = [
        (
            PortSlot::ExecutionCommit,
            "apxm.execution-commit.v1",
            "b64becdf1c246b6a05bf02206dcf2306171501079257f188a7d76d88af9f26f7",
        ),
        (
            PortSlot::Confinement,
            "apxm.confinement.v1",
            "5d5e5c8e9e0a6d6e5f87eaed8e4c5ee3dfeef7fbd2501dc4f180785b9f5d7e0f",
        ),
        (
            PortSlot::ModelInference,
            "apxm.model-inference.v1",
            "fdf6aea657550b87f8b66f63f8e50cf793cec427b320c5bce940ba24fbc97362",
        ),
        (
            PortSlot::Capability,
            "apxm.capability-invocation.v1",
            "9369bd4c3506ba145d9424425efcb0493b288d418c582701d0856e8216fc8383",
        ),
        (
            PortSlot::ExternalAgentCapability,
            "apxm.external-agent.v1",
            "d7f7a1319eeac3c69af8355dd58b60a8f86d4861e5d32e3b8cac85105bd5eb5d",
        ),
        (
            PortSlot::DurableEvent,
            "apxm.durable-event.v1",
            "75d3c93c0c34f3c3d4dfeb89cec81cdbc208ac9106bba6955a923b0c566342d9",
        ),
        (
            PortSlot::ProgramComposition,
            "apxm.program-composition.v1",
            "7cacd4d1d1a0f17f30e167907997fbaa86e3e84026c69eee11d0673ceb67351a",
        ),
    ]
    .into_iter()
    .map(|(slot, schema_id, contract_digest)| AdmittedPortBinding {
        slot: slot.as_str().into(),
        port_contract_schema_id: schema_id.into(),
        port_contract_digest: format!("sha256:{contract_digest}"),
        binding_digest: digest_text(&format!("apxm.reference-host.binding.{}", slot.as_str())),
        proof_digest: digest_text(&format!("apxm.reference-host.proof.{}", slot.as_str())),
    })
    .collect();
    ReferenceRuntimeDescriptor {
        port_bindings,
        resource_ceilings: ResourceCeilings {
            max_wall_ms: 60_000,
            max_memory_bytes: 64 * 1024 * 1024,
            max_effect_bytes: 1024 * 1024,
        },
        confinement: AdmittedConfinement {
            confinement_type: "NATIVE-SANDBOX".into(),
            sandbox_digest: digest_text("apxm.reference-host.sandbox.v1"),
            policy_digest: digest_text("apxm.reference-host.policy.v1"),
        },
    }
}

pub fn reference_host_port_bindings_digest() -> String {
    digest_serializable(&reference_host_runtime_descriptor().port_bindings)
        .expect("reference host bindings are serializable")
}

pub fn reference_host_resource_ceiling_digest() -> String {
    digest_serializable(&reference_host_runtime_descriptor().resource_ceilings)
        .expect("reference host ceilings are serializable")
}

fn digest_text(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

struct DevModel;
impl ModelInferencePort for DevModel {
    fn attempt(&self, _request: &ModelCallRequest, _attempt: u32) -> AttemptDisposition {
        AttemptDisposition::Success(Usage {
            input_tokens: 1,
            output_tokens: 1,
        })
    }
}

struct DevConfinement;

#[async_trait]
impl ConfinementPort for DevConfinement {
    async fn attest(
        &self,
        request: ConfinementRequest,
    ) -> Result<ConfinementAttestation, ConfinementError> {
        Ok(ConfinementAttestation {
            attestation_id: format!("dev-attestation.{}", request.execution_id),
            host_id: request.host_id,
            execution_id: request.execution_id,
            confinement_type: request.confinement_type,
            sandbox_digest: request.sandbox_digest,
            policy_digest: request.policy_digest,
            attested_at: "dev-profile".into(),
            signature: "dev-profile-attestation".into(),
        })
    }
}

struct UnavailableModelRequestMetadata;

impl ModelCallRequestMetadataPort for UnavailableModelRequestMetadata {
    fn materialize(
        &self,
        _preparation: &ModelCallPreparation,
    ) -> Result<ModelCallRequestMetadata, TypedError> {
        Err(TypedError {
            category: apxm_inference::ErrorCategory::Configuration,
            code: "model_request_metadata_unavailable".into(),
            message: "canonical local execution has no admitted model request metadata source"
                .into(),
        })
    }
}

struct DevCapability;
#[async_trait]
impl CapabilityPort for DevCapability {
    async fn invoke(&self, _request: CapabilityRequest) -> CapabilityOutcome {
        CapabilityOutcome::Failed {
            message: "canonical local execution has no Invocation Admission authority source"
                .into(),
        }
    }
}

struct DevExternalAgent;
#[async_trait]
impl ExternalAgentCapabilityPort for DevExternalAgent {
    async fn prompt(&self, request: AcpPromptRequest) -> AcpPromptOutcome {
        AcpPromptOutcome {
            session_ref: request.session_ref,
            state: PromptEffectState::Completed {
                stop_reason: Some("dev_profile_completed".into()),
            },
            nested_events: vec![AttributedEvent {
                event_sequence: 0,
                kind: AttributedEventKind::Message,
                detail: Some("dev-profile external agent completed".into()),
                reverse_operation: None,
                reverse_target: None,
                reverse_decision: None,
            }],
            peer_usage: vec![PeerUsage {
                reported_by: "dev-profile".into(),
                metric_scope: "peer.usage.unavailable".into(),
                reported_value: "unavailable_not_observed".into(),
                availability_partial: None,
            }],
        }
    }
}

struct DevEvents;
#[async_trait]
impl EventPort for DevEvents {
    async fn await_event(&self, request: EventAwait) -> EventOutcome {
        EventOutcome::Fulfilled {
            payload: format!("event:{}", request.event_ref),
            event_ref: request.event_ref,
        }
    }
}

struct DevComposition;
#[async_trait]
impl CompositionPort for DevComposition {
    async fn program_new(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Created {
            child_instance_ref: format!("dev.child.{}", request.receiver.reference()),
        }
    }

    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome {
        // A stateful instance receiver resolves to the already-created child;
        // a program receiver is a one-shot invocation.
        let child = match &request.receiver {
            CompositionReceiver::Instance {
                program_instance_ref,
            } => program_instance_ref.clone(),
            CompositionReceiver::Program { program_ref } => format!("dev.child.{program_ref}"),
        };
        CompositionOutcome::Invoked {
            child_instance_ref: child,
        }
    }
}

#[derive(Default)]
struct DevCommit {
    version: Mutex<u64>,
}

#[async_trait]
impl ExecutionCommitPort for DevCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        let mut version = self.version.lock().expect("dev commit mutex poisoned");
        if *version != request.expected_program_state_version {
            return ExecutionCommitResult::CompareConflict {
                current_program_state_version: *version,
            };
        }
        *version += 1;
        ExecutionCommitResult::Committed {
            new_program_state_version: *version,
            evidence_position_ref: "dev.evidence.1".into(),
        }
    }

    async fn current_version(&self, _program_instance_ref: &ProgramInstanceRef) -> u64 {
        *self.version.lock().expect("dev commit mutex poisoned")
    }

    /// The in-process development commit holds no durable record, so it never
    /// has a committed continuation to hand back. Resumption requires a real
    /// admitted commit binding.
    async fn load_continuation(
        &self,
        _program_instance_ref: &ProgramInstanceRef,
    ) -> Option<serde_json::Value> {
        None
    }
}

#[cfg(test)]
const DEV_BINDING_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[cfg(test)]
fn dev_ports(
    commit: Arc<DevCommit>,
    model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
) -> Result<ExecutionPorts> {
    let contract = |schema_id: &str| SchemaDigestRef {
        schema_id: schema_id.into(),
        digest: DEV_BINDING_DIGEST.into(),
    };
    let binding = |slot, schema_id| ExactPortBinding {
        slot,
        port_contract: contract(schema_id),
        binding_digest: DEV_BINDING_DIGEST.into(),
        proof_digest: DEV_BINDING_DIGEST.into(),
    };
    let spec = PortBundleSpec::new(vec![
        (
            PortSlot::ExecutionCommit,
            contract("apxm.execution-commit.v1"),
        ),
        (
            PortSlot::ModelInference,
            contract("apxm.model-inference.v1"),
        ),
        (
            PortSlot::Capability,
            contract("apxm.capability-invocation.v1"),
        ),
        (
            PortSlot::ExternalAgentCapability,
            contract("apxm.external-agent.v1"),
        ),
    ]);
    let kernel_bundle = PortBundle::construct(
        &spec,
        vec![
            (
                binding(PortSlot::ExecutionCommit, "apxm.execution-commit.v1"),
                PortImplementation::ExecutionCommit(commit),
            ),
            (
                binding(PortSlot::ModelInference, "apxm.model-inference.v1"),
                PortImplementation::ModelInference(Arc::new(DevModel)),
            ),
            (
                binding(PortSlot::Capability, "apxm.capability-invocation.v1"),
                PortImplementation::Capability(Arc::new(DevCapability)),
            ),
            (
                binding(PortSlot::ExternalAgentCapability, "apxm.external-agent.v1"),
                PortImplementation::ExternalAgentCapability(Arc::new(DevExternalAgent)),
            ),
        ],
    )?;
    let bundle = ExecutionPortBundle::construct(
        Arc::new(kernel_bundle),
        contract("apxm.durable-event.v1"),
        binding(PortSlot::DurableEvent, "apxm.durable-event.v1"),
        Arc::new(DevEvents),
        contract("apxm.program-composition.v1"),
        binding(PortSlot::ProgramComposition, "apxm.program-composition.v1"),
        Arc::new(DevComposition),
    )?;
    Ok(ExecutionPorts::from_admitted_bundle(
        &bundle,
        model_call_request_metadata,
        Arc::new(NoopStaticHookHandler),
    )?)
}

async fn runtime_profile_from_invocation(
    commit: Arc<DevCommit>,
    model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
    verified: VerifiedInvocationAdmission,
    execution_id: &str,
) -> Result<RuntimeProfile> {
    let entries = verified
        .port_bindings
        .iter()
        .map(|binding| {
            let implementation = match binding.slot {
                PortSlot::ExecutionCommit => PortImplementation::ExecutionCommit(commit.clone()),
                PortSlot::Confinement => PortImplementation::Confinement(Arc::new(DevConfinement)),
                PortSlot::ModelInference => PortImplementation::ModelInference(Arc::new(DevModel)),
                PortSlot::Capability => PortImplementation::Capability(Arc::new(DevCapability)),
                PortSlot::ExternalAgentCapability => {
                    PortImplementation::ExternalAgentCapability(Arc::new(DevExternalAgent))
                }
                PortSlot::DurableEvent => PortImplementation::DurableEvent(Arc::new(DevEvents)),
                PortSlot::ProgramComposition => {
                    PortImplementation::ProgramComposition(Arc::new(DevComposition))
                }
            };
            (binding.clone(), implementation)
        })
        .collect();
    let runtime_admission =
        RuntimeAdmission::admit_invocation(verified, entries, "apxm-reference-host", execution_id)
            .await
            .map_err(|error| anyhow::anyhow!(error))?;
    RuntimeProfile::from_fully_admitted(
        runtime_admission,
        model_call_request_metadata,
        Arc::new(NoopStaticHookHandler),
    )
    .map_err(|error| anyhow::anyhow!(error))
}

fn node_outcome_json(outcome: &NodeOutcome) -> Value {
    match outcome {
        NodeOutcome::Model {
            node_id,
            outcome,
            result,
            replaced,
        } => json!({
            "node_id": node_id,
            "kind": "model.call",
            "outcome": model_outcome_json(outcome),
            "result": result,
            "replaced": replaced,
        }),
        NodeOutcome::Capability { node_id, outcome } => json!({
            "node_id": node_id,
            "kind": "capability.invoke",
            "outcome": capability_outcome_json(outcome),
        }),
        NodeOutcome::ExternalAgent { node_id, evidence } => json!({
            "node_id": node_id,
            "kind": "capability.invoke.external_agent",
            "outcome": "completed",
            "evidence": evidence,
        }),
        NodeOutcome::ProgramNew { node_id, outcome } => json!({
            "node_id": node_id,
            "kind": "program.new",
            "outcome": composition_outcome_json(outcome),
        }),
        NodeOutcome::ProgramInvoke { node_id, outcome } => json!({
            "node_id": node_id,
            "kind": "program.invoke",
            "outcome": composition_outcome_json(outcome),
        }),
        NodeOutcome::AwaitEvent { node_id, outcome } => json!({
            "node_id": node_id,
            "kind": "await.event",
            "outcome": event_outcome_json(outcome),
        }),
    }
}

fn model_outcome_json(outcome: &ModelOutcome) -> Value {
    match outcome {
        ModelOutcome::CommittedSuccess { usage } => json!({
            "status": "committed_success",
            "usage": usage,
        }),
        ModelOutcome::TypedFailure { error } => json!({
            "status": "typed_failure",
            "error": error,
        }),
        ModelOutcome::Cancelled => json!({"status": "cancelled"}),
        ModelOutcome::ModelOutcomeUnknown { uncertain_usage } => json!({
            "status": "model_outcome_unknown",
            "uncertain_usage": uncertain_usage,
        }),
    }
}

fn capability_outcome_json(outcome: &CapabilityOutcome) -> Value {
    match outcome {
        CapabilityOutcome::Completed { result } => {
            json!({"status": "completed", "result": result})
        }
        CapabilityOutcome::Failed { message } => json!({"status": "failed", "message": message}),
        CapabilityOutcome::OutcomeUnknown { message } => {
            json!({"status": "outcome_unknown", "message": message})
        }
    }
}

fn composition_outcome_json(outcome: &CompositionOutcome) -> Value {
    match outcome {
        CompositionOutcome::Created { child_instance_ref } => {
            json!({"status": "created", "child_instance_ref": child_instance_ref})
        }
        CompositionOutcome::Invoked { child_instance_ref } => {
            json!({"status": "invoked", "child_instance_ref": child_instance_ref})
        }
        CompositionOutcome::Failed { message } => json!({"status": "failed", "message": message}),
    }
}

fn event_outcome_json(outcome: &EventOutcome) -> Value {
    match outcome {
        EventOutcome::Fulfilled { event_ref, payload } => {
            json!({"status": "fulfilled", "event_ref": event_ref, "payload": payload})
        }
        EventOutcome::Parked => json!({"status": "parked"}),
        EventOutcome::Expired => json!({"status": "expired"}),
        EventOutcome::Cancelled => json!({"status": "cancelled"}),
        EventOutcome::Mismatched {
            delivered_event_ref,
        } => json!({
            "status": "mismatched",
            "delivered_event_ref": delivered_event_ref,
        }),
    }
}

fn commit_result_json(result: &ExecutionCommitResult) -> Value {
    match result {
        ExecutionCommitResult::Committed {
            new_program_state_version,
            evidence_position_ref,
        } => json!({
            "status": "committed",
            "new_program_state_version": new_program_state_version,
            "evidence_position_ref": evidence_position_ref,
        }),
        ExecutionCommitResult::CompareConflict {
            current_program_state_version,
        } => json!({
            "status": "compare_conflict",
            "current_program_state_version": current_program_state_version,
        }),
        ExecutionCommitResult::OutcomeUnknown { reconciliation_ref } => json!({
            "status": "outcome_unknown",
            "reconciliation_ref": reconciliation_ref,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_inference::ModelTargetRef;

    const TEST_RELEASE_BYTES: &[u8] = b"reference-release-v1";
    const TEST_PROVENANCE_BYTES: &[u8] = b"reference-provenance-v1";

    fn bytes_digest(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    fn invocation_admission(air: &AirModule, invocation_id: &str) -> InvocationAdmission {
        let artifact_bytes = serde_json::to_vec(air).expect("test AIR serialization");
        InvocationAdmission {
            schema_version: apxm_kernel::INVOCATION_ADMISSION_SCHEMA.into(),
            invocation_id: invocation_id.into(),
            artifact_digest: bytes_digest(&artifact_bytes),
            release_digest: bytes_digest(TEST_RELEASE_BYTES),
            port_bindings_digest: reference_host_port_bindings_digest(),
            resource_ceiling_digest: reference_host_resource_ceiling_digest(),
            provenance_digest: bytes_digest(TEST_PROVENANCE_BYTES),
        }
    }

    async fn admitted_profile(
        air: &AirModule,
        invocation_id: &str,
        commit: Arc<DevCommit>,
    ) -> RuntimeProfile {
        let descriptor = reference_host_runtime_descriptor();
        let admission = invocation_admission(air, invocation_id);
        let artifact_bytes = serde_json::to_vec(air).expect("test AIR serialization");
        let verified = verify_invocation_admission(
            &admission,
            &artifact_bytes,
            TEST_RELEASE_BYTES,
            TEST_PROVENANCE_BYTES,
            &descriptor.port_bindings,
            descriptor.resource_ceilings,
            &descriptor.confinement,
        )
        .expect("test invocation admission");
        runtime_profile_from_invocation(
            commit,
            Arc::new(UnavailableModelRequestMetadata),
            verified,
            "test.execution",
        )
        .await
        .expect("fully admitted profile")
    }

    fn empty_profile_air() -> AirModule {
        serde_json::from_value(json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("empty profile AIR")
    }

    fn profile_request(air: AirModule, suffix: &str) -> ExecutionRequest {
        ExecutionRequest {
            model_admission: model_admission(&air, DEV_BINDING_DIGEST),
            air,
            hook_bindings: Vec::new(),
            capability_invocations: BTreeMap::new(),
            program_instance_ref: ProgramInstanceRef::new(format!("profile.instance.{suffix}")),
            program_invocation_ref: ProgramInvocationRef::new(format!(
                "profile.invocation.{suffix}"
            )),
            commit_id: format!("profile.commit.{suffix}"),
            write_set: dev_write_set(),
        }
    }

    #[tokio::test]
    async fn canonical_runtime_requires_transport_invocation_admission() {
        let runtime = CanonicalRuntime::new();
        let air = empty_profile_air();
        let mut admission = invocation_admission(&air, "invocation.reference-host.unauthorized");
        admission.artifact_digest = digest('f');
        let output = runtime
            .execute(air, &admission, TEST_RELEASE_BYTES, TEST_PROVENANCE_BYTES)
            .await;
        assert!(output.is_err(), "tampered admission must fail closed");
    }

    #[tokio::test]
    async fn canonical_runtime_rejects_a_tampered_but_valid_air_module() {
        let runtime = CanonicalRuntime::new();
        let original = empty_profile_air();
        let admission = invocation_admission(&original, "invocation.reference-host.air-drift");
        let mut tampered_json = serde_json::to_value(&original).expect("AIR value");
        tampered_json["source_map"]["source_language"] = Value::String("typescript".into());
        let tampered: AirModule =
            serde_json::from_value(tampered_json).expect("tampered AIR remains well-formed");
        assert!(tampered.verify().is_accepted(), "tampered AIR remains valid");

        let error = runtime
            .execute(
                tampered,
                &admission,
                TEST_RELEASE_BYTES,
                TEST_PROVENANCE_BYTES,
            )
            .await
            .expect_err("valid AIR with different bytes must fail closed");
        assert!(error.to_string().contains("artifact digest mismatch"));
    }

    #[tokio::test]
    async fn canonical_execution_enters_through_the_verified_invocation_admission() {
        let runtime = CanonicalRuntime::new();
        let air = empty_profile_air();
        let admission = invocation_admission(&air, "invocation.reference-host.1");
        let output = runtime
            .execute(air, &admission, TEST_RELEASE_BYTES, TEST_PROVENANCE_BYTES)
            .await
            .expect("verified transport authority reaches canonical runtime");
        assert_eq!(output["runtime"], "apxm_execution");
        assert_eq!(output["status"], "completed");
    }

    #[tokio::test]
    async fn invocation_admission_is_carried_into_atomic_runtime_commit() {
        let runtime = CanonicalRuntime::new();
        let air = empty_profile_air();
        let admission = invocation_admission(&air, "invocation.reference-host.commit");
        let output = runtime
            .execute(air, &admission, TEST_RELEASE_BYTES, TEST_PROVENANCE_BYTES)
            .await
            .expect("exact host admission reaches canonical runtime");
        assert_eq!(output["status"], "completed");
        assert_eq!(output["commit"]["status"], "committed");
    }

    #[tokio::test]
    async fn invocation_admission_provenance_drift_fails_before_commit() {
        let runtime = CanonicalRuntime::new();
        let air = empty_profile_air();
        let admission = invocation_admission(&air, "invocation.reference-host.negative");
        let error = runtime
            .execute(air, &admission, TEST_RELEASE_BYTES, b"tampered-provenance")
            .await
            .expect_err("provenance drift must fail closed");
        assert!(error.to_string().contains("provenance digest mismatch"));
    }

    #[tokio::test]
    async fn profile_shutdown_rejects_new_work_fail_closed() {
        let air = empty_profile_air();
        let profile = admitted_profile(
            &air,
            "invocation.profile.closed",
            Arc::new(DevCommit::default()),
        )
        .await;
        profile.shutdown();

        let error = profile
            .execute(profile_request(empty_profile_air(), "closed"), Value::Null)
            .await
            .expect_err("closed profile must reject new work");
        assert!(matches!(error, RuntimeProfileError::Closed));
    }

    #[test]
    fn profile_admission_rejects_missing_required_slot() {
        let air = empty_profile_air();
        let descriptor = reference_host_runtime_descriptor();
        let bindings = descriptor
            .port_bindings
            .into_iter()
            .filter(|binding| binding.slot != PortSlot::Confinement.as_str())
            .collect::<Vec<_>>();
        let admission = invocation_admission(&air, "invocation.profile.no-confinement");
        let artifact_bytes = serde_json::to_vec(&air).expect("test AIR serialization");
        let error = verify_invocation_admission(
            &admission,
            &artifact_bytes,
            TEST_RELEASE_BYTES,
            TEST_PROVENANCE_BYTES,
            &bindings,
            reference_host_runtime_descriptor().resource_ceilings,
            &reference_host_runtime_descriptor().confinement,
        )
        .expect_err("missing confinement binding must fail closed");
        assert!(error.to_string().contains("port binding digest mismatch"));
    }

    #[tokio::test]
    async fn profile_recovery_is_instance_local_and_new_profile_can_run() {
        let air = empty_profile_air();
        let stopped = admitted_profile(
            &air,
            "invocation.profile.stopped",
            Arc::new(DevCommit::default()),
        )
        .await;
        stopped.shutdown();

        let recovered = admitted_profile(
            &air,
            "invocation.profile.recovered",
            Arc::new(DevCommit::default()),
        )
        .await;
        let report = recovered
            .execute(
                profile_request(empty_profile_air(), "recovered"),
                Value::Null,
            )
            .await
            .expect("new profile accepts work after recovery");
        assert!(matches!(
            report.commit,
            ExecutionCommitResult::Committed { .. }
        ));
        assert!(!stopped.is_accepting());
        assert!(recovered.is_accepting());
    }

    struct TestModelRequestMetadata;

    impl ModelCallRequestMetadataPort for TestModelRequestMetadata {
        fn materialize(
            &self,
            preparation: &ModelCallPreparation,
        ) -> Result<ModelCallRequestMetadata, TypedError> {
            Ok(ModelCallRequestMetadata {
                model_context_envelope_ref: apxm_inference::ModelContextEnvelopeRef {
                    context_id: format!(
                        "test.context.{}",
                        preparation.node_execution_id().as_str()
                    ),
                    sealed_digest: digest('e'),
                },
                idempotency: apxm_inference::IdempotencyKey {
                    key_id: format!("test.idempotency.{}", preparation.effect_id()),
                    scope_ref: "test.idempotency.scope".into(),
                },
                stream_mode: apxm_inference::ModelStreamMode::Buffered,
            })
        }
    }

    #[tokio::test]
    async fn executes_canonical_air_with_dev_ports() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {"node_id": "n.model", "op": "model.call", "parent_region_id": "r.root", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target.v1", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.output", "type_ref": "ModelOutput"}},
                {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.root", "execution_order": 1, "operands": [{"slot": "program_ref", "value_id": "child", "type_ref": "ProgramRef"}], "result": {"value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}},
                {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.root", "execution_order": 2, "operands": [{"slot": "receiver", "value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}, {"slot": "input", "value_id": "value.program.input", "type_ref": "ProgramInput"}], "result": {"value_id": "value.program.output", "type_ref": "ProgramOutput"}},
                {"node_id": "n.await", "op": "await.event", "parent_region_id": "r.root", "execution_order": 3, "operands": [{"slot": "event_ref", "value_id": "ready", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}}
            ],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("canonical air");
        assert!(air.verify().is_accepted());

        let commit = Arc::new(DevCommit::default());
        let ports = dev_ports(commit, Arc::new(TestModelRequestMetadata))
            .expect("development ports form an admitted bundle");
        let model_admission = model_admission(&air, DEV_BINDING_DIGEST);
        let report = execute(
            &ports,
            ExecutionRequest {
                air,
                hook_bindings: Vec::new(),
                model_admission,
                capability_invocations: BTreeMap::new(),
                program_instance_ref: ProgramInstanceRef::new("test.instance"),
                program_invocation_ref: ProgramInvocationRef::new("test.invocation"),
                commit_id: "test.commit".into(),
                write_set: dev_write_set(),
            },
            Value::Null,
        )
        .await
        .expect("canonical execution");

        assert_eq!(report.node_outcomes.len(), 4);
        assert!(matches!(
            report.commit,
            ExecutionCommitResult::Committed { .. }
        ));
    }

    #[tokio::test]
    async fn executes_canonical_air_without_a_model_binding() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.root", "execution_order": 0, "operands": [{"slot": "program_ref", "value_id": "child", "type_ref": "ProgramRef"}], "result": {"value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}},
                {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.root", "execution_order": 1, "operands": [{"slot": "receiver", "value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}, {"slot": "input", "value_id": "value.program.input", "type_ref": "ProgramInput"}], "result": {"value_id": "value.program.output", "type_ref": "ProgramOutput"}},
                {"node_id": "n.await", "op": "await.event", "parent_region_id": "r.root", "execution_order": 2, "operands": [{"slot": "event_ref", "value_id": "ready", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}}
            ],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("canonical air without a model call");
        assert!(air.verify().is_accepted());

        let model_admission = model_admission(&air, DEV_BINDING_DIGEST);
        assert!(matches!(
            model_admission.validate(&ModelTargetRef("model.absent".into())),
            Err(apxm_inference::BindingError::MissingTarget(_))
        ));

        let commit = Arc::new(DevCommit::default());
        let ports = dev_ports(commit, Arc::new(UnavailableModelRequestMetadata))
            .expect("development ports form an admitted bundle");
        let report = execute(
            &ports,
            ExecutionRequest {
                model_admission,
                air,
                hook_bindings: Vec::new(),
                capability_invocations: BTreeMap::new(),
                program_instance_ref: ProgramInstanceRef::new("test.instance.no-model"),
                program_invocation_ref: ProgramInvocationRef::new("test.invocation.no-model"),
                commit_id: "test.commit.no-model".into(),
                write_set: dev_write_set(),
            },
            Value::Null,
        )
        .await
        .expect("canonical execution without a model binding");

        assert_eq!(report.node_outcomes.len(), 3);
        assert!(matches!(
            report.commit,
            ExecutionCommitResult::Committed { .. }
        ));
    }

    #[test]
    fn rejects_local_capability_without_invocation_admission_authority() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {"node_id": "n.cap", "op": "capability.invoke", "parent_region_id": "r.root", "execution_order": 0, "operands": [{"slot": "capability_ref", "value_id": "cap.search", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.capability.arguments", "type_ref": "CapabilityArguments"}], "result": {"value_id": "value.capability.output", "type_ref": "CapabilityOutput"}}
            ],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("canonical Capability AIR");

        let error = ensure_local_capability_authority_available(&air)
            .expect_err("local execution has no authority source");
        assert!(
            error
                .to_string()
                .contains("explicit Invocation Admission authority is required")
        );
    }

    #[test]
    fn admits_each_distinct_authored_model_target_once() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {"node_id": "n.model.first", "op": "model.call", "parent_region_id": "r.root", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target.first", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.request.first", "type_ref": "ModelRequest"}], "result": {"value_id": "value.output.first", "type_ref": "ModelOutput"}},
                {"node_id": "n.model.second", "op": "model.call", "parent_region_id": "r.root", "execution_order": 1, "operands": [{"slot": "model_ref", "value_id": "model.target.second", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.request.second", "type_ref": "ModelRequest"}], "result": {"value_id": "value.output.second", "type_ref": "ModelOutput"}},
                {"node_id": "n.model.first.again", "op": "model.call", "parent_region_id": "r.root", "execution_order": 2, "operands": [{"slot": "model_ref", "value_id": "model.target.first", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.request.third", "type_ref": "ModelRequest"}], "result": {"value_id": "value.output.third", "type_ref": "ModelOutput"}}
            ],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("canonical air with distinct model targets");
        assert!(air.verify().is_accepted());

        let admission = model_admission(&air, DEV_BINDING_DIGEST);
        for target in ["model.target.first", "model.target.second"] {
            let resolved = admission
                .validate(&ModelTargetRef(target.into()))
                .expect("authored target has exactly one local binding");
            assert_eq!(resolved.model_target.reference.0, target);
        }
    }
}
