//! G3 acceptance suite for P-006–P-009: product-neutral Execution Admission,
//! exact binding, confinement fail-closed, checkpoint/resume/cancel/revoke,
//! atomic commit, outcome_unknown without blind replay, and one checkpoint
//! advancer with no duplicate external effect.
//!
//! Suites: positive, negative, failure, revocation, recovery, boundary,
//! crash, replay, cancellation, confinement-escape, ambiguous-binding,
//! lost-reply.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;

use apxm_program::artifact::SchemaDigestRef;
use apxm_program::runtime_evidence::{
    LayerDecisions, PermissionDecision, PermissionLayer, PermissionResolution, ProgramIdentity,
    ResolvedPermission, RuntimeEvidenceVersion,
};

use apxm_kernel::{
    AdmissionError, AdmittedCapabilityPermission, AdmittedModelTarget, AdmittedPortBinding,
    AtomicWriteSet, CapabilityOutcome, CapabilityPort, CapabilityRequest, CheckpointAdvancer,
    ConfinementAttestation, ConfinementError, ConfinementPort, ConfinementRequest, EffectRecord,
    EffectState, EffectTransition, ExactPortBinding, ExecutionAdmission, ExecutionCommitPort,
    ExecutionCommitRequest, ExecutionCommitResult, ExecutionCommitTuple, InstanceError, Invocation,
    IssuerKeyring, IssuerSigningKey, NonceLedger, PortBundle, PortImplementation, PortSlot,
    PreparedEffect, ProgramInstance, ProgramInstanceRef, ProgramInvocationRef, RuntimeAdmission,
    RuntimeAdmissionError, SignatureRejection, admitted_capability_permissions, digest_char,
    minimal_port_bindings, parse_execution_admission, unsigned_admission_skeleton,
    verify_execution_admission,
};

fn write_set(tag: char) -> AtomicWriteSet {
    AtomicWriteSet {
        next_program_state_digest: digest_char(tag),
        continuation_digest: digest_char('2'),
        checkpoint_effect_outcomes_digest: digest_char('3'),
        runtime_evidence_batch_digest: apxm_kernel::runtime_evidence_and_observation_digest(
            &[],
            &[],
        ),
        usage_facts_digest: digest_char('5'),
        session_output_refs_digest: apxm_kernel::session_output_refs_digest(&[]),
    }
}

#[test]
fn nonce_ledger_refuses_replay_without_eviction_or_unbounded_growth() {
    let ledger = NonceLedger::with_capacity(2);
    ledger.observe("nonce.1").expect("first nonce");
    ledger.observe("nonce.2").expect("second nonce");
    assert!(matches!(
        ledger.observe("nonce.3"),
        Err(AdmissionError::NonceLedgerFull { capacity: 2 })
    ));
    assert!(matches!(
        ledger.observe("nonce.1"),
        Err(AdmissionError::NonceReuse(nonce)) if nonce == "nonce.1"
    ));
    assert!(matches!(
        ledger.observe(&"x".repeat(apxm_kernel::MAX_NONCE_BYTES + 1)),
        Err(AdmissionError::NonceTooLong { .. })
    ));
}

fn identity(id: &str) -> ProgramIdentity {
    ProgramIdentity {
        artifact_digest: digest_char('a'),
        entrypoint: "run".into(),
        agent_identity_binding: "agent.1".into(),
        program_instance_id: Some(id.into()),
    }
}

struct FixtureState {
    versions: HashMap<String, u64>,
    by_id: HashMap<String, ExecutionCommitResult>,
    force_unknown: HashSet<String>,
    external_effects: AtomicU64,
}

struct FixtureCommit {
    state: Mutex<FixtureState>,
}

impl FixtureCommit {
    fn new() -> Self {
        Self {
            state: Mutex::new(FixtureState {
                versions: HashMap::new(),
                by_id: HashMap::new(),
                force_unknown: HashSet::new(),
                external_effects: AtomicU64::new(0),
            }),
        }
    }

    fn inject_outcome_unknown(&self, commit_id: &str) {
        self.state
            .lock()
            .expect("fixture")
            .force_unknown
            .insert(commit_id.into());
    }

    fn external_effects(&self) -> u64 {
        self.state
            .lock()
            .expect("fixture")
            .external_effects
            .load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ExecutionCommitPort for FixtureCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        let mut state = self.state.lock().expect("fixture");
        if let Some(prior) = state.by_id.get(&request.commit_id).cloned() {
            return prior;
        }
        if state.force_unknown.contains(&request.commit_id) {
            let result = ExecutionCommitResult::OutcomeUnknown {
                reconciliation_ref: format!("reconcile.{}", request.commit_id),
            };
            state
                .by_id
                .insert(request.commit_id.clone(), result.clone());
            return result;
        }
        let current = *state
            .versions
            .get(request.program_instance_ref.as_str())
            .unwrap_or(&0);
        if current != request.expected_program_state_version {
            return ExecutionCommitResult::CompareConflict {
                current_program_state_version: current,
            };
        }
        let new_version = current + 1;
        state
            .versions
            .insert(request.program_instance_ref.as_str().into(), new_version);
        // One external effect per winning commit only.
        state.external_effects.fetch_add(1, Ordering::SeqCst);
        let result = ExecutionCommitResult::Committed {
            new_program_state_version: new_version,
            evidence_position_ref: format!("evidence.{}", request.commit_id),
        };
        state
            .by_id
            .insert(request.commit_id.clone(), result.clone());
        result
    }

    async fn current_version(&self, program_instance_ref: &ProgramInstanceRef) -> u64 {
        *self
            .state
            .lock()
            .expect("fixture")
            .versions
            .get(program_instance_ref.as_str())
            .unwrap_or(&0)
    }

    async fn load_continuation(
        &self,
        _program_instance_ref: &ProgramInstanceRef,
    ) -> Option<serde_json::Value> {
        None
    }
}

struct ExactConfinement {
    sandbox_digest: String,
    policy_digest: String,
}

#[async_trait]
impl ConfinementPort for ExactConfinement {
    async fn attest(
        &self,
        request: ConfinementRequest,
    ) -> Result<ConfinementAttestation, ConfinementError> {
        if request.sandbox_digest != self.sandbox_digest {
            return Err(ConfinementError::UnadmittedSandbox {
                confinement_type: request.confinement_type.as_str(),
                sandbox_digest: request.sandbox_digest,
            });
        }
        if request.policy_digest != self.policy_digest {
            return Err(ConfinementError::UnadmittedPolicy {
                policy_digest: request.policy_digest,
            });
        }
        Ok(ConfinementAttestation {
            attestation_id: "attest.1".into(),
            host_id: request.host_id,
            execution_id: request.execution_id,
            confinement_type: request.confinement_type,
            sandbox_digest: request.sandbox_digest,
            policy_digest: request.policy_digest,
            attested_at: "2026-08-03T00:00:00Z".into(),
            signature: "fixture".into(),
        })
    }
}

struct RefuseCapability;

#[async_trait]
impl CapabilityPort for RefuseCapability {
    async fn invoke(&self, _request: CapabilityRequest) -> CapabilityOutcome {
        CapabilityOutcome::OutcomeUnknown {
            message: "capability must not run without admission".into(),
        }
    }
}

fn seal_valid(nonce: &str) -> (ExecutionAdmission, IssuerKeyring, IssuerSigningKey) {
    let signer = IssuerSigningKey::generate("issuer.test.1");
    let keyring = IssuerKeyring::from_keys([signer.enrollment(u64::MAX, false)]).expect("keyring");
    let unsigned = unsigned_admission_skeleton(
        "invocation.1",
        nonce,
        "runtime.audience.1",
        10_000,
        minimal_port_bindings(),
    );
    let sealed = signer.seal_admission(unsigned);
    (sealed, keyring, signer)
}

fn construct_from_admission(
    verified: &apxm_kernel::VerifiedExecutionAdmission,
    commit: Arc<dyn ExecutionCommitPort>,
    confinement: Arc<dyn ConfinementPort>,
) -> PortBundle {
    let mut entries = Vec::new();
    for binding in &verified.port_bindings {
        let implementation = match binding.slot {
            PortSlot::ExecutionCommit => PortImplementation::ExecutionCommit(commit.clone()),
            PortSlot::Confinement => PortImplementation::Confinement(confinement.clone()),
            PortSlot::Capability => PortImplementation::Capability(Arc::new(RefuseCapability)),
            other => panic!("unexpected slot in minimal fixture: {other:?}"),
        };
        entries.push((binding.clone(), implementation));
    }
    PortBundle::construct(&verified.bundle_spec, entries).expect("admitted bundle")
}

async fn runtime_from_admission(
    verified: &apxm_kernel::VerifiedExecutionAdmission,
    commit: Arc<dyn ExecutionCommitPort>,
    confinement: Arc<dyn ConfinementPort>,
) -> RuntimeAdmission {
    let mut entries = Vec::new();
    for binding in &verified.port_bindings {
        let implementation = match binding.slot {
            PortSlot::ExecutionCommit => PortImplementation::ExecutionCommit(commit.clone()),
            PortSlot::Confinement => PortImplementation::Confinement(confinement.clone()),
            PortSlot::Capability => PortImplementation::Capability(Arc::new(RefuseCapability)),
            other => panic!("unexpected slot in minimal fixture: {other:?}"),
        };
        entries.push((binding.clone(), implementation));
    }
    RuntimeAdmission::admit(verified.clone(), entries, "host.1", "exec.1")
        .await
        .expect("runtime admission")
}

#[test]
fn positive_signed_expiring_nonce_bound_admission_verifies() {
    let (admission, keyring, _) = seal_valid("nonce.positive.1");
    let ledger = NonceLedger::new();
    let verified = verify_execution_admission(
        &admission,
        &keyring,
        &ledger,
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect("valid admission");
    assert_eq!(verified.port_bindings.len(), 2);
    assert!(
        verified
            .port_bindings
            .iter()
            .any(|b| b.slot == PortSlot::Confinement)
    );
}

#[test]
fn negative_expired_signature_nonce_reuse_and_audience_fail_closed() {
    let (admission, keyring, signer) = seal_valid("nonce.neg.1");
    let ledger = NonceLedger::new();

    let expired = verify_execution_admission(
        &admission,
        &keyring,
        &ledger,
        "runtime.audience.1",
        10_000,
        false,
    );
    assert!(matches!(expired, Err(AdmissionError::Expired { .. })));

    let bad_audience = verify_execution_admission(
        &admission,
        &keyring,
        &NonceLedger::new(),
        "other.audience",
        1_000,
        false,
    );
    assert!(matches!(
        bad_audience,
        Err(AdmissionError::AudienceMismatch { .. })
    ));

    let ledger = NonceLedger::new();
    verify_execution_admission(
        &admission,
        &keyring,
        &ledger,
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect("first use");
    let reused = verify_execution_admission(
        &admission,
        &keyring,
        &ledger,
        "runtime.audience.1",
        1_000,
        false,
    );
    assert!(matches!(reused, Err(AdmissionError::NonceReuse(_))));

    let mut tampered = admission.clone();
    tampered.artifact_ref = "tampered".into();
    let bad_sig = verify_execution_admission(
        &tampered,
        &keyring,
        &NonceLedger::new(),
        "runtime.audience.1",
        1_000,
        false,
    );
    assert!(matches!(
        bad_sig,
        Err(AdmissionError::Signature(
            SignatureRejection::SignatureMismatch | SignatureRejection::DigestMismatch
        ))
    ));

    let revoked_keyring =
        IssuerKeyring::from_keys([signer.enrollment(u64::MAX, true)]).expect("revoked");
    let revoked = verify_execution_admission(
        &admission,
        &revoked_keyring,
        &NonceLedger::new(),
        "runtime.audience.1",
        1_000,
        false,
    );
    assert!(matches!(
        revoked,
        Err(AdmissionError::Signature(SignatureRejection::KeyRevoked))
    ));
}

#[test]
fn ambiguous_and_missing_bindings_fail_closed() {
    let signer = IssuerSigningKey::generate("issuer.test.2");
    let keyring = IssuerKeyring::from_keys([signer.enrollment(u64::MAX, false)]).expect("keyring");
    let mut bindings = minimal_port_bindings();
    bindings.push(bindings[0].clone()); // duplicate execution_commit
    let sealed = signer.seal_admission(unsigned_admission_skeleton(
        "invocation.ambig",
        "nonce.ambig",
        "runtime.audience.1",
        10_000,
        bindings,
    ));
    let err = verify_execution_admission(
        &sealed,
        &keyring,
        &NonceLedger::new(),
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect_err("ambiguous");
    assert_eq!(
        err,
        AdmissionError::AmbiguousBinding(PortSlot::ExecutionCommit)
    );

    let mut missing = minimal_port_bindings();
    missing.retain(|b| b.slot != "confinement");
    let sealed = signer.seal_admission(unsigned_admission_skeleton(
        "invocation.missing",
        "nonce.missing",
        "runtime.audience.1",
        10_000,
        missing,
    ));
    let err = verify_execution_admission(
        &sealed,
        &keyring,
        &NonceLedger::new(),
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect_err("missing confinement");
    assert_eq!(
        err,
        AdmissionError::MissingRequiredSlot(PortSlot::Confinement)
    );
}

#[test]
fn product_plane_and_ambient_authority_fail_closed() {
    let (admission, _, _) = seal_valid("nonce.ambient.1");
    let mut with_company = serde_json::to_value(&admission).expect("json");
    with_company.as_object_mut().expect("object").insert(
        "company_ref".into(),
        json!({"ref_type":"CompanyRef","ref":"c1"}),
    );
    assert!(matches!(
        parse_execution_admission(&with_company),
        Err(AdmissionError::ProductPlaneField(_))
    ));

    let signer = IssuerSigningKey::generate("issuer.ambient");
    let keyring = IssuerKeyring::from_keys([signer.enrollment(u64::MAX, false)]).expect("keyring");
    let mut ambient = unsigned_admission_skeleton(
        "invocation.ambient",
        "nonce.ambient.2",
        "runtime.audience.1",
        10_000,
        minimal_port_bindings(),
    );
    ambient
        .caller_correlations
        .insert("AWS_SECRET_ACCESS_KEY".into(), "x".into());
    let sealed = signer.seal_admission(ambient);
    let err = verify_execution_admission(
        &sealed,
        &keyring,
        &NonceLedger::new(),
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect_err("ambient");
    assert!(matches!(err, AdmissionError::AmbientAuthorityRefused(_)));

    let signer = IssuerSigningKey::generate("issuer.unconfined");
    let keyring = IssuerKeyring::from_keys([signer.enrollment(u64::MAX, false)]).expect("keyring");
    let mut unconfined = unsigned_admission_skeleton(
        "invocation.unconfined",
        "nonce.unconfined",
        "runtime.audience.1",
        10_000,
        minimal_port_bindings(),
    );
    unconfined.confinement.confinement_type = "unconfined".into();
    let sealed = signer.seal_admission(unconfined);
    let err = verify_execution_admission(
        &sealed,
        &keyring,
        &NonceLedger::new(),
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect_err("unconfined");
    assert_eq!(err, AdmissionError::UnconfinedForbidden);
}

#[test]
fn invalid_binding_does_not_consume_nonce_and_contract_slot_is_closed() {
    let signer = IssuerSigningKey::generate("issuer.binding");
    let keyring = IssuerKeyring::from_keys([signer.enrollment(u64::MAX, false)]).expect("keyring");
    let ledger = NonceLedger::new();
    let mut bindings = minimal_port_bindings();
    bindings[0].port_contract_schema_id = "apxm.capability-invocation".into();
    let invalid = signer.seal_admission(unsigned_admission_skeleton(
        "invocation.binding.invalid",
        "nonce.binding.1",
        "runtime.audience.1",
        10_000,
        bindings,
    ));
    assert!(matches!(
        verify_execution_admission(
            &invalid,
            &keyring,
            &ledger,
            "runtime.audience.1",
            1_000,
            false,
        ),
        Err(AdmissionError::PortContractSchemaMismatch { .. })
    ));

    let valid = signer.seal_admission(unsigned_admission_skeleton(
        "invocation.binding.valid",
        "nonce.binding.1",
        "runtime.audience.1",
        10_000,
        minimal_port_bindings(),
    ));
    verify_execution_admission(
        &valid,
        &keyring,
        &ledger,
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect("invalid records do not burn the nonce");
}

#[test]
fn optional_model_target_is_still_exact_and_closed() {
    let signer = IssuerSigningKey::generate("issuer.model");
    let keyring = IssuerKeyring::from_keys([signer.enrollment(u64::MAX, false)]).expect("keyring");
    let mut bindings = minimal_port_bindings();
    bindings.push(AdmittedPortBinding {
        slot: "model_inference".into(),
        port_contract_schema_id: "apxm.model-inference".into(),
        port_contract_digest: digest_char('7'),
        binding_digest: digest_char('8'),
        proof_digest: digest_char('9'),
    });
    let mut admission = unsigned_admission_skeleton(
        "invocation.model",
        "nonce.model.1",
        "runtime.audience.1",
        10_000,
        bindings,
    );
    admission.model_target = Some(AdmittedModelTarget {
        model_target_ref: "target.1".into(),
        model_deployment_ref: "deployment.1".into(),
        exact_port_binding_digest: digest_char('f'),
    });
    let sealed = signer.seal_admission(admission);
    assert_eq!(
        verify_execution_admission(
            &sealed,
            &keyring,
            &NonceLedger::new(),
            "runtime.audience.1",
            1_000,
            false,
        )
        .expect_err("present model target must match exactly"),
        AdmissionError::ModelBindingMismatch
    );
}

#[tokio::test]
async fn runtime_admission_rejects_rebinding_after_verification() {
    let (admission, keyring, _) = seal_valid("nonce.rebind.1");
    let verified = verify_execution_admission(
        &admission,
        &keyring,
        &NonceLedger::new(),
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect("admit");
    let mut entries = verified
        .port_bindings
        .iter()
        .cloned()
        .map(|mut binding| {
            if binding.slot == PortSlot::ExecutionCommit {
                binding.binding_digest = digest_char('f');
                (
                    binding,
                    PortImplementation::ExecutionCommit(Arc::new(FixtureCommit::new())),
                )
            } else {
                (
                    binding,
                    PortImplementation::Confinement(Arc::new(ExactConfinement {
                        sandbox_digest: verified.sandbox_digest.clone(),
                        policy_digest: verified.policy_digest.clone(),
                    })),
                )
            }
        })
        .collect::<Vec<_>>();
    let commit = Arc::new(FixtureCommit::new());
    if let Some((_, implementation)) = entries.first_mut() {
        *implementation = PortImplementation::ExecutionCommit(commit);
    }
    let result = RuntimeAdmission::admit(verified, entries, "host.1", "exec.1").await;
    assert!(matches!(
        result,
        Err(RuntimeAdmissionError::BindingMismatch(
            PortSlot::ExecutionCommit
        ))
    ));
}

#[tokio::test]
async fn confinement_escape_fails_closed_before_effects() {
    let (admission, keyring, _) = seal_valid("nonce.conf.1");
    let verified = verify_execution_admission(
        &admission,
        &keyring,
        &NonceLedger::new(),
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect("admit");
    let commit = Arc::new(FixtureCommit::new());
    let confinement = Arc::new(ExactConfinement {
        sandbox_digest: verified.sandbox_digest.clone(),
        policy_digest: verified.policy_digest.clone(),
    });
    let runtime = runtime_from_admission(&verified, commit, confinement.clone()).await;
    let bundle = runtime.into_bundle();
    let port = bundle.confinement().expect("confinement required").clone();
    let escape = port
        .attest(ConfinementRequest {
            host_id: "host.1".into(),
            execution_id: "exec.1".into(),
            confinement_type: verified.confinement_type,
            sandbox_digest: digest_char('f'), // wrong sandbox
            policy_digest: verified.policy_digest.clone(),
        })
        .await;
    assert!(matches!(
        escape,
        Err(ConfinementError::UnadmittedSandbox { .. })
    ));
}

#[tokio::test]
async fn crash_replay_lost_reply_one_advancer_no_duplicate_effect() {
    let (admission, keyring, _) = seal_valid("nonce.crash.1");
    let verified = verify_execution_admission(
        &admission,
        &keyring,
        &NonceLedger::new(),
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect("admit");

    let commit = Arc::new(FixtureCommit::new());
    let confinement = Arc::new(ExactConfinement {
        sandbox_digest: verified.sandbox_digest.clone(),
        policy_digest: verified.policy_digest.clone(),
    });
    let bundle = runtime_from_admission(&verified, commit.clone(), confinement)
        .await
        .into_bundle();
    let instance_id = "instance.crash.1";
    let instance = ProgramInstance::new(
        identity(instance_id),
        ProgramInstanceRef::new(instance_id),
        bundle,
    )
    .expect("instance");
    let advancer = CheckpointAdvancer::new(instance_id);
    assert!(advancer.fork().is_err(), "only one checkpoint advancer");

    // Lost reply / outcome_unknown: no advance, no external effect.
    commit.inject_outcome_unknown("commit.lost.1");
    let report = instance
        .invoke(Invocation {
            commit_id: "commit.lost.1".into(),
            program_invocation_ref: ProgramInvocationRef::new("invocation.1"),
            write_set: write_set('1'),
        })
        .await
        .expect("invoke");
    assert!(matches!(
        report,
        apxm_kernel::InvocationReport::OutcomeUnknown { .. }
    ));
    assert_eq!(advancer.sequence(), 0);
    assert_eq!(commit.external_effects(), 0);

    // Recovery commit wins once.
    let report = instance
        .invoke(Invocation {
            commit_id: "commit.ok.1".into(),
            program_invocation_ref: ProgramInvocationRef::new("invocation.1"),
            write_set: write_set('7'),
        })
        .await
        .expect("invoke");
    assert!(matches!(
        report,
        apxm_kernel::InvocationReport::Committed { .. }
    ));
    assert_eq!(
        advancer.advance_on_commit(instance_id, 1).expect("advance"),
        1
    );
    assert_eq!(
        advancer
            .advance_on_commit(instance_id, 1)
            .expect("idempotent replay"),
        1
    );
    assert_eq!(commit.external_effects(), 1);

    // Replay of the same commit id publishes nothing new.
    let replay = commit
        .commit(ExecutionCommitRequest {
            commit_id: "commit.ok.1".into(),
            program_instance_ref: ProgramInstanceRef::new(instance_id),
            program_invocation_ref: ProgramInvocationRef::new("invocation.1"),
            idempotency_key: "idem.ok.1".into(),
            expected_program_state_version: 0,
            write_set: write_set('7'),
            tuple: ExecutionCommitTuple::empty(vec![]),
            evidence_batch: vec![],
        })
        .await;
    assert!(matches!(replay, ExecutionCommitResult::Committed { .. }));
    assert_eq!(commit.external_effects(), 1, "no duplicate external effect");
    assert_eq!(advancer.sequence(), 1);
}

#[tokio::test]
async fn cancellation_and_revocation_before_send_are_terminal() {
    let prepared = PreparedEffect {
        effect_id: apxm_kernel::EffectId::new("effect.cancel.1").expect("id"),
        program_instance_ref: ProgramInstanceRef::new("instance.1"),
        program_invocation_ref: ProgramInvocationRef::new("invocation.1"),
        node_execution_id: "node.1".into(),
        request_digest: digest_char('a'),
        exact_binding_ref: "binding.1".into(),
        authority_decision_ref: "decision.1".into(),
        idempotency_contract_ref: "idem.1".into(),
    };
    let mut record = EffectRecord::prepare(prepared);
    record.apply(EffectTransition::CommitWon).expect("commit");
    record
        .apply(EffectTransition::CancelledBeforeSend)
        .expect("cancel");
    assert_eq!(record.state(), &EffectState::CancelledBeforeSend);
    assert!(record.state().authorizes_next_activation());

    let mut revoked = EffectRecord::prepare(PreparedEffect {
        effect_id: apxm_kernel::EffectId::new("effect.revoke.1").expect("id"),
        program_instance_ref: ProgramInstanceRef::new("instance.1"),
        program_invocation_ref: ProgramInvocationRef::new("invocation.1"),
        node_execution_id: "node.2".into(),
        request_digest: digest_char('a'),
        exact_binding_ref: "binding.1".into(),
        authority_decision_ref: "decision.1".into(),
        idempotency_contract_ref: "idem.1".into(),
    });
    revoked.apply(EffectTransition::CommitWon).expect("commit");
    revoked
        .apply(EffectTransition::AuthorityRevoked)
        .expect("revoke");
    assert_eq!(revoked.state(), &EffectState::RevokedBeforeSend);

    // outcome_unknown never authorizes blind replay / next activation.
    let mut unknown = EffectRecord::prepare(PreparedEffect {
        effect_id: apxm_kernel::EffectId::new("effect.unknown.1").expect("id"),
        program_instance_ref: ProgramInstanceRef::new("instance.1"),
        program_invocation_ref: ProgramInvocationRef::new("invocation.1"),
        node_execution_id: "node.3".into(),
        request_digest: digest_char('a'),
        exact_binding_ref: "binding.1".into(),
        authority_decision_ref: "decision.1".into(),
        idempotency_contract_ref: "idem.1".into(),
    });
    unknown.apply(EffectTransition::CommitWon).expect("commit");
    unknown
        .apply(EffectTransition::ClaimedBeforeSend {
            epoch: 1,
            attempt: 1,
        })
        .expect("claim");
    unknown
        .apply(EffectTransition::DispatchStarted)
        .expect("dispatch");
    unknown
        .apply(EffectTransition::ProviderOutcomeUnknown)
        .expect("unknown");
    assert_eq!(unknown.state(), &EffectState::OutcomeUnknown);
    assert!(!unknown.state().authorizes_next_activation());
    assert!(
        unknown.apply(EffectTransition::ExplicitNewAttempt).is_err(),
        "no blind replay from outcome_unknown"
    );
}

#[tokio::test]
async fn boundary_busy_instance_and_missing_port_fail_closed() {
    let (admission, keyring, _) = seal_valid("nonce.busy.1");
    let verified = verify_execution_admission(
        &admission,
        &keyring,
        &NonceLedger::new(),
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect("admit");
    let commit = Arc::new(FixtureCommit::new());
    let confinement = Arc::new(ExactConfinement {
        sandbox_digest: verified.sandbox_digest.clone(),
        policy_digest: verified.policy_digest.clone(),
    });
    let bundle = construct_from_admission(&verified, commit, confinement);
    let instance = ProgramInstance::new(
        identity("instance.busy"),
        ProgramInstanceRef::new("instance.busy"),
        bundle,
    )
    .expect("instance");

    // Identity mismatch is a construction boundary failure.
    let bad = ProgramInstance::new(
        identity("other"),
        ProgramInstanceRef::new("instance.busy"),
        construct_from_admission(
            &verified,
            Arc::new(FixtureCommit::new()),
            Arc::new(ExactConfinement {
                sandbox_digest: verified.sandbox_digest.clone(),
                policy_digest: verified.policy_digest.clone(),
            }),
        ),
    );
    assert!(matches!(
        bad,
        Err(InstanceError::ProgramInstanceIdentityMismatch { .. })
    ));

    // SchemaDigestRef unused guard for clarity of contract digests in bindings.
    let _ = SchemaDigestRef {
        schema_id: "apxm.execution-commit".into(),
        digest: digest_char('e'),
    };
    let _ = RuntimeEvidenceVersion::V1;
    let _ = ExactPortBinding {
        slot: PortSlot::ExecutionCommit,
        port_contract: SchemaDigestRef {
            schema_id: "apxm.execution-commit".into(),
            digest: digest_char('e'),
        },
        binding_digest: digest_char('b'),
        proof_digest: digest_char('f'),
    };
    let _ = instance;
}

#[test]
fn checkpoint_advancer_is_monotonic_and_replay_idempotent() {
    let advancer = CheckpointAdvancer::new("instance.checkpoint");
    assert!(matches!(
        advancer.advance_on_commit("instance.checkpoint", 2),
        Err(AdmissionError::CheckpointVersionMismatch {
            expected: 1,
            actual: 2
        })
    ));
    assert_eq!(
        advancer
            .advance_on_commit("instance.checkpoint", 1)
            .expect("first commit"),
        1
    );
    assert_eq!(
        advancer
            .advance_on_commit("instance.checkpoint", 1)
            .expect("replayed commit"),
        1
    );
}

/// The composition root resolves the layer stack and seals the outcome into
/// admission, so the runtime reads a decision it can attribute to a layer
/// rather than inferring one. Nothing here is a grant: `grant_fact_refs`
/// stays banned, and a decision may not travel as an opaque correlation.
#[test]
fn admission_carries_resolved_permission_decisions_but_never_a_grant() {
    let resolution = PermissionResolution::resolve(&BTreeMap::from([
        (
            PermissionLayer::Code,
            LayerDecisions::from([("cap.search".to_string(), PermissionDecision::allow())]),
        ),
        (
            PermissionLayer::Deployment,
            LayerDecisions::from([(
                "cap.search".to_string(),
                PermissionDecision::deny("no egress"),
            )]),
        ),
    ]))
    .expect("a tightening stack resolves");

    let signer = IssuerSigningKey::generate("issuer.permissions");
    let keyring = IssuerKeyring::from_keys([signer.enrollment(u64::MAX, false)]).expect("keyring");
    let mut unsigned = unsigned_admission_skeleton(
        "invocation.permissions",
        "nonce.permissions.1",
        "runtime.audience.1",
        10_000,
        minimal_port_bindings(),
    );
    unsigned.capability_permissions = admitted_capability_permissions(&resolution);
    let sealed = signer.seal_admission(unsigned);

    let verified = verify_execution_admission(
        &sealed,
        &keyring,
        &NonceLedger::new(),
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect("an admission carrying decisions verifies");
    let admitted = &verified.admission.capability_permissions;
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].capability_ref, "cap.search");
    assert_eq!(
        admitted[0].permission.decision,
        PermissionDecision::deny("no egress")
    );
    assert_eq!(
        admitted[0].permission.layer,
        PermissionLayer::Deployment,
        "the sealed decision names the layer that produced it"
    );

    // The typed field is the only way in. A decision spelled as a correlation,
    // or a grant reference beside it, is still refused.
    let mut smuggled = serde_json::to_value(&sealed).expect("encode admission");
    smuggled["grant_fact_refs"] = json!(["grant.search.1"]);
    assert!(matches!(
        parse_execution_admission(&smuggled),
        Err(AdmissionError::ProductPlaneField(field)) if field == "grant_fact_refs"
    ));
}

/// A repeated capability reference would leave which decision applies up to
/// iteration order, so it fails closed before anything is admitted.
#[test]
fn a_capability_decided_twice_in_one_admission_fails_closed() {
    let signer = IssuerSigningKey::generate("issuer.duplicate-permission");
    let keyring = IssuerKeyring::from_keys([signer.enrollment(u64::MAX, false)]).expect("keyring");
    let mut unsigned = unsigned_admission_skeleton(
        "invocation.duplicate",
        "nonce.duplicate.1",
        "runtime.audience.1",
        10_000,
        minimal_port_bindings(),
    );
    unsigned.capability_permissions = vec![
        AdmittedCapabilityPermission {
            capability_ref: "cap.search".into(),
            permission: ResolvedPermission {
                decision: PermissionDecision::deny("no egress"),
                layer: PermissionLayer::Deployment,
            },
        },
        AdmittedCapabilityPermission {
            capability_ref: "cap.search".into(),
            permission: ResolvedPermission {
                decision: PermissionDecision::allow(),
                layer: PermissionLayer::Code,
            },
        },
    ];
    let sealed = signer.seal_admission(unsigned);

    let error = verify_execution_admission(
        &sealed,
        &keyring,
        &NonceLedger::new(),
        "runtime.audience.1",
        1_000,
        false,
    )
    .expect_err("one capability, one decision");
    assert!(
        matches!(error, AdmissionError::DuplicateCapabilityPermission(ref capability_ref) if capability_ref == "cap.search"),
        "{error}"
    );
}
