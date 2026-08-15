//! Runtime Service composition: admitted ports plus artifact execution.
//!
//! This is the product handler body. It does not relocate `CanonicalRuntime`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::ports::capability::LocalCapabilityPort;
use crate::ports::model::{LocalModelInferencePort, LocalModelRequestMetadata};

use apxm_ais::permissions::{LayerDecisions, PermissionDecision, PermissionResolution};
use apxm_execution::{
    CapabilityGrantSet, CapabilityInvocationAdmission, CapturedHookBodyHandler, CompositionOutcome,
    CompositionPort, CompositionReceiver, CompositionRequest, EventAwait, EventOutcome, EventPort,
    ExecutionRequest, NodeOutcome, RuntimeProfile,
};
use apxm_inference::{
    InferenceTargetCommitment, ModelBindingAdmission, ModelCallRequestMetadataPort, ModelOutcome,
    ResolvedModelBinding,
};
use apxm_kernel::{
    AcpPromptOutcome, AcpPromptRequest, AdmittedCapabilityPermission, AdmittedConfinement,
    AdmittedPortBinding, AtomicWriteSet, CapabilityOutcome, ConfinementAttestation,
    ConfinementError, ConfinementPort, ConfinementRequest, ExecutionCommitPort,
    ExecutionCommitRequest, ExecutionCommitResult, ExternalAgentCapabilityPort,
    INVOCATION_ADMISSION_SCHEMA, InvocationAdmission, InvocationAdmissionClaim, PortImplementation,
    PortSlot, ProgramInstanceRef, ProgramInvocationRef, PromptEffectState, ResourceCeilings,
    RuntimeAdmission, VerifiedInvocationAdmission, admitted_capability_permissions,
    digest_serializable, verify_invocation_admission,
};
use apxm_program::CapabilityInvocationAuthority;
use apxm_program::air::{AirModule, SemanticOpKind};
use apxm_program::external_agent::{AttributedEvent, AttributedEventKind, PeerUsage};
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const LOCAL_ACTING_PRINCIPAL_REF: &str = "apxm.canonical.local.acting-principal";
const LOCAL_AGENT_IDENTITY_REF: &str = "apxm.canonical.local.agent-identity";
const LOCAL_CAPABILITY_GRANT_PREFIX: &str = "apxm.canonical.local.grant.";

/// Exact APXM-owned descriptors used by the service composition root.
#[derive(Clone, Debug)]
pub struct CanonicalRuntimeDescriptor {
    pub port_bindings: Vec<AdmittedPortBinding>,
    pub resource_ceilings: ResourceCeilings,
    pub confinement: AdmittedConfinement,
}

/// Digest-bound artifact bytes the Runtime Service may instantiate.
#[derive(Default)]
pub struct ArtifactStore {
    committed: BTreeMap<String, Vec<u8>>,
}

impl ArtifactStore {
    /// Commit exact artifact bytes under their content digest.
    pub fn commit(&mut self, bytes: Vec<u8>) -> String {
        let digest = artifact_digest(&bytes);
        self.committed.insert(digest.clone(), bytes);
        digest
    }

    /// Commit bytes under a caller-supplied digest. Used when the admission
    /// already named the digest.
    pub fn commit_named(&mut self, digest: String, bytes: Vec<u8>) {
        self.committed.insert(digest, bytes);
    }

    #[must_use]
    pub fn get(&self, digest: &str) -> Option<&[u8]> {
        self.committed.get(digest).map(Vec::as_slice)
    }
}

/// SHA-256 digest in the admission wire form.
#[must_use]
pub fn artifact_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Exact reference runtime descriptor. The transport digest must hash these
/// descriptors; it is never copied onto an unrelated implementation.
#[must_use]
pub fn canonical_runtime_descriptor() -> CanonicalRuntimeDescriptor {
    let port_bindings = [
        (
            PortSlot::ExecutionCommit,
            "apxm.execution-commit",
            "b64becdf1c246b6a05bf02206dcf2306171501079257f188a7d76d88af9f26f7",
        ),
        (
            PortSlot::Confinement,
            "apxm.confinement",
            "5d5e5c8e9e0a6d6e5f87eaed8e4c5ee3dfeef7fbd2501dc4f180785b9f5d7e0f",
        ),
        (
            PortSlot::ModelInference,
            "apxm.model-inference",
            "fdf6aea657550b87f8b66f63f8e50cf793cec427b320c5bce940ba24fbc97362",
        ),
        (
            PortSlot::Capability,
            "apxm.capability-invocation",
            "9369bd4c3506ba145d9424425efcb0493b288d418c582701d0856e8216fc8383",
        ),
        (
            PortSlot::ExternalAgentCapability,
            "apxm.external-agent",
            "d7f7a1319eeac3c69af8355dd58b60a8f86d4861e5d32e3b8cac85105bd5eb5d",
        ),
        (
            PortSlot::DurableEvent,
            "apxm.durable-event",
            "75d3c93c0c34f3c3d4dfeb89cec81cdbc208ac9106bba6955a923b0c566342d9",
        ),
        (
            PortSlot::ProgramComposition,
            "apxm.program-composition",
            "7cacd4d1d1a0f17f30e167907997fbaa86e3e84026c69eee11d0673ceb67351a",
        ),
    ]
    .into_iter()
    .map(|(slot, schema_id, contract_digest)| AdmittedPortBinding {
        slot: slot.as_str().into(),
        port_contract_schema_id: schema_id.into(),
        port_contract_digest: format!("sha256:{contract_digest}"),
        binding_digest: digest_text(&format!("apxm.canonical.binding.{}", slot.as_str())),
        proof_digest: digest_text(&format!("apxm.canonical.proof.{}", slot.as_str())),
    })
    .collect();
    CanonicalRuntimeDescriptor {
        port_bindings,
        resource_ceilings: ResourceCeilings {
            max_wall_ms: 60_000,
            max_memory_bytes: 64 * 1024 * 1024,
            max_effect_bytes: 1024 * 1024,
        },
        confinement: AdmittedConfinement {
            confinement_type: "NATIVE-SANDBOX".into(),
            sandbox_digest: digest_text("apxm.canonical.sandbox.v1"),
            policy_digest: digest_text("apxm.canonical.policy.v1"),
        },
    }
}

#[must_use]
pub fn canonical_port_bindings_digest() -> String {
    digest_serializable(&canonical_runtime_descriptor().port_bindings)
        .expect("canonical bindings are serializable")
}

#[must_use]
pub fn canonical_resource_ceiling_digest() -> String {
    digest_serializable(&canonical_runtime_descriptor().resource_ceilings)
        .expect("canonical ceilings are serializable")
}

fn digest_text(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
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
    continuation: Mutex<Option<Value>>,
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

    async fn load_continuation(&self, _program_instance_ref: &ProgramInstanceRef) -> Option<Value> {
        self.continuation
            .lock()
            .expect("dev commit continuation mutex poisoned")
            .clone()
    }
}

/// Materials required to admit one invocation of a committed artifact.
pub struct InvocationMaterials {
    pub admission: InvocationAdmission,
    pub release_bytes: Vec<u8>,
    pub provenance_bytes: Vec<u8>,
}

/// Package-handler implementations supplied to one Runtime Service instance.
#[derive(Debug, Clone)]
pub struct AdmittedPackageHandlers {
    /// Private handler-worker command for each language the manifest uses.
    pub workers:
        std::collections::BTreeMap<apxm_core::types::HandlerLanguage, PackageHandlerWorkerCommand>,
    /// Validated manifest those workers may evaluate.
    pub manifest: apxm_core::types::HandlerManifest,
}

/// How one language's private worker is started.
#[derive(Debug, Clone)]
pub struct PackageHandlerWorkerCommand {
    /// Interpreter that runs the worker entry.
    pub interpreter: String,
    /// Private worker entry path.
    pub entry: PathBuf,
}

/// Execute one admitted AIR artifact through the shared runtime profile.
pub async fn execute_admitted_artifact(
    air: AirModule,
    artifact_bytes: &[u8],
    materials: &InvocationMaterials,
    handlers: Option<&AdmittedPackageHandlers>,
    package_root: Option<&Path>,
) -> Result<Value, String> {
    let admission = &materials.admission;
    let descriptor = canonical_runtime_descriptor();
    if admission.port_bindings_digest != canonical_port_bindings_digest()
        || admission.resource_ceiling_digest != canonical_resource_ceiling_digest()
    {
        return Err("admission_profile_mismatch".to_owned());
    }
    let verified = verify_invocation_admission(
        admission,
        InvocationAdmissionClaim {
            artifact_bytes,
            release_bytes: &materials.release_bytes,
            provenance_bytes: &materials.provenance_bytes,
            artifact_semantic_requirements: &apxm_program::air_semantic_requirements(&air),
            admitted_port_bindings: &descriptor.port_bindings,
            resource_ceilings: &descriptor.resource_ceilings,
            confinement: &descriptor.confinement,
        },
    )
    .map_err(|error| error.to_string())?;
    let capability = Arc::new(
        LocalCapabilityPort::with_package_root(handlers, package_root)
            .map_err(|error| error.to_string())?,
    );
    let capability_permissions =
        local_capability_permissions(&air, &capability.admitted_names(), package_root)?;
    let capability_invocations = local_capability_invocation_admissions(
        &air,
        &CapabilityGrantSet::from_registered_implementations(capability.registered_names()),
        &capability_permissions,
    )?;
    let model_binding_digest = descriptor
        .port_bindings
        .iter()
        .find(|binding| binding.slot == PortSlot::ModelInference.as_str())
        .map(|binding| binding.binding_digest.clone())
        .ok_or_else(|| "reference runtime model binding is absent".to_owned())?;
    let hook_bindings = apxm_program::air_hook_bindings(&air);
    let request = ExecutionRequest {
        model_admission: model_admission(&air, &model_binding_digest),
        initial_values: initial_model_request_values(&air),
        air,
        hook_bindings,
        capability_invocations,
        program_instance_ref: ProgramInstanceRef::new("canonical.instance"),
        program_invocation_ref: ProgramInvocationRef::new(admission.invocation_id.clone()),
        commit_id: format!("canonical.commit.{}", admission.invocation_id),
        write_set: reference_write_set(&admission.invocation_id),
    };
    let commit = Arc::new(DevCommit::default());
    let model = Arc::new(
        LocalModelInferencePort::from_backend_roster().map_err(|error| error.to_string())?,
    );
    let profile = runtime_profile_from_invocation(
        commit,
        capability,
        model.clone(),
        Arc::new(LocalModelRequestMetadata),
        verified,
        "runtime.service.execution",
    )
    .await?;
    let report = profile
        .execute(request, Value::Null)
        .await
        .map_err(|error| error.to_string())?;
    Ok(json!({
        "schema_version": "apxm.local-execute-result",
        "runtime": "apxm_execution",
        "status": "completed",
        "content": report.final_context,
        "results": {
            "node_outcomes": report.node_outcomes.iter().map(node_outcome_json).collect::<Vec<_>>(),
            "external_agent_evidence": report.external_agent_evidence,
            "model_attempt_diagnostics": model.attempt_diagnostics(),
        },
        "stats": {
            "executed_nodes": report.node_outcomes.len(),
            "failed_nodes": failed_node_count(&report.node_outcomes),
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

/// Build admission materials that match the reference profile for these bytes.
pub fn materials_for_artifact(
    artifact_bytes: &[u8],
    invocation_id: impl Into<String>,
    release_bytes: Vec<u8>,
    provenance_bytes: Vec<u8>,
) -> InvocationMaterials {
    let admission = InvocationAdmission {
        schema_version: INVOCATION_ADMISSION_SCHEMA.to_owned(),
        invocation_id: invocation_id.into(),
        artifact_digest: artifact_digest(artifact_bytes),
        release_digest: artifact_digest(&release_bytes),
        port_bindings_digest: canonical_port_bindings_digest(),
        resource_ceiling_digest: canonical_resource_ceiling_digest(),
        provenance_digest: artifact_digest(&provenance_bytes),
    };
    InvocationMaterials {
        admission,
        release_bytes,
        provenance_bytes,
    }
}

async fn runtime_profile_from_invocation(
    commit: Arc<DevCommit>,
    capability: Arc<LocalCapabilityPort>,
    model: Arc<LocalModelInferencePort>,
    model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
    verified: VerifiedInvocationAdmission,
    execution_id: &str,
) -> Result<RuntimeProfile, String> {
    let entries = verified
        .port_bindings
        .iter()
        .map(|binding| {
            let implementation = match binding.slot {
                PortSlot::ExecutionCommit => PortImplementation::ExecutionCommit(commit.clone()),
                PortSlot::Confinement => PortImplementation::Confinement(Arc::new(DevConfinement)),
                PortSlot::ModelInference => PortImplementation::ModelInference(model.clone()),
                PortSlot::Capability => PortImplementation::Capability(capability.clone()),
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
        RuntimeAdmission::admit_invocation(verified, entries, "apxm-runtime-service", execution_id)
            .await
            .map_err(|error| error.to_string())?;
    RuntimeProfile::from_fully_admitted(
        runtime_admission,
        model_call_request_metadata,
        Arc::new(CapturedHookBodyHandler),
    )
    .map_err(|error| error.to_string())
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

fn initial_model_request_values(air: &AirModule) -> BTreeMap<String, Value> {
    air.semantic_operations
        .iter()
        .filter(|operation| operation.op == SemanticOpKind::ModelCall)
        .filter_map(|operation| {
            let value_id = operation
                .operands
                .iter()
                .find(|operand| operand.slot == "request")?
                .value_id
                .clone();
            (!air
                .value_assemblies
                .iter()
                .any(|assembly| assembly.value_id == value_id))
            .then(|| (value_id.clone(), json!({"value_id": value_id})))
        })
        .collect()
}

fn local_capability_invocation_admissions(
    air: &AirModule,
    grants: &CapabilityGrantSet,
    permissions: &[AdmittedCapabilityPermission],
) -> Result<BTreeMap<String, CapabilityInvocationAdmission>, String> {
    let mut admissions = BTreeMap::new();
    for operation in &air.semantic_operations {
        if operation.op != SemanticOpKind::CapabilityInvoke {
            continue;
        }
        let Some(capability_ref) = operation
            .operands
            .iter()
            .find(|operand| operand.slot == apxm_ais::SLOT_CAPABILITY_REF)
            .map(|operand| operand.value_id.clone())
        else {
            continue;
        };
        let authority = CapabilityInvocationAuthority::new(
            LOCAL_ACTING_PRINCIPAL_REF,
            LOCAL_AGENT_IDENTITY_REF,
            format!("{LOCAL_CAPABILITY_GRANT_PREFIX}{capability_ref}"),
            Vec::new(),
        )
        .map_err(|error| error.to_string())?;
        let permission = permissions
            .iter()
            .find(|entry| entry.capability_ref == capability_ref)
            .map(|entry| entry.permission.clone())
            .ok_or_else(|| format!("no permission for {capability_ref}"))?;
        let admission = grants
            .admit(&capability_ref, authority, permission)
            .map_err(|error| error.to_string())?;
        admissions.insert(operation.node_id.clone(), admission);
    }
    Ok(admissions)
}

fn local_capability_permissions(
    air: &AirModule,
    admitted: &BTreeSet<String>,
    package_root: Option<&Path>,
) -> Result<Vec<AdmittedCapabilityPermission>, String> {
    let mut requested = LayerDecisions::new();
    for capability_ref in air.invoked_capability_refs() {
        let authored = air
            .capability_permission_requests
            .get(capability_ref)
            .cloned()
            .unwrap_or_else(PermissionDecision::allow);
        requested.insert(capability_ref.to_string(), authored);
    }

    let (package_layer, deployment_layer) = if let Some(package_root) = package_root {
        let package_decisions = canonical_package_permission_decisions(package_root, &requested)?;
        let package_layer = requested
            .iter()
            .filter_map(|(capability_ref, authored)| {
                let resolved = package_decisions.get(capability_ref)?;
                (resolved != authored).then(|| (capability_ref.clone(), resolved.clone()))
            })
            .collect();
        (package_layer, denied_unadmitted_capabilities(air, admitted))
    } else {
        (
            denied_unadmitted_capabilities(air, admitted),
            LayerDecisions::new(),
        )
    };

    let layers = if package_root.is_some() {
        BTreeMap::from([
            (apxm_ais::permissions::PermissionLayer::Code, requested),
            (
                apxm_ais::permissions::PermissionLayer::Package,
                package_layer,
            ),
            (
                apxm_ais::permissions::PermissionLayer::Deployment,
                deployment_layer,
            ),
        ])
    } else {
        BTreeMap::from([
            (apxm_ais::permissions::PermissionLayer::Code, requested),
            (
                apxm_ais::permissions::PermissionLayer::Package,
                package_layer,
            ),
        ])
    };
    let resolution = PermissionResolution::resolve(&layers).map_err(|error| error.to_string())?;
    Ok(admitted_capability_permissions(&resolution))
}

/// Validate the code-to-package tightening rule before a Runtime Service asks
/// an approval broker to resolve authored `Ask` decisions. A package that tries
/// to widen an authored request must fail at admission; otherwise the broker
/// could return `ask_denied` first and hide the invalid package layer.
pub fn validate_package_permission_resolution(
    air: &AirModule,
    package_root: Option<&Path>,
) -> Result<(), String> {
    let Some(package_root) = package_root else {
        return Ok(());
    };
    let requested = air
        .invoked_capability_refs()
        .into_iter()
        .map(|capability_ref| {
            let decision = air
                .capability_permission_requests
                .get(capability_ref)
                .cloned()
                .unwrap_or_else(PermissionDecision::allow);
            (capability_ref.to_string(), decision)
        })
        .collect();
    canonical_package_permission_decisions(package_root, &requested).map(|_| ())
}

fn denied_unadmitted_capabilities(air: &AirModule, admitted: &BTreeSet<String>) -> LayerDecisions {
    air.invoked_capability_refs()
        .into_iter()
        .filter(|capability_ref| !admitted.contains(*capability_ref))
        .map(|capability_ref| {
            (
                capability_ref.to_string(),
                PermissionDecision::deny(format!(
                    "canonical local execution binds no sandbox backend and no issued Capability \
                     grant, so it admits only the read-only capability surface [{}]",
                    admitted
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            )
        })
        .collect()
}

#[derive(Debug, serde::Deserialize)]
struct CanonicalPackageManifest {
    #[serde(default)]
    permissions: LayerDecisions,
}

fn canonical_package_permission_decisions(
    package_root: &Path,
    authored: &LayerDecisions,
) -> Result<BTreeMap<String, PermissionDecision>, String> {
    let manifest_path = package_root.join("agent.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;
    let manifest: CanonicalPackageManifest = toml::from_str(&manifest)
        .map_err(|error| format!("failed to parse {}: {error}", manifest_path.display()))?;

    let mut grantable = apxm_ais::capabilities::BUILTINS
        .iter()
        .map(|id| (*id).to_string())
        .collect::<BTreeSet<_>>();
    let capabilities_dir = package_root.join("capabilities");
    if capabilities_dir.is_dir() {
        for entry in std::fs::read_dir(&capabilities_dir)
            .map_err(|error| format!("failed to read {}: {error}", capabilities_dir.display()))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            if !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                continue;
            }
            let path = entry.path();
            if path.join("handler.py").is_file() || path.join("handler.ts").is_file() {
                grantable.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }

    let requested_for_package = grantable
        .into_iter()
        .map(|capability_ref| {
            let decision = authored
                .get(&capability_ref)
                .cloned()
                .unwrap_or_else(PermissionDecision::allow);
            (capability_ref, decision)
        })
        .collect();
    let resolution = PermissionResolution::resolve_code_over_package(
        requested_for_package,
        manifest.permissions,
    )
    .map_err(|error| error.to_string())?;
    Ok(resolution
        .iter()
        .map(|(capability_ref, resolved)| (capability_ref.to_string(), resolved.decision.clone()))
        .collect())
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
                        "canonical.model",
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

fn failed_node_count(outcomes: &[NodeOutcome]) -> usize {
    outcomes
        .iter()
        .filter(|outcome| match outcome {
            NodeOutcome::Model { outcome, .. } => {
                matches!(outcome, ModelOutcome::TypedFailure { .. })
            }
            NodeOutcome::Capability { outcome, .. } => {
                matches!(outcome, CapabilityOutcome::Failed { .. })
            }
            NodeOutcome::ProgramNew { outcome, .. }
            | NodeOutcome::ProgramInvoke { outcome, .. } => {
                matches!(outcome, CompositionOutcome::Failed { .. })
            }
            NodeOutcome::AwaitEvent { outcome, .. } => matches!(
                outcome,
                EventOutcome::Expired | EventOutcome::Mismatched { .. }
            ),
            NodeOutcome::ExternalAgent { .. } => false,
        })
        .count()
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
        NodeOutcome::Capability {
            node_id,
            outcome,
            replaced,
        } => json!({
            "node_id": node_id,
            "kind": "capability.invoke",
            "outcome": capability_outcome_json(outcome),
            "replaced": replaced,
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
