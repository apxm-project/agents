//! Model inference contract: exact binding with no substitution, stable
//! request identity, retry discipline, usage, streaming, cancellation, and
//! outcome-unknown — driven by a deterministic in-crate fake backend. The fake
//! implements the contract for testing only and registers no live backend.

use std::cell::{Cell, RefCell};

use apxm_inference::{
    AttemptDisposition, BindingError, CancelToken, ErrorCategory, ExactPortBindingRef,
    ModelBindingAdmission, ModelCallRequest, ModelDeploymentRef, ModelInferencePort, ModelOutcome,
    ModelStreamPort, ModelTargetRef, ResolvedModelBinding, RetryPolicy, StreamChunk, TypedError,
    Usage, execute, stream,
};

const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn binding(target: &str, deployment: &str, digest: &str) -> ResolvedModelBinding {
    ResolvedModelBinding {
        model_target_ref: ModelTargetRef(target.to_string()),
        model_deployment_ref: ModelDeploymentRef(deployment.to_string()),
        exact_port_binding: ExactPortBindingRef {
            binding_digest: digest.to_string(),
        },
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
    let request = ModelCallRequest::authorize(
        "effect.1",
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        &ModelTargetRef("model.beta".to_string()),
        &admission,
    )
    .expect("authorize authored target");
    assert_eq!(request.target().0, "model.beta");
    assert_eq!(request.resolved_binding.binding_digest(), DIGEST_B);

    assert!(
        ModelCallRequest::authorize(
            "effect.2",
            "sha256:2222222222222222222222222222222222222222222222222222222222222222",
            &ModelTargetRef("model.substitute".to_string()),
            &admission,
        )
        .is_err()
    );
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
        self.seen_identity
            .borrow_mut()
            .push((request.effect_id.clone(), request.request_digest.clone()));
        let disposition = self.dispositions[attempt as usize].clone();
        if matches!(
            disposition,
            AttemptDisposition::Success(_) | AttemptDisposition::FailedAfterSend(_)
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
    ModelCallRequest::authorize(
        "effect.1",
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        &ModelTargetRef("model.alpha".to_string()),
        &admission(),
    )
    .expect("authorize")
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
    chunks: Vec<StreamChunk>,
    terminal: ModelOutcome,
    cancel_at: Option<u64>,
    token: CancelToken,
}

impl ModelStreamPort for ScriptedStream {
    fn next_chunk(&self, _request: &ModelCallRequest, sequence: u64) -> Option<StreamChunk> {
        if self.cancel_at == Some(sequence) {
            self.token.cancel();
        }
        self.chunks.get(sequence as usize).cloned()
    }

    fn terminal_outcome(&self, _request: &ModelCallRequest) -> ModelOutcome {
        self.terminal.clone()
    }
}

fn chunk(sequence: u64, text: &str) -> StreamChunk {
    StreamChunk {
        sequence,
        text: text.to_string(),
    }
}

#[test]
fn stream_yields_ordered_chunks_then_terminal() {
    let token = CancelToken::new();
    let backend = ScriptedStream {
        chunks: vec![chunk(0, "he"), chunk(1, "llo")],
        terminal: ModelOutcome::CommittedSuccess {
            usage: Usage {
                input_tokens: 3,
                output_tokens: 2,
            },
        },
        cancel_at: None,
        token: token.clone(),
    };
    let result = stream(&backend, &request(), &token);
    assert_eq!(result.chunks, vec![chunk(0, "he"), chunk(1, "llo")]);
    assert!(matches!(
        result.terminal,
        ModelOutcome::CommittedSuccess { .. }
    ));
}

#[test]
fn pre_cancelled_stream_emits_no_chunks() {
    let token = CancelToken::new();
    token.cancel();
    let backend = ScriptedStream {
        chunks: vec![chunk(0, "x")],
        terminal: ModelOutcome::CommittedSuccess {
            usage: Usage::default(),
        },
        cancel_at: None,
        token: token.clone(),
    };
    let result = stream(&backend, &request(), &token);
    assert!(result.chunks.is_empty());
    assert_eq!(result.terminal, ModelOutcome::Cancelled);
}

#[test]
fn cancel_mid_stream_keeps_prior_chunks_and_never_fabricates_success() {
    let token = CancelToken::new();
    let backend = ScriptedStream {
        chunks: vec![chunk(0, "he"), chunk(1, "llo")],
        terminal: ModelOutcome::CommittedSuccess {
            usage: Usage::default(),
        },
        cancel_at: Some(0),
        token: token.clone(),
    };
    let result = stream(&backend, &request(), &token);
    assert_eq!(result.chunks, vec![chunk(0, "he")]);
    assert_eq!(result.terminal, ModelOutcome::Cancelled);
}
