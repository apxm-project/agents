//! The prepared-effect reducer — the Agents share of the three-layer effect
//! boundary.
//!
//! A local node prepares an exact effect but never sends external bytes. The
//! winning Execution Commit that persists the preparation is what makes durable
//! effect work dispatchable, so the local activation has already ended before
//! any byte can leave.
//!
//! This module owns exactly one thing: the closed [`EffectState`] transition
//! set, what [`EffectState::OutcomeUnknown`] means, and the condition under
//! which an effect result may authorize a later activation. It owns no durable
//! store, no attempt record, no reconciliation schedule, and no provider
//! protocol. Server owns durable managed work, attempt and result records,
//! reconciliation scheduling, and publication of the exact next activation. The
//! Adapter owns provider send, status, idempotency, and cancellation. A
//! standalone durability implementation must reproduce this same transition
//! contract; it does not get to invent a different one.
//!
//! ```text
//! Prepared
//!   -- winning Execution Commit --> Dispatchable
//!
//! Dispatchable
//!   -- claim while authority remains valid --> ClaimedBeforeSend(epoch, attempt)
//!   -- authority revoked / cancelled --------> RevokedBeforeSend | CancelledBeforeSend
//!
//! ClaimedBeforeSend
//!   -- durable pre-send transition --> DispatchStarted(attempt)
//!   -- proven no bytes could leave --> Dispatchable | TerminalNotSent
//!
//! DispatchStarted
//!   --> Committed | ProvenNotApplied | OutcomeUnknown
//!
//! OutcomeUnknown --> Reconciling
//!
//! Reconciling
//!   --> Committed | ProvenNotApplied | OutcomeUnknown | TerminalManualDecision
//!
//! ProvenNotApplied -- explicit new attempt, same effect id --> Dispatchable
//! ```
//!
//! Commit uncertainty and effect uncertainty stay distinct. An ambiguous send
//! becomes [`EffectState::OutcomeUnknown`] and reconciles; it never enters
//! ordinary retry, and replacement work reconciles rather than resends.

use serde::{Deserialize, Serialize};

use crate::commit::{ProgramInstanceRef, ProgramInvocationRef};

/// The stable identity of one prepared effect.
///
/// It is minted once with the preparation and survives every claim, attempt,
/// reclaim, and reconciliation. A new attempt reuses the same identity; it never
/// mints a second one.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EffectId(String);

impl EffectId {
    /// Construct one non-empty effect identity.
    ///
    /// # Errors
    ///
    /// Returns [`EffectError::EmptyEffectId`] for a blank identity.
    pub fn new(value: impl Into<String>) -> Result<Self, EffectError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(EffectError::EmptyEffectId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for EffectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The exact effect one local node prepared, as persisted by the winning
/// Execution Commit.
///
/// Every member is required at the type level: an effect cannot become
/// dispatchable while its adapter binding, authority decision, or idempotency
/// contract is still unknown. It carries no credential, no endpoint, no broker
/// subject, and no provider payload — those belong to the Adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedEffect {
    pub effect_id: EffectId,
    pub program_instance_ref: ProgramInstanceRef,
    pub program_invocation_ref: ProgramInvocationRef,
    /// The node occurrence whose local execution prepared this effect.
    pub node_execution_id: String,
    /// Digest of the exact request identity this effect will send.
    pub request_digest: String,
    /// The exact adapter port binding admitted for this effect. No discovery,
    /// ranking, or fallback selects it.
    pub exact_binding_ref: String,
    /// The Auth decision that authorized this effect for its Acting Principal.
    pub authority_decision_ref: String,
    /// The provider idempotency and reconciliation contract this effect commits
    /// to before any byte leaves.
    pub idempotency_contract_ref: String,
}

/// The closed prepared-effect state set. No other state exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    /// Prepared locally and not yet made durable by a winning commit.
    Prepared,
    /// A winning Execution Commit made this effect durable work.
    Dispatchable,
    /// Claimed by a durable effect worker while the authority was still valid.
    ClaimedBeforeSend { epoch: u64, attempt: u64 },
    /// Authority was revoked before any byte could leave.
    RevokedBeforeSend,
    /// The effect was cancelled before any byte could leave.
    CancelledBeforeSend,
    /// A durable pre-send transition was recorded; bytes may now be in flight.
    DispatchStarted { attempt: u64 },
    /// The provider applied the effect exactly once.
    Committed,
    /// The provider provably did not apply the effect.
    ProvenNotApplied,
    /// Whether the provider applied the effect is unknown. This is not a
    /// failure and not an ordinary retry: it is uncertainty that must be
    /// reconciled.
    OutcomeUnknown,
    /// Reconciliation is resolving an unknown outcome.
    Reconciling,
    /// Terminal, with proof that no byte left.
    TerminalNotSent,
    /// Terminal by an authorized manual decision after reconciliation could not
    /// resolve the outcome.
    TerminalManualDecision,
}

impl EffectState {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Dispatchable => "dispatchable",
            Self::ClaimedBeforeSend { .. } => "claimed_before_send",
            Self::RevokedBeforeSend => "revoked_before_send",
            Self::CancelledBeforeSend => "cancelled_before_send",
            Self::DispatchStarted { .. } => "dispatch_started",
            Self::Committed => "committed",
            Self::ProvenNotApplied => "proven_not_applied",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::Reconciling => "reconciling",
            Self::TerminalNotSent => "terminal_not_sent",
            Self::TerminalManualDecision => "terminal_manual_decision",
        }
    }

    /// True once the effect's outcome is settled.
    ///
    /// [`EffectState::OutcomeUnknown`] and [`EffectState::Reconciling`] are
    /// deliberately not resolved: uncertainty is a state to reconcile, never a
    /// result to act on.
    #[must_use]
    pub const fn is_resolved(&self) -> bool {
        matches!(
            self,
            Self::Committed
                | Self::ProvenNotApplied
                | Self::RevokedBeforeSend
                | Self::CancelledBeforeSend
                | Self::TerminalNotSent
                | Self::TerminalManualDecision
        )
    }

    /// Whether this effect result may authorize a later activation.
    ///
    /// Reconciliation creates a later activation only after the effect reaches a
    /// committed, proven-not-applied, or authorized manual-terminal outcome; the
    /// proven-not-sent pre-send terminals qualify on the same ground, because no
    /// byte left. An unresolved effect authorizes nothing, so an ambiguous send
    /// can never silently release downstream Program work.
    #[must_use]
    pub const fn authorizes_next_activation(&self) -> bool {
        self.is_resolved()
    }

    /// True when no further transition is legal.
    ///
    /// [`EffectState::ProvenNotApplied`] is resolved but not terminal: an
    /// explicit new attempt may return it to [`EffectState::Dispatchable`] under
    /// the same effect identity.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Committed
                | Self::RevokedBeforeSend
                | Self::CancelledBeforeSend
                | Self::TerminalNotSent
                | Self::TerminalManualDecision
        )
    }
}

impl std::fmt::Display for EffectState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// The closed set of inputs the reducer accepts. Each names a fact that already
/// happened durably; the reducer decides what that fact means, never whether it
/// should happen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectTransition {
    /// The Execution Commit that persisted this preparation won.
    CommitWon,
    /// A durable effect worker claimed the effect while its authority was valid.
    ClaimedBeforeSend { epoch: u64, attempt: u64 },
    /// Auth revoked the authority before any byte could leave.
    AuthorityRevoked,
    /// The effect was cancelled before any byte could leave.
    CancelledBeforeSend,
    /// The durable pre-send transition was recorded.
    DispatchStarted,
    /// The claim was released with proof that no byte left, and the effect may
    /// be claimed again.
    ProvenNotSentRetryable,
    /// The claim was released with proof that no byte left, terminally.
    ProvenNotSentTerminal,
    /// The provider applied the effect.
    ProviderCommitted,
    /// The provider provably did not apply the effect.
    ProviderProvenNotApplied,
    /// Whether the provider applied the effect is unknown.
    ProviderOutcomeUnknown,
    /// Reconciliation started for an unknown outcome.
    ReconciliationStarted,
    /// An authorized operator terminated an unresolvable reconciliation.
    ManualTerminalDecision,
    /// An explicit new attempt was authorized under the same effect identity.
    ExplicitNewAttempt,
}

impl EffectTransition {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::CommitWon => "commit_won",
            Self::ClaimedBeforeSend { .. } => "claimed_before_send",
            Self::AuthorityRevoked => "authority_revoked",
            Self::CancelledBeforeSend => "cancelled_before_send",
            Self::DispatchStarted => "dispatch_started",
            Self::ProvenNotSentRetryable => "proven_not_sent_retryable",
            Self::ProvenNotSentTerminal => "proven_not_sent_terminal",
            Self::ProviderCommitted => "provider_committed",
            Self::ProviderProvenNotApplied => "provider_proven_not_applied",
            Self::ProviderOutcomeUnknown => "provider_outcome_unknown",
            Self::ReconciliationStarted => "reconciliation_started",
            Self::ManualTerminalDecision => "manual_terminal_decision",
            Self::ExplicitNewAttempt => "explicit_new_attempt",
        }
    }
}

impl std::fmt::Display for EffectTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Why an effect operation was refused. Every variant fails closed: the effect
/// keeps its current state and authorizes nothing new.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectError {
    EmptyEffectId,
    /// The transition is not in the closed set for the current state.
    IllegalTransition {
        effect_id: EffectId,
        from: EffectState,
        transition: EffectTransition,
    },
    /// A claim did not strictly advance the attempt under the current epoch, or
    /// went backwards in epoch. A reclaim increments the epoch so an old worker
    /// cannot commit.
    StaleClaim {
        effect_id: EffectId,
        last_epoch: u64,
        last_attempt: u64,
        presented_epoch: u64,
        presented_attempt: u64,
    },
    /// A transition was applied to a different effect than the one it names.
    EffectIdentityMismatch {
        expected: EffectId,
        presented: EffectId,
    },
}

impl std::fmt::Display for EffectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyEffectId => f.write_str("effect identity must not be empty"),
            Self::IllegalTransition {
                effect_id,
                from,
                transition,
            } => write!(
                f,
                "effect {effect_id} cannot apply {transition} from {from}"
            ),
            Self::StaleClaim {
                effect_id,
                last_epoch,
                last_attempt,
                presented_epoch,
                presented_attempt,
            } => write!(
                f,
                "stale claim ({presented_epoch}, {presented_attempt}) for effect {effect_id}; \
                 the live claim is ({last_epoch}, {last_attempt})"
            ),
            Self::EffectIdentityMismatch {
                expected,
                presented,
            } => write!(
                f,
                "transition names effect {presented} but the record is effect {expected}"
            ),
        }
    }
}

impl std::error::Error for EffectError {}

/// One prepared effect and its current state, reduced by the closed transition
/// set.
///
/// The record holds no durable handle: it is a pure decision function over facts
/// that a durable owner has already made authoritative.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectRecord {
    prepared: PreparedEffect,
    state: EffectState,
    last_epoch: u64,
    last_attempt: u64,
}

impl EffectRecord {
    /// Open a record for one locally prepared effect. It starts
    /// [`EffectState::Prepared`]: no byte can leave until a commit wins.
    #[must_use]
    pub const fn prepare(prepared: PreparedEffect) -> Self {
        Self {
            prepared,
            state: EffectState::Prepared,
            last_epoch: 0,
            last_attempt: 0,
        }
    }

    #[must_use]
    pub const fn prepared(&self) -> &PreparedEffect {
        &self.prepared
    }

    #[must_use]
    pub const fn state(&self) -> &EffectState {
        &self.state
    }

    #[must_use]
    pub fn effect_id(&self) -> &EffectId {
        &self.prepared.effect_id
    }

    /// The highest claim epoch and attempt this effect has observed.
    #[must_use]
    pub const fn last_claim(&self) -> (u64, u64) {
        (self.last_epoch, self.last_attempt)
    }

    /// Whether this effect's current result may authorize a later activation.
    #[must_use]
    pub const fn authorizes_next_activation(&self) -> bool {
        self.state.authorizes_next_activation()
    }

    /// Apply one durable fact, checked against the same effect identity.
    ///
    /// # Errors
    ///
    /// Returns [`EffectError::EffectIdentityMismatch`] when `effect_id` is not
    /// this record's identity, and otherwise whatever [`EffectRecord::apply`]
    /// returns.
    pub fn apply_for(
        &mut self,
        effect_id: &EffectId,
        transition: EffectTransition,
    ) -> Result<&EffectState, EffectError> {
        if effect_id != &self.prepared.effect_id {
            return Err(EffectError::EffectIdentityMismatch {
                expected: self.prepared.effect_id.clone(),
                presented: effect_id.clone(),
            });
        }
        self.apply(transition)
    }

    /// Apply one durable fact and return the resulting state.
    ///
    /// # Errors
    ///
    /// Returns [`EffectError::IllegalTransition`] when the transition is not in
    /// the closed set for the current state, and [`EffectError::StaleClaim`]
    /// when a claim does not strictly advance under a non-decreasing epoch.
    pub fn apply(&mut self, transition: EffectTransition) -> Result<&EffectState, EffectError> {
        let next = self.next_state(&transition)?;
        if let EffectTransition::ClaimedBeforeSend { epoch, attempt } = transition {
            self.last_epoch = epoch;
            self.last_attempt = attempt;
        }
        self.state = next;
        Ok(&self.state)
    }

    // The transition table is written one `(state, transition)` pair per line so
    // the closed set is readable against the contract's state machine. Merging
    // the pairs that happen to share a destination today would hide which
    // transitions are actually admitted and make a future divergence silent.
    #[allow(clippy::match_same_arms)]
    fn next_state(&self, transition: &EffectTransition) -> Result<EffectState, EffectError> {
        use EffectState as S;
        use EffectTransition as T;

        let next = match (&self.state, transition) {
            (S::Prepared, T::CommitWon) => S::Dispatchable,

            (S::Dispatchable, T::ClaimedBeforeSend { epoch, attempt }) => {
                self.check_claim(*epoch, *attempt)?;
                S::ClaimedBeforeSend {
                    epoch: *epoch,
                    attempt: *attempt,
                }
            }
            (S::Dispatchable, T::AuthorityRevoked) => S::RevokedBeforeSend,
            (S::Dispatchable, T::CancelledBeforeSend) => S::CancelledBeforeSend,

            (S::ClaimedBeforeSend { attempt, .. }, T::DispatchStarted) => {
                S::DispatchStarted { attempt: *attempt }
            }
            (S::ClaimedBeforeSend { .. }, T::ProvenNotSentRetryable) => S::Dispatchable,
            (S::ClaimedBeforeSend { .. }, T::ProvenNotSentTerminal) => S::TerminalNotSent,

            (S::DispatchStarted { .. }, T::ProviderCommitted) => S::Committed,
            (S::DispatchStarted { .. }, T::ProviderProvenNotApplied) => S::ProvenNotApplied,
            (S::DispatchStarted { .. }, T::ProviderOutcomeUnknown) => S::OutcomeUnknown,

            (S::OutcomeUnknown, T::ReconciliationStarted) => S::Reconciling,

            (S::Reconciling, T::ProviderCommitted) => S::Committed,
            (S::Reconciling, T::ProviderProvenNotApplied) => S::ProvenNotApplied,
            (S::Reconciling, T::ProviderOutcomeUnknown) => S::OutcomeUnknown,
            (S::Reconciling, T::ManualTerminalDecision) => S::TerminalManualDecision,

            // The one re-entry into dispatchable work: an explicit new attempt
            // under the same effect identity, never an automatic resend.
            (S::ProvenNotApplied, T::ExplicitNewAttempt) => S::Dispatchable,

            _ => {
                return Err(EffectError::IllegalTransition {
                    effect_id: self.prepared.effect_id.clone(),
                    from: self.state.clone(),
                    transition: transition.clone(),
                });
            }
        };
        Ok(next)
    }

    /// A claim must not go backwards in epoch, and within one epoch it must
    /// strictly advance the attempt. Reclaim increments the epoch so a runner
    /// holding an old claim can no longer commit.
    fn check_claim(&self, epoch: u64, attempt: u64) -> Result<(), EffectError> {
        let advances =
            epoch > self.last_epoch || (epoch == self.last_epoch && attempt > self.last_attempt);
        if advances {
            return Ok(());
        }
        Err(EffectError::StaleClaim {
            effect_id: self.prepared.effect_id.clone(),
            last_epoch: self.last_epoch,
            last_attempt: self.last_attempt,
            presented_epoch: epoch,
            presented_attempt: attempt,
        })
    }
}
