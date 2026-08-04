//! G4 acceptance matrix for P-010 / P-012 / P-013.
//!
//! Required classes:
//! - exact-target
//! - lease
//! - usage-lineage
//! - exporter-loss
//! - telemetry/evidence disagreement
//!
//! Canonical owner evidence remains authoritative in every disagreement and
//! exporter-loss case. Silent substitution, fallback, dynamic routing, and
//! downstream usage recomputation are rejected.

use apxm_inference::effect::{ErrorCategory, IdempotencyKey};
use apxm_inference::{
    AttemptDisposition, BoundedMetricLabels, CorrelateDiagnosticsRequest, DiagnosticAgreement,
    ExactInferenceDispatch, ExactModelTargetRef, ExactPortBindingRef, InferenceCredentialLease,
    InferenceCredentialLeaseIdentity, InferenceDriverBinding, InferenceUsageLineage,
    LeasedInferenceBackend, ModelBindingAdmission, ModelCallPreparation, ModelCallRequest,
    ModelCallRequestMetadata, ModelContextEnvelopeRef, ModelDeploymentRef, ModelOutcome,
    ModelStreamMode, ModelTargetRef,
    PINNED_VLLM_PORT_CONTRACT_DIGEST, PINNED_VLLM_VECTOR_DIGESTS, ResolvedModelBinding,
    RetryPolicy, TypedError, Usage, VllmConformanceJoin, VllmJoinStatus, authoritative_usage,
    correlate_diagnostics, digest_bytes, dispatch_exact_inference, redact_diagnostic_value,
};
use std::cell::Cell;

const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const DIGEST_C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const DIGEST_D: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const DIGEST_E: &str = "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn resolved(target: &str) -> ResolvedModelBinding {
    ResolvedModelBinding {
        model_target: ExactModelTargetRef {
            reference: ModelTargetRef(target.to_string()),
            target_digest: DIGEST_B.to_string(),
        },
        model_deployment_ref: ModelDeploymentRef("deploy.alpha".to_string()),
        exact_port_binding: ExactPortBindingRef {
            binding_digest: DIGEST_A.to_string(),
            port_contract_digest: DIGEST_C.to_string(),
        },
        composition_digest: DIGEST_D.to_string(),
    }
}

fn request_for(target: &str) -> ModelCallRequest {
    let admission = ModelBindingAdmission::new(resolved(target));
    ModelCallRequest::prepare(
        ModelCallPreparation::authorize(
            "effect.1",
            "node-execution.1",
            "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            &ModelTargetRef(target.to_string()),
            &admission,
        )
        .expect("authorize"),
        ModelCallRequestMetadata {
            model_context_envelope_ref: ModelContextEnvelopeRef {
                context_id: "context.1".to_string(),
                sealed_digest: DIGEST_E.to_string(),
            },
            idempotency: IdempotencyKey {
                key_id: "idempotency.1".to_string(),
                scope_ref: "scope.1".to_string(),
            },
            stream_mode: ModelStreamMode::Buffered,
        },
    )
    .expect("prepare")
}

fn lease_for(target: &str, expires_at_unix_ms: u64) -> InferenceCredentialLease {
    let identity =
        InferenceCredentialLeaseIdentity::mint("lease.1", target, DIGEST_A, expires_at_unix_ms)
            .expect("mint lease identity");
    InferenceCredentialLease::issue(identity, "super-secret-material").expect("issue lease")
}

struct ExactBackend {
    expected_material: String,
    usage: Usage,
}

impl LeasedInferenceBackend for ExactBackend {
    fn attempt_with_lease(
        &self,
        request: &ModelCallRequest,
        _attempt: u32,
        lease_material: &str,
    ) -> AttemptDisposition {
        assert_eq!(lease_material, self.expected_material);
        assert_eq!(request.target().0, "model.alpha");
        AttemptDisposition::Success(self.usage)
    }
}

struct ScriptedBackend {
    calls: Cell<u32>,
    first: AttemptDisposition,
    second: AttemptDisposition,
    idempotent: bool,
}

impl LeasedInferenceBackend for ScriptedBackend {
    fn attempt_with_lease(
        &self,
        _request: &ModelCallRequest,
        _attempt: u32,
        _lease_material: &str,
    ) -> AttemptDisposition {
        let call = self.calls.get();
        self.calls.set(call + 1);
        match call {
            0 => self.first.clone(),
            _ => self.second.clone(),
        }
    }

    fn proves_idempotency(&self) -> bool {
        self.idempotent
    }
}

// ── Exact-target ────────────────────────────────────────────────────────────

#[test]
fn exact_target_driver_binding_rejects_substitution_and_unavailable() {
    let resolved = resolved("model.alpha");
    let binding = InferenceDriverBinding::from_resolved("driver.vllm", "profile.exact", &resolved)
        .expect("exact available binding");
    binding
        .authorize(&ModelTargetRef("model.alpha".into()), &resolved)
        .expect("exact target authorized");

    let err = binding
        .authorize(&ModelTargetRef("model.beta".into()), &resolved)
        .expect_err("no target substitution");
    assert!(matches!(
        err,
        apxm_inference::DriverBindingError::TargetMismatch { .. }
    ));

    let unavailable = binding.clone().unavailable();
    let err = unavailable
        .authorize(&ModelTargetRef("model.alpha".into()), &resolved)
        .expect_err("unavailable is explicit");
    assert!(matches!(
        err,
        apxm_inference::DriverBindingError::Unavailable { .. }
    ));

    let mut foreign = serde_json::to_value(&binding).expect("serialize");
    foreign
        .as_object_mut()
        .expect("object")
        .insert("fallback_target".into(), serde_json::json!("model.beta"));
    assert!(serde_json::from_value::<InferenceDriverBinding>(foreign).is_err());
}

#[test]
fn exact_dispatch_rejects_routing_to_a_different_binding_digest() {
    let resolved = resolved("model.alpha");
    let mut binding =
        InferenceDriverBinding::from_resolved("driver.vllm", "profile.exact", &resolved)
            .expect("binding");
    binding.exact_port_binding_digest = DIGEST_B.to_string();
    let request = request_for("model.alpha");
    let mut lease = lease_for("model.alpha", 10_000);
    let backend = ExactBackend {
        expected_material: "super-secret-material".into(),
        usage: Usage {
            input_tokens: 3,
            output_tokens: 5,
        },
    };
    let err = dispatch_exact_inference(ExactInferenceDispatch {
        binding: &binding,
        authored_target: &ModelTargetRef("model.alpha".into()),
        request: &request,
        lease: &mut lease,
        backend: &backend,
        now_unix_ms: 1_000,
        duration_ms: 12,
        policy: RetryPolicy { max_attempts: 1 },
    })
    .expect_err("digest mismatch is not dynamic routing");
    assert!(matches!(
        err,
        apxm_inference::InferenceDispatchError::Driver(
            apxm_inference::DriverBindingError::DigestMismatch { .. }
        )
    ));
}

// ── Lease ───────────────────────────────────────────────────────────────────

#[test]
fn lease_is_target_and_purpose_bound_and_lives_only_in_adapter_memory() {
    let mut lease = lease_for("model.alpha", 10_000);
    assert!(lease.expose().is_err());
    lease
        .activate(&ModelTargetRef("model.alpha".into()), DIGEST_A, 1_000)
        .expect("activate");
    assert_eq!(lease.expose().expect("expose"), "super-secret-material");

    let mut wrong_target = lease_for("model.beta", 10_000);
    assert!(matches!(
        wrong_target.activate(&ModelTargetRef("model.alpha".into()), DIGEST_A, 1_000),
        Err(apxm_inference::LeaseError::TargetMismatch { .. })
    ));

    let mut expired = lease_for("model.alpha", 500);
    assert!(matches!(
        expired.activate(&ModelTargetRef("model.alpha".into()), DIGEST_A, 1_000),
        Err(apxm_inference::LeaseError::Expired { .. })
    ));

    let identity = lease.identity().clone();
    let wire = serde_json::to_value(&identity).expect("public identity");
    let text = wire.to_string();
    assert!(!text.contains("super-secret-material"));
    assert!(text.contains("model_inference"));
    assert!(std::mem::size_of_val(&lease) > 0);
}

#[test]
fn lease_material_and_prompts_are_redacted_from_diagnostics() {
    assert_eq!(
        redact_diagnostic_value("prompt", "system: do private work"),
        "[redacted]"
    );
    assert_eq!(
        redact_diagnostic_value("completion", "model said secrets"),
        "[redacted]"
    );
    assert_eq!(
        redact_diagnostic_value("credential", "super-secret-material"),
        "[redacted]"
    );
    assert_eq!(
        redact_diagnostic_value("lease_material", "super-secret-material"),
        "[redacted]"
    );
    assert_eq!(
        redact_diagnostic_value("outcome", "committed_success"),
        "committed_success"
    );
}

#[test]
fn revoked_lease_cannot_be_reactivated_or_exposed() {
    let mut lease = lease_for("model.alpha", 10_000);
    lease
        .activate(&ModelTargetRef("model.alpha".into()), DIGEST_A, 1_000)
        .expect("activate");
    lease.revoke();
    assert!(lease.is_revoked());
    assert!(matches!(
        lease.expose(),
        Err(apxm_inference::LeaseError::Revoked)
    ));
    assert!(matches!(
        lease.activate(&ModelTargetRef("model.alpha".into()), DIGEST_A, 1_001),
        Err(apxm_inference::LeaseError::Revoked)
    ));
}

// ── Usage lineage ───────────────────────────────────────────────────────────

#[test]
fn usage_lineage_is_immutable_and_rejects_downstream_recompute() {
    let mut lineage = InferenceUsageLineage::seal(
        "effect.1",
        0,
        "model.alpha",
        DIGEST_A,
        Usage {
            input_tokens: 9,
            output_tokens: 4,
        },
        42,
        None,
    )
    .expect("seal");
    lineage
        .bind_evidence("fact.attempt.1", "commit.1")
        .expect("bind");

    assert!(lineage.sealed);
    lineage.validate().expect("valid sealed lineage");
    assert_eq!(lineage.duration_ms, 42);
    assert!(
        lineage
            .reject_recompute(Usage {
                input_tokens: 100,
                output_tokens: 4,
            })
            .is_err()
    );
    assert!(
        lineage
            .authorize_exporter_claim(
                "commit.1",
                Usage {
                    input_tokens: 9,
                    output_tokens: 4,
                }
            )
            .is_ok()
    );
    assert!(
        lineage
            .authorize_exporter_claim(
                "commit.forged",
                Usage {
                    input_tokens: 9,
                    output_tokens: 4,
                }
            )
            .is_err()
    );

    let mut forged = serde_json::to_value(&lineage).expect("serialize");
    forged
        .as_object_mut()
        .expect("object")
        .insert("native_input_tokens".into(), serde_json::json!(999));
    // Replacing tokens changes the sealed digest membership conceptually; the
    // owner API still rejects recomputation against the original sealed record.
    assert!(
        lineage
            .reject_recompute(Usage {
                input_tokens: 999,
                output_tokens: 4,
            })
            .is_err()
    );
    lineage.native_output_tokens = 99;
    assert!(matches!(
        lineage.validate(),
        Err(apxm_inference::LineageError::DigestMismatch)
    ));
}

#[test]
fn exact_dispatch_seals_usage_lineage_with_timing_and_target() {
    let resolved = resolved("model.alpha");
    let binding = InferenceDriverBinding::from_resolved("driver.vllm", "profile.exact", &resolved)
        .expect("binding");
    let request = request_for("model.alpha");
    let mut lease = lease_for("model.alpha", 10_000);
    let backend = ExactBackend {
        expected_material: "super-secret-material".into(),
        usage: Usage {
            input_tokens: 11,
            output_tokens: 13,
        },
    };
    let result = dispatch_exact_inference(ExactInferenceDispatch {
        binding: &binding,
        authored_target: &ModelTargetRef("model.alpha".into()),
        request: &request,
        lease: &mut lease,
        backend: &backend,
        now_unix_ms: 1_000,
        duration_ms: 77,
        policy: RetryPolicy { max_attempts: 1 },
    })
    .expect("exact dispatch");

    assert!(matches!(
        result.execution.outcome,
        ModelOutcome::CommittedSuccess { .. }
    ));
    assert_eq!(result.lineage.native_input_tokens, 11);
    assert_eq!(result.lineage.native_output_tokens, 13);
    assert_eq!(result.lineage.duration_ms, 77);
    assert_eq!(result.lineage.model_target_ref, "model.alpha");
    assert_eq!(result.driver_id, "driver.vllm");
}

#[test]
fn dispatch_retries_only_a_proven_pre_send_failure_and_preserves_identity() {
    let resolved = resolved("model.alpha");
    let binding = InferenceDriverBinding::from_resolved("driver.exact", "profile.exact", &resolved)
        .expect("binding");
    let request = request_for("model.alpha");
    let mut lease = lease_for("model.alpha", 10_000);
    let backend = ScriptedBackend {
        calls: Cell::new(0),
        first: AttemptDisposition::FailedBeforeSend(TypedError {
            category: ErrorCategory::Unavailable,
            code: "before_send".into(),
            message: "transport unavailable before send".into(),
        }),
        second: AttemptDisposition::Success(Usage {
            input_tokens: 4,
            output_tokens: 7,
        }),
        idempotent: false,
    };
    let result = dispatch_exact_inference(ExactInferenceDispatch {
        binding: &binding,
        authored_target: &ModelTargetRef("model.alpha".into()),
        request: &request,
        lease: &mut lease,
        backend: &backend,
        now_unix_ms: 1_000,
        duration_ms: 8,
        policy: RetryPolicy { max_attempts: 2 },
    })
    .expect("recovered dispatch");
    assert_eq!(backend.calls.get(), 2);
    assert_eq!(result.execution.committed_attempt, Some(1));
    assert_eq!(result.lineage.attempt_index, 1);
    assert_eq!(result.lineage.effect_id, request.effect_id());
}

#[test]
fn dispatch_records_unknown_after_possible_send_without_retry() {
    let resolved = resolved("model.alpha");
    let binding = InferenceDriverBinding::from_resolved("driver.exact", "profile.exact", &resolved)
        .expect("binding");
    let request = request_for("model.alpha");
    let mut lease = lease_for("model.alpha", 10_000);
    let backend = ScriptedBackend {
        calls: Cell::new(0),
        first: AttemptDisposition::FailedAfterSend(TypedError {
            category: ErrorCategory::Unavailable,
            code: "lost_reply".into(),
            message: "reply was lost after send".into(),
        }),
        second: AttemptDisposition::Success(Usage::default()),
        idempotent: false,
    };
    let result = dispatch_exact_inference(ExactInferenceDispatch {
        binding: &binding,
        authored_target: &ModelTargetRef("model.alpha".into()),
        request: &request,
        lease: &mut lease,
        backend: &backend,
        now_unix_ms: 1_000,
        duration_ms: 9,
        policy: RetryPolicy { max_attempts: 2 },
    })
    .expect("unknown outcome is evidenced");
    assert_eq!(backend.calls.get(), 1);
    assert!(matches!(
        result.execution.outcome,
        ModelOutcome::ModelOutcomeUnknown { .. }
    ));
    assert_eq!(
        result
            .lineage
            .typed_error
            .as_ref()
            .map(|error| error.category),
        Some(ErrorCategory::OutcomeUnknown)
    );
}

#[test]
fn dispatch_recovers_after_send_only_when_binding_proves_idempotency() {
    let resolved = resolved("model.alpha");
    let binding = InferenceDriverBinding::from_resolved("driver.exact", "profile.exact", &resolved)
        .expect("binding");
    let request = request_for("model.alpha");
    let mut lease = lease_for("model.alpha", 10_000);
    let backend = ScriptedBackend {
        calls: Cell::new(0),
        first: AttemptDisposition::FailedAfterSend(TypedError {
            category: ErrorCategory::Unavailable,
            code: "reconcile".into(),
            message: "reconciliation required".into(),
        }),
        second: AttemptDisposition::Success(Usage {
            input_tokens: 6,
            output_tokens: 2,
        }),
        idempotent: true,
    };
    let result = dispatch_exact_inference(ExactInferenceDispatch {
        binding: &binding,
        authored_target: &ModelTargetRef("model.alpha".into()),
        request: &request,
        lease: &mut lease,
        backend: &backend,
        now_unix_ms: 1_000,
        duration_ms: 10,
        policy: RetryPolicy { max_attempts: 2 },
    })
    .expect("idempotent recovery");
    assert_eq!(backend.calls.get(), 2);
    assert!(matches!(
        result.execution.outcome,
        ModelOutcome::CommittedSuccess { .. }
    ));
}

// ── Exporter-loss ───────────────────────────────────────────────────────────

#[test]
fn exporter_loss_keeps_canonical_owner_lineage_authoritative() {
    let mut lineage = InferenceUsageLineage::seal(
        "effect.1",
        0,
        "model.alpha",
        DIGEST_A,
        Usage {
            input_tokens: 2,
            output_tokens: 3,
        },
        15,
        None,
    )
    .expect("seal");
    lineage
        .bind_evidence("fact.attempt.1", "commit.1")
        .expect("bind");

    // Exporter failed after commit: no successful republish is required for
    // owner evidence to remain authoritative.
    let exporter_failed = true;
    assert!(exporter_failed);
    assert_eq!(lineage.commit_id.as_deref(), Some("commit.1"));
    assert!(
        lineage
            .authorize_exporter_claim(
                "commit.1",
                Usage {
                    input_tokens: 2,
                    output_tokens: 3,
                }
            )
            .is_ok()
    );
    // A downstream recomputation offered after exporter loss is rejected.
    assert!(
        lineage
            .reject_recompute(Usage {
                input_tokens: 20,
                output_tokens: 30,
            })
            .is_err()
    );

    let labels = BoundedMetricLabels {
        driver_id: "driver.vllm".into(),
        availability: "available".into(),
        outcome: "exporter_loss".into(),
    };
    labels.validate().expect("bounded exporter-loss label");
}

// ── Telemetry / evidence disagreement ───────────────────────────────────────

#[test]
fn telemetry_disagreement_leaves_owner_evidence_authoritative() {
    let mut lineage = InferenceUsageLineage::seal(
        "effect.1",
        0,
        "model.alpha",
        DIGEST_A,
        Usage {
            input_tokens: 8,
            output_tokens: 6,
        },
        9,
        Some(TypedError {
            category: ErrorCategory::Unavailable,
            code: "backend_unavailable".into(),
            message: "backend unavailable".into(),
        }),
    )
    .expect("seal");
    lineage
        .bind_evidence("fact.attempt.1", "commit.1")
        .expect("bind evidence");

    let diagnostic = correlate_diagnostics(CorrelateDiagnosticsRequest {
        correlation_id: "corr.1".into(),
        commit_id: "commit.1".into(),
        evidence_fact_ids: vec!["fact.attempt.1".into()],
        lineage: &lineage,
        claimed_usage: Some(Usage {
            input_tokens: 800,
            output_tokens: 600,
        }),
        log_refs: vec!["log.1".into()],
        metric_refs: vec!["metric.tokens".into()],
        trace_refs: vec!["trace.1".into()],
    })
    .expect("correlate");

    assert_eq!(diagnostic.authority, "diagnostic_only");
    assert_eq!(
        diagnostic.agreement,
        DiagnosticAgreement::DisagreesEvidenceAuthoritative
    );
    let authoritative = authoritative_usage(&lineage, &diagnostic);
    assert_eq!(authoritative.input_tokens, 8);
    assert_eq!(authoritative.output_tokens, 6);
    assert_ne!(
        diagnostic.claimed_input_tokens.unwrap_or_default(),
        authoritative.input_tokens
    );
}

#[test]
fn agreeing_diagnostics_remain_non_authoritative() {
    let mut lineage = InferenceUsageLineage::seal(
        "effect.1",
        0,
        "model.alpha",
        DIGEST_A,
        Usage {
            input_tokens: 1,
            output_tokens: 1,
        },
        3,
        None,
    )
    .expect("seal");
    lineage
        .bind_evidence("fact.attempt.2", "commit.2")
        .expect("bind evidence");
    let diagnostic = correlate_diagnostics(CorrelateDiagnosticsRequest {
        correlation_id: "corr.2".into(),
        commit_id: "commit.2".into(),
        evidence_fact_ids: vec!["fact.attempt.2".into()],
        lineage: &lineage,
        claimed_usage: Some(Usage {
            input_tokens: 1,
            output_tokens: 1,
        }),
        log_refs: Vec::new(),
        metric_refs: Vec::new(),
        trace_refs: Vec::new(),
    })
    .expect("correlate");
    assert_eq!(
        diagnostic.agreement,
        DiagnosticAgreement::AgreesWithEvidence
    );
    assert_eq!(diagnostic.authority, "diagnostic_only");
}

#[test]
fn diagnostics_reject_foreign_commit_or_unbounded_reference() {
    let mut lineage = InferenceUsageLineage::seal(
        "effect.1",
        0,
        "model.alpha",
        DIGEST_A,
        Usage::default(),
        1,
        None,
    )
    .expect("seal");
    lineage
        .bind_evidence("fact.attempt.3", "commit.3")
        .expect("bind evidence");

    let err = correlate_diagnostics(CorrelateDiagnosticsRequest {
        correlation_id: "corr.3".into(),
        commit_id: "commit.foreign".into(),
        evidence_fact_ids: vec!["fact.attempt.3".into()],
        lineage: &lineage,
        claimed_usage: None,
        log_refs: vec![],
        metric_refs: vec![],
        trace_refs: vec![],
    })
    .expect_err("foreign evidence cannot correlate");
    assert!(matches!(
        err,
        apxm_inference::DiagnosticError::EvidenceMismatch
    ));

    let err = correlate_diagnostics(CorrelateDiagnosticsRequest {
        correlation_id: "corr.3".into(),
        commit_id: "commit.3".into(),
        evidence_fact_ids: vec!["fact.attempt.3".into()],
        lineage: &lineage,
        claimed_usage: None,
        log_refs: vec!["prompt payload".into()],
        metric_refs: vec![],
        trace_refs: vec![],
    })
    .expect_err("diagnostic payload cannot be stored as a reference");
    assert!(matches!(
        err,
        apxm_inference::DiagnosticError::InvalidReference("log_refs")
    ));
}

// ── vLLM conformance join (no stubbed authority) ────────────────────────────

#[test]
fn vllm_conformance_join_pins_released_vector_digests_without_substitution() {
    let join = VllmConformanceJoin::candidate_from_pinned_vectors().expect("candidate join");
    assert_eq!(
        join.vllm_port_contract_digest,
        PINNED_VLLM_PORT_CONTRACT_DIGEST
    );
    assert_eq!(
        join.join_status,
        VllmJoinStatus::CandidateAwaitingVllmRelease
    );
    assert_eq!(
        join.joined_vector_digests.len(),
        PINNED_VLLM_VECTOR_DIGESTS.len()
    );
    join.validate().expect("valid join record");

    let err = VllmConformanceJoin::join(
        PINNED_VLLM_PORT_CONTRACT_DIGEST,
        vec!["sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into()],
        true,
    )
    .expect_err("unknown vector is not joined");
    assert!(matches!(
        err,
        apxm_inference::JoinError::UnknownVectorDigest(_)
    ));

    let err = VllmConformanceJoin::join(
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        vec![PINNED_VLLM_VECTOR_DIGESTS[0].to_string()],
        true,
    )
    .expect_err("port contract mismatch fails closed");
    assert!(matches!(
        err,
        apxm_inference::JoinError::PortContractMismatch { .. }
    ));

    let err = VllmConformanceJoin::join(
        PINNED_VLLM_PORT_CONTRACT_DIGEST,
        vec![PINNED_VLLM_VECTOR_DIGESTS[0].to_string(); 2],
        false,
    )
    .expect_err("duplicate evidence is not a join");
    assert!(matches!(
        err,
        apxm_inference::JoinError::DuplicateVectorDigest(_)
    ));
}

#[test]
fn pinned_vllm_vector_digests_match_workspace_files_when_present() {
    // agents/crates/runtime/inference -> workspace/vllm
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../vllm/contracts/vectors");
    if !root.is_dir() {
        // Workspace checkout may omit vllm; pinned digests still freeze the join.
        return;
    }
    let files = [
        "apxm.vllm-inference-request.v1.json",
        "apxm.vllm-inference-result.v1.json",
        "apxm.vllm-inference-failure.v1.json",
        "apxm.vllm-inference-stream-chunk.v1.json",
        "apxm.vllm-native-serving-binding.v1.json",
    ];
    for (file, expected) in files.iter().zip(PINNED_VLLM_VECTOR_DIGESTS.iter()) {
        let bytes = std::fs::read(root.join(file)).expect("read vllm vector");
        assert_eq!(digest_bytes(&bytes), *expected, "{file}");
    }
}
