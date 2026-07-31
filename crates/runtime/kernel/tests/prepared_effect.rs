//! Prepared-effect reducer conformance.
//!
//! Agents owns the `PreparedEffect`/`EffectState` transitions, what
//! `OutcomeUnknown` means, and the condition under which an effect result may
//! authorize a later activation. These cases pin exactly that and nothing about
//! the durable store or the provider protocol behind it.

use apxm_kernel::{
    EffectError, EffectId, EffectRecord, EffectState, EffectTransition, PreparedEffect,
    ProgramInstanceRef, ProgramInvocationRef,
};

fn effect_id() -> EffectId {
    EffectId::new("capability-effect.1").expect("effect identity")
}

fn prepared() -> PreparedEffect {
    PreparedEffect {
        effect_id: effect_id(),
        program_instance_ref: ProgramInstanceRef::new("instance.1"),
        program_invocation_ref: ProgramInvocationRef::new("invocation.1"),
        node_execution_id: "node-exec.1".into(),
        request_digest: format!("sha256:{}", "a".repeat(64)),
        exact_binding_ref: "binding.capability.1".into(),
        authority_decision_ref: "decision.1".into(),
        idempotency_contract_ref: "idempotency.1".into(),
    }
}

fn record() -> EffectRecord {
    EffectRecord::prepare(prepared())
}

/// Drive a record to `dispatch_started` under one claim.
fn dispatched() -> EffectRecord {
    let mut record = record();
    record.apply(EffectTransition::CommitWon).expect("commit won");
    record
        .apply(EffectTransition::ClaimedBeforeSend {
            epoch: 1,
            attempt: 1,
        })
        .expect("claim");
    record
        .apply(EffectTransition::DispatchStarted)
        .expect("dispatch started");
    record
}

#[test]
fn an_empty_effect_identity_is_refused() {
    assert_eq!(EffectId::new("   "), Err(EffectError::EmptyEffectId));
}

#[test]
fn a_prepared_effect_is_not_dispatchable_until_a_commit_wins() {
    let mut record = record();
    assert_eq!(record.state(), &EffectState::Prepared);
    // No claim can reach a merely prepared effect: the local activation has not
    // ended, so no byte may leave.
    let error = record
        .apply(EffectTransition::ClaimedBeforeSend {
            epoch: 1,
            attempt: 1,
        })
        .expect_err("claim before commit");
    assert!(matches!(error, EffectError::IllegalTransition { .. }));
    assert_eq!(record.state(), &EffectState::Prepared);

    record.apply(EffectTransition::CommitWon).expect("commit won");
    assert_eq!(record.state(), &EffectState::Dispatchable);
}

#[test]
fn a_claim_records_its_epoch_and_attempt() {
    let mut record = record();
    record.apply(EffectTransition::CommitWon).expect("commit won");
    record
        .apply(EffectTransition::ClaimedBeforeSend {
            epoch: 3,
            attempt: 7,
        })
        .expect("claim");
    assert_eq!(
        record.state(),
        &EffectState::ClaimedBeforeSend {
            epoch: 3,
            attempt: 7,
        }
    );
    assert_eq!(record.last_claim(), (3, 7));
}

#[test]
fn a_claim_that_does_not_advance_is_stale_and_changes_nothing() {
    let mut record = record();
    record.apply(EffectTransition::CommitWon).expect("commit won");
    record
        .apply(EffectTransition::ClaimedBeforeSend {
            epoch: 2,
            attempt: 5,
        })
        .expect("claim");
    record
        .apply(EffectTransition::ProvenNotSentRetryable)
        .expect("released with proof");
    assert_eq!(record.state(), &EffectState::Dispatchable);

    // An old runner replays its claim under the superseded epoch.
    let error = record
        .apply(EffectTransition::ClaimedBeforeSend {
            epoch: 1,
            attempt: 9,
        })
        .expect_err("stale epoch");
    assert_eq!(
        error,
        EffectError::StaleClaim {
            effect_id: effect_id(),
            last_epoch: 2,
            last_attempt: 5,
            presented_epoch: 1,
            presented_attempt: 9,
        }
    );
    assert_eq!(record.state(), &EffectState::Dispatchable);

    // The same epoch without advancing the attempt is equally stale.
    assert!(matches!(
        record.apply(EffectTransition::ClaimedBeforeSend {
            epoch: 2,
            attempt: 5,
        }),
        Err(EffectError::StaleClaim { .. })
    ));

    // A reclaim under a higher epoch is admitted.
    record
        .apply(EffectTransition::ClaimedBeforeSend {
            epoch: 3,
            attempt: 1,
        })
        .expect("reclaim under a new epoch");
}

#[test]
fn revocation_and_cancellation_before_send_are_terminal_and_authorize_the_next_activation() {
    for transition in [
        EffectTransition::AuthorityRevoked,
        EffectTransition::CancelledBeforeSend,
    ] {
        let mut record = record();
        record.apply(EffectTransition::CommitWon).expect("commit won");
        record.apply(transition).expect("pre-send terminal");
        assert!(record.state().is_terminal());
        // No byte left, so the outcome is settled and downstream work is safe.
        assert!(record.authorizes_next_activation());
    }
}

#[test]
fn a_dispatched_effect_with_an_unknown_outcome_authorizes_nothing() {
    let mut record = dispatched();
    record
        .apply(EffectTransition::ProviderOutcomeUnknown)
        .expect("unknown outcome");
    assert_eq!(record.state(), &EffectState::OutcomeUnknown);
    assert!(!record.state().is_resolved());
    assert!(
        !record.authorizes_next_activation(),
        "an ambiguous send must not release downstream Program work"
    );

    record
        .apply(EffectTransition::ReconciliationStarted)
        .expect("reconciling");
    assert_eq!(record.state(), &EffectState::Reconciling);
    assert!(!record.authorizes_next_activation());
}

#[test]
fn reconciliation_resolves_to_committed_and_then_authorizes_the_next_activation() {
    let mut record = dispatched();
    record
        .apply(EffectTransition::ProviderOutcomeUnknown)
        .expect("unknown");
    record
        .apply(EffectTransition::ReconciliationStarted)
        .expect("reconciling");
    record
        .apply(EffectTransition::ProviderCommitted)
        .expect("resolved committed");
    assert_eq!(record.state(), &EffectState::Committed);
    assert!(record.state().is_terminal());
    assert!(record.authorizes_next_activation());
}

#[test]
fn reconciliation_that_stays_unknown_can_only_end_by_an_authorized_manual_decision() {
    let mut record = dispatched();
    record
        .apply(EffectTransition::ProviderOutcomeUnknown)
        .expect("unknown");
    record
        .apply(EffectTransition::ReconciliationStarted)
        .expect("reconciling");
    // Reconciliation may legally observe unknown again.
    record
        .apply(EffectTransition::ProviderOutcomeUnknown)
        .expect("still unknown");
    assert_eq!(record.state(), &EffectState::OutcomeUnknown);
    record
        .apply(EffectTransition::ReconciliationStarted)
        .expect("reconciling again");
    record
        .apply(EffectTransition::ManualTerminalDecision)
        .expect("authorized manual terminal");
    assert_eq!(record.state(), &EffectState::TerminalManualDecision);
    assert!(record.authorizes_next_activation());
}

#[test]
fn an_unknown_outcome_never_enters_ordinary_retry() {
    let mut record = dispatched();
    record
        .apply(EffectTransition::ProviderOutcomeUnknown)
        .expect("unknown");
    // The one re-entry into dispatchable work is an explicit new attempt after
    // proven-not-applied. An unknown outcome may not take it: replacement work
    // reconciles rather than resends.
    let error = record
        .apply(EffectTransition::ExplicitNewAttempt)
        .expect_err("no resend from unknown");
    assert!(matches!(error, EffectError::IllegalTransition { .. }));
    assert_eq!(record.state(), &EffectState::OutcomeUnknown);
}

#[test]
fn proven_not_applied_is_resolved_but_may_be_retried_under_the_same_effect_identity() {
    let mut record = dispatched();
    record
        .apply(EffectTransition::ProviderProvenNotApplied)
        .expect("proven not applied");
    assert_eq!(record.state(), &EffectState::ProvenNotApplied);
    assert!(record.state().is_resolved());
    assert!(!record.state().is_terminal());

    record
        .apply(EffectTransition::ExplicitNewAttempt)
        .expect("explicit new attempt");
    assert_eq!(record.state(), &EffectState::Dispatchable);
    // The identity is stable across the new attempt.
    assert_eq!(record.effect_id(), &effect_id());
}

#[test]
fn a_claim_released_with_proof_may_be_terminal_instead_of_retryable() {
    let mut record = record();
    record.apply(EffectTransition::CommitWon).expect("commit won");
    record
        .apply(EffectTransition::ClaimedBeforeSend {
            epoch: 1,
            attempt: 1,
        })
        .expect("claim");
    record
        .apply(EffectTransition::ProvenNotSentTerminal)
        .expect("terminal, nothing sent");
    assert_eq!(record.state(), &EffectState::TerminalNotSent);
    assert!(record.state().is_terminal());
    assert!(record.authorizes_next_activation());
}

#[test]
fn a_terminal_effect_accepts_no_further_transition() {
    let mut record = dispatched();
    record
        .apply(EffectTransition::ProviderCommitted)
        .expect("committed");
    for transition in [
        EffectTransition::CommitWon,
        EffectTransition::ClaimedBeforeSend {
            epoch: 9,
            attempt: 9,
        },
        EffectTransition::DispatchStarted,
        EffectTransition::ProviderProvenNotApplied,
        EffectTransition::ProviderOutcomeUnknown,
        EffectTransition::ExplicitNewAttempt,
        EffectTransition::ManualTerminalDecision,
    ] {
        assert!(
            record.apply(transition).is_err(),
            "a committed effect must refuse every further transition"
        );
        assert_eq!(record.state(), &EffectState::Committed);
    }
}

#[test]
fn a_transition_naming_a_different_effect_is_refused() {
    let mut record = record();
    let other = EffectId::new("capability-effect.2").expect("other identity");
    let error = record
        .apply_for(&other, EffectTransition::CommitWon)
        .expect_err("identity mismatch");
    assert_eq!(
        error,
        EffectError::EffectIdentityMismatch {
            expected: effect_id(),
            presented: other,
        }
    );
    assert_eq!(record.state(), &EffectState::Prepared);

    record
        .apply_for(&effect_id(), EffectTransition::CommitWon)
        .expect("matching identity");
    assert_eq!(record.state(), &EffectState::Dispatchable);
}

#[test]
fn a_prepared_effect_carries_its_exact_binding_authority_and_idempotency_contract() {
    let record = record();
    let prepared = record.prepared();
    assert_eq!(prepared.exact_binding_ref, "binding.capability.1");
    assert_eq!(prepared.authority_decision_ref, "decision.1");
    assert_eq!(prepared.idempotency_contract_ref, "idempotency.1");
    // Preparation is bound to the exact invocation that produced it.
    assert_eq!(prepared.program_invocation_ref.as_str(), "invocation.1");
}

#[test]
fn a_prepared_effect_rejects_an_unknown_wire_field() {
    let mut value = serde_json::to_value(prepared()).expect("serialize prepared effect");
    value
        .as_object_mut()
        .expect("object")
        .insert("endpoint_url".into(), serde_json::json!("https://provider"));
    assert!(
        serde_json::from_value::<PreparedEffect>(value).is_err(),
        "a provider endpoint is not part of the Agents-owned preparation"
    );
}
