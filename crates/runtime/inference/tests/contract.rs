//! Model inference contract: exact binding with no substitution, stable
//! request identity, retry discipline, usage, streaming, cancellation, and
//! outcome-unknown — driven by a deterministic in-crate fake backend. The fake
//! implements the contract for testing only and registers no live backend.

use std::cell::{Cell, RefCell};

use apxm_inference::{
    AttemptDisposition, BindingError, CancelToken, ErrorCategory, ExactModelTargetRef,
    ExactPortBindingRef, IdempotencyKey, ModelBindingAdmission, ModelCallPreparation,
    ModelCallRequest, ModelCallRequestMetadata, ModelContentRef, ModelContextEnvelopeRef,
    ModelDeploymentRef, ModelInferencePort, ModelOutcome, ModelStreamEvent, ModelStreamMode,
    ModelStreamPort, ModelStreamStep, ModelTargetRef, ResolvedModelBinding, RetryPolicy,
    TypedError, Usage, execute, stream,
};

const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const DIGEST_C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const DIGEST_D: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const DIGEST_E: &str = "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn binding(target: &str, deployment: &str, digest: &str) -> ResolvedModelBinding {
    ResolvedModelBinding {
        model_target: ExactModelTargetRef {
            reference: ModelTargetRef(target.to_string()),
            target_digest: DIGEST_B.to_string(),
        },
        model_deployment_ref: ModelDeploymentRef(deployment.to_string()),
        exact_port_binding: ExactPortBindingRef {
            binding_digest: digest.to_string(),
            port_contract_digest: DIGEST_C.to_string(),
        },
        composition_digest: DIGEST_D.to_string(),
    }
}

fn admission() -> ModelBindingAdmission {
    ModelBindingAdmission::new(binding("model.alpha", "deploy.alpha", DIGEST_A))
}

fn error() -> TypedError {
    TypedError {
        category: ErrorCategory::Unavailable,
        code: "backend_unavailable".to_string(),
        message: "backend unavailable".to_string(),
    }
}

fn metadata() -> ModelCallRequestMetadata {
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
    }
}

// ── Exact binding validation: no ambient/default/alias/first-available ──────

#[test]
fn validates_the_materialized_binding() {
    let resolved = admission()
        .validate(&ModelTargetRef("model.alpha".to_string()))
        .expect("validated binding");
    assert_eq!(resolved.model_deployment_ref.0, "deploy.alpha");
    assert_eq!(resolved.binding_digest(), DIGEST_A);
}

#[test]
fn mismatched_target_fails_closed_with_no_substitution() {
    let err = admission()
        .validate(&ModelTargetRef("model.beta".to_string()))
        .expect_err("no target substitution");
    assert!(matches!(err, BindingError::TargetMismatch { .. }));
}

#[test]
fn invalid_binding_digest_fails_closed() {
    let admission =
        ModelBindingAdmission::new(binding("model.alpha", "deploy.alpha", "not-a-digest"));
    let err = admission
        .validate(&ModelTargetRef("model.alpha".to_string()))
        .expect_err("invalid binding digest");
    assert!(matches!(err, BindingError::InvalidBindingDigest(_)));
}

#[test]
fn request_binds_only_its_authored_target() {
    let admission = ModelBindingAdmission::new(binding("model.beta", "deploy.beta", DIGEST_B));
    let request = ModelCallRequest::prepare(
        ModelCallPreparation::authorize(
            "effect.1",
            "node-execution.1",
            "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            &ModelTargetRef("model.beta".to_string()),
            &admission,
        )
        .expect("authorize authored target"),
        metadata(),
    )
    .expect("prepare request");
    assert_eq!(request.target().0, "model.beta");
    assert_eq!(request.resolved_binding().binding_digest(), DIGEST_B);

    assert!(
        ModelCallPreparation::authorize(
            "effect.2",
            "node-execution.2",
            "sha256:2222222222222222222222222222222222222222222222222222222222222222",
            &ModelTargetRef("model.substitute".to_string()),
            &admission,
        )
        .is_err()
    );
}

#[test]
fn prepared_request_preserves_exact_admission_and_host_coordinates() {
    let request = request();
    assert_eq!(request.node_execution_id().as_str(), "node-execution.1");
    assert_eq!(request.target().0, "model.alpha");
    assert_eq!(
        request.resolved_binding().model_target.target_digest,
        DIGEST_B
    );
    assert_eq!(request.resolved_binding().composition_digest, DIGEST_D);
    assert_eq!(request.resolved_binding().port_contract_digest(), DIGEST_C);
    assert_eq!(request.model_context_envelope_ref().sealed_digest, DIGEST_E);
    assert_eq!(request.idempotency().key_id, "idempotency.1");
    assert_eq!(request.idempotency().scope_ref, "scope.1");
    assert_eq!(request.stream_mode(), ModelStreamMode::Buffered);
}

#[test]
fn prepared_request_rejects_unknown_wire_fields() {
    let mut value = serde_json::to_value(request()).expect("serialize exact request");
    value
        .as_object_mut()
        .expect("request is an object")
        .insert("provider".into(), serde_json::json!("first_available"));

    assert!(serde_json::from_value::<ModelCallRequest>(value).is_err());
}

#[test]
fn missing_or_invalid_host_metadata_fails_before_dispatch() {
    let preparation = ModelCallPreparation::authorize(
        "effect.1",
        "node-execution.1",
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        &ModelTargetRef("model.alpha".to_string()),
        &admission(),
    )
    .expect("authorize exact binding");
    let mut missing_scope = metadata();
    missing_scope.idempotency.scope_ref.clear();
    assert!(matches!(
        ModelCallRequest::prepare(preparation.clone(), missing_scope),
        Err(apxm_inference::ModelCallRequestError::EmptyField(
            "idempotency.scope_ref"
        ))
    ));

    let mut invalid_context_digest = metadata();
    invalid_context_digest
        .model_context_envelope_ref
        .sealed_digest = "not-a-digest".into();
    assert!(matches!(
        ModelCallRequest::prepare(preparation, invalid_context_digest),
        Err(apxm_inference::ModelCallRequestError::InvalidDigest(
            "model_context_envelope_ref.sealed_digest"
        ))
    ));
}

#[test]
fn invalid_target_composition_or_port_contract_digest_fails_closed() {
    let mut invalid_target = binding("model.alpha", "deploy.alpha", DIGEST_A);
    invalid_target.model_target.target_digest = "not-a-digest".into();
    assert!(matches!(
        ModelBindingAdmission::new(invalid_target).validate(&ModelTargetRef("model.alpha".into())),
        Err(BindingError::InvalidTargetDigest(_))
    ));

    let mut invalid_composition = binding("model.alpha", "deploy.alpha", DIGEST_A);
    invalid_composition.composition_digest = "not-a-digest".into();
    assert!(matches!(
        ModelBindingAdmission::new(invalid_composition)
            .validate(&ModelTargetRef("model.alpha".into())),
        Err(BindingError::InvalidCompositionDigest(_))
    ));

    let mut invalid_port_contract = binding("model.alpha", "deploy.alpha", DIGEST_A);
    invalid_port_contract
        .exact_port_binding
        .port_contract_digest = "not-a-digest".into();
    assert!(matches!(
        ModelBindingAdmission::new(invalid_port_contract)
            .validate(&ModelTargetRef("model.alpha".into())),
        Err(BindingError::InvalidPortContractDigest(_))
    ));
}

#[test]
fn invocation_admission_keeps_each_exact_model_binding_distinct() {
    let admission = ModelBindingAdmission::for_invocation(vec![
        binding("model.alpha", "deploy.alpha", DIGEST_A),
        binding("model.beta", "deploy.beta", DIGEST_B),
    ]);

    let alpha = admission
        .validate(&ModelTargetRef("model.alpha".to_string()))
        .expect("alpha is admitted");
    let beta = admission
        .validate(&ModelTargetRef("model.beta".to_string()))
        .expect("beta is admitted");

    assert_eq!(alpha.model_deployment_ref.0, "deploy.alpha");
    assert_eq!(beta.model_deployment_ref.0, "deploy.beta");
    assert_eq!(alpha.binding_digest(), DIGEST_A);
    assert_eq!(beta.binding_digest(), DIGEST_B);
}

#[test]
fn duplicate_model_binding_target_fails_closed() {
    let admission = ModelBindingAdmission::for_invocation(vec![
        binding("model.alpha", "deploy.alpha", DIGEST_A),
        binding("model.alpha", "deploy.beta", DIGEST_B),
    ]);

    let err = admission
        .validate(&ModelTargetRef("model.alpha".to_string()))
        .expect_err("duplicate target is not an exact admission");
    assert!(matches!(err, BindingError::DuplicateTarget(_)));
}

// ── Deterministic fake backend ─────────────────────────────────────────────

struct ScriptedBackend {
    dispositions: Vec<AttemptDisposition>,
    proves_idempotency: bool,
    sends: Cell<u32>,
    seen_identity: RefCell<Vec<(String, String)>>,
}

impl ScriptedBackend {
    fn new(dispositions: Vec<AttemptDisposition>, proves_idempotency: bool) -> Self {
        Self {
            dispositions,
            proves_idempotency,
            sends: Cell::new(0),
            seen_identity: RefCell::new(Vec::new()),
        }
    }
}

impl ModelInferencePort for ScriptedBackend {
    fn attempt(&self, request: &ModelCallRequest, attempt: u32) -> AttemptDisposition {
        self.seen_identity.borrow_mut().push((
            request.effect_id().to_string(),
            request.request_digest().to_string(),
        ));
        let disposition = self.dispositions[attempt as usize].clone();
        if matches!(
            disposition,
            AttemptDisposition::Success(_)
                | AttemptDisposition::DeliveredTypedFailure(_)
                | AttemptDisposition::FailedAfterSend(_)
        ) {
            self.sends.set(self.sends.get() + 1);
        }
        disposition
    }

    fn proves_idempotency(&self) -> bool {
        self.proves_idempotency
    }
}

fn request() -> ModelCallRequest {
    ModelCallRequest::prepare(
        ModelCallPreparation::authorize(
            "effect.1",
            "node-execution.1",
            "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            &ModelTargetRef("model.alpha".to_string()),
            &admission(),
        )
        .expect("authorize"),
        metadata(),
    )
    .expect("prepare")
}

// ── Retry, usage, outcome-unknown ──────────────────────────────────────────

#[test]
fn success_commits_typed_usage() {
    let usage = Usage {
        input_tokens: 12,
        output_tokens: 34,
    };
    let backend = ScriptedBackend::new(vec![AttemptDisposition::Success(usage)], false);
    let outcome = execute(&backend, &request(), RetryPolicy::default());
    assert_eq!(outcome, ModelOutcome::CommittedSuccess { usage });
    assert_eq!(backend.sends.get(), 1);
}

#[test]
fn pre_send_failure_retries_then_succeeds() {
    let usage = Usage {
        input_tokens: 1,
        output_tokens: 2,
    };
    let backend = ScriptedBackend::new(
        vec![
            AttemptDisposition::FailedBeforeSend(error()),
            AttemptDisposition::FailedBeforeSend(error()),
            AttemptDisposition::Success(usage),
        ],
        false,
    );
    let outcome = execute(&backend, &request(), RetryPolicy { max_attempts: 3 });
    assert_eq!(outcome, ModelOutcome::CommittedSuccess { usage });
    assert_eq!(backend.sends.get(), 1, "only the committing attempt sends");
    // Every attempt reused the same stable request identity.
    let seen = backend.seen_identity.borrow();
    assert_eq!(seen.len(), 3);
    assert!(seen.iter().all(|id| *id == seen[0]));
}

#[test]
fn post_send_non_idempotent_failure_is_outcome_unknown_without_duplicate() {
    let backend = ScriptedBackend::new(
        vec![
            AttemptDisposition::FailedAfterSend(error()),
            // A second scripted disposition exists but must never be consumed.
            AttemptDisposition::Success(Usage::default()),
        ],
        false,
    );
    let outcome = execute(&backend, &request(), RetryPolicy { max_attempts: 3 });
    assert_eq!(
        outcome,
        ModelOutcome::ModelOutcomeUnknown {
            uncertain_usage: None
        }
    );
    assert_eq!(
        backend.sends.get(),
        1,
        "a non-idempotent request is never sent twice"
    );
    assert_eq!(backend.seen_identity.borrow().len(), 1, "no retry attempt");
}

#[test]
fn delivered_typed_failure_is_terminal_without_retry_or_outcome_unknown() {
    let delivered = TypedError {
        category: ErrorCategory::Validation,
        code: "context_length_exceeded".to_string(),
        message: "prompt exceeds the admitted context limit".to_string(),
    };
    let backend = ScriptedBackend::new(
        vec![
            AttemptDisposition::DeliveredTypedFailure(delivered.clone()),
            AttemptDisposition::Success(Usage::default()),
        ],
        true,
    );

    let outcome = execute(&backend, &request(), RetryPolicy { max_attempts: 3 });

    assert_eq!(outcome, ModelOutcome::TypedFailure { error: delivered });
    assert_eq!(backend.sends.get(), 1);
    assert_eq!(backend.seen_identity.borrow().len(), 1);
}

#[test]
fn post_send_idempotent_failure_may_retry() {
    let usage = Usage {
        input_tokens: 5,
        output_tokens: 6,
    };
    let backend = ScriptedBackend::new(
        vec![
            AttemptDisposition::FailedAfterSend(error()),
            AttemptDisposition::Success(usage),
        ],
        true,
    );
    let outcome = execute(&backend, &request(), RetryPolicy { max_attempts: 3 });
    assert_eq!(outcome, ModelOutcome::CommittedSuccess { usage });
    assert_eq!(
        backend.sends.get(),
        2,
        "idempotent backend reconciles and retries"
    );
}

#[test]
fn cancellation_commits_cancelled() {
    let backend = ScriptedBackend::new(vec![AttemptDisposition::Cancelled], false);
    let outcome = execute(&backend, &request(), RetryPolicy::default());
    assert_eq!(outcome, ModelOutcome::Cancelled);
}

// ── Streaming with explicit cancellation ───────────────────────────────────

struct ScriptedStream {
    steps: Vec<ModelStreamStep>,
    cancel_at: Option<u64>,
    calls: Cell<usize>,
}

impl ModelStreamPort for ScriptedStream {
    fn next_step(
        &self,
        _request: &ModelCallRequest,
        expected_sequence: u64,
        cancel: &CancelToken,
    ) -> ModelStreamStep {
        if self.cancel_at == Some(expected_sequence) {
            cancel.cancel();
        }
        let call = self.calls.get();
        self.calls.set(call + 1);
        self.steps[call].clone()
    }
}

fn content_delta(sequence: u64, content_ref: &str) -> ModelStreamEvent {
    ModelStreamEvent::ContentDelta {
        sequence,
        content_ref: ModelContentRef(content_ref.to_string()),
    }
}

fn tool_call_delta(sequence: u64, content_ref: &str) -> ModelStreamEvent {
    ModelStreamEvent::ToolCallDelta {
        sequence,
        content_ref: ModelContentRef(content_ref.to_string()),
    }
}

fn event(event: ModelStreamEvent) -> ModelStreamStep {
    ModelStreamStep::Event { event }
}

fn terminal(outcome: ModelOutcome) -> ModelStreamStep {
    ModelStreamStep::Terminal { outcome }
}

#[test]
fn stream_preserves_content_references_and_heartbeat_then_terminal() {
    let token = CancelToken::new();
    let events = vec![
        content_delta(0, "content://model-output/delta-0"),
        ModelStreamEvent::Heartbeat { sequence: 1 },
        tool_call_delta(2, "content://model-output/tool-call-0"),
    ];
    let backend = ScriptedStream {
        steps: vec![
            event(events[0].clone()),
            event(events[1].clone()),
            event(events[2].clone()),
            terminal(ModelOutcome::CommittedSuccess {
                usage: Usage {
                    input_tokens: 3,
                    output_tokens: 2,
                },
            }),
        ],
        cancel_at: None,
        calls: Cell::new(0),
    };
    let result = stream(&backend, &request(), &token);
    assert_eq!(result.events, events);
    assert!(matches!(
        result.terminal,
        ModelOutcome::CommittedSuccess { .. }
    ));
    assert_eq!(backend.calls.get(), 4, "one terminal step ends the stream");
}

#[test]
fn out_of_sequence_event_fails_closed_without_committing_the_event() {
    let token = CancelToken::new();
    let backend = ScriptedStream {
        steps: vec![event(content_delta(
            4,
            "content://model-output/out-of-sequence",
        ))],
        cancel_at: None,
        calls: Cell::new(0),
    };

    let result = stream(&backend, &request(), &token);

    assert!(result.events.is_empty());
    assert!(matches!(
        result.terminal,
        ModelOutcome::TypedFailure {
            error: TypedError {
                category: ErrorCategory::Internal,
                ref code,
                ..
            }
        } if code == "stream_sequence_mismatch"
    ));
    assert_eq!(backend.calls.get(), 1);
}

#[test]
fn pre_cancelled_stream_emits_no_events_or_transport_calls() {
    let token = CancelToken::new();
    token.cancel();
    let backend = ScriptedStream {
        steps: vec![event(content_delta(0, "content://model-output/delta-0"))],
        cancel_at: None,
        calls: Cell::new(0),
    };
    let result = stream(&backend, &request(), &token);
    assert!(result.events.is_empty());
    assert_eq!(result.terminal, ModelOutcome::Cancelled);
    assert_eq!(backend.calls.get(), 0);
}

#[test]
fn in_flight_cancellation_keeps_prior_events_and_marks_the_outcome_unknown() {
    let token = CancelToken::new();
    let first = content_delta(0, "content://model-output/delta-0");
    let backend = ScriptedStream {
        steps: vec![
            event(first.clone()),
            event(content_delta(1, "content://model-output/late-delta")),
            terminal(ModelOutcome::CommittedSuccess {
                usage: Usage::default(),
            }),
        ],
        cancel_at: Some(1),
        calls: Cell::new(0),
    };
    let result = stream(&backend, &request(), &token);
    assert_eq!(result.events, vec![first]);
    assert_eq!(
        result.terminal,
        ModelOutcome::ModelOutcomeUnknown {
            uncertain_usage: None
        }
    );
    assert_eq!(backend.calls.get(), 2);
}

#[test]
fn transport_confirmed_cancellation_remains_cancelled() {
    let token = CancelToken::new();
    let backend = ScriptedStream {
        steps: vec![terminal(ModelOutcome::Cancelled)],
        cancel_at: Some(0),
        calls: Cell::new(0),
    };

    let result = stream(&backend, &request(), &token);

    assert!(result.events.is_empty());
    assert_eq!(result.terminal, ModelOutcome::Cancelled);
    assert_eq!(backend.calls.get(), 1);
}

#[test]
fn an_exact_terminal_outcome_wins_a_racing_cancellation() {
    let token = CancelToken::new();
    let outcome = ModelOutcome::CommittedSuccess {
        usage: Usage {
            input_tokens: 3,
            output_tokens: 2,
        },
    };
    let backend = ScriptedStream {
        steps: vec![terminal(outcome.clone())],
        cancel_at: Some(0),
        calls: Cell::new(0),
    };

    let result = stream(&backend, &request(), &token);

    assert!(result.events.is_empty());
    assert_eq!(result.terminal, outcome);
    assert_eq!(backend.calls.get(), 1);
}
