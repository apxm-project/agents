//! Closed, deterministic diagnostics for the Agent Program semantic surface.
//!
//! Every rejection carries one member of the closed [`DiagnosticCode`] set. The
//! set is exhaustive: there is no open "other" variant and no free-form error
//! channel. A [`Verdict`] orders its diagnostics deterministically so identical
//! input always yields identical output, byte for byte.

use std::cmp::Ordering;

/// The closed set of rejection reasons for graph, AIR, source-map, and artifact
/// verification. Adding a reason is a deliberate, reviewed change to this enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticCode {
    /// The value did not decode against its closed schema shape: an unknown
    /// field, an unknown enum member, a wrong `schema_version`, or a missing
    /// required field. This is the fail-closed decode boundary.
    SchemaViolation,
    /// A typed identifier did not match the contract identifier grammar.
    InvalidIdentifier,
    /// A digest was not a lowercase `sha256:` value of the exact length.
    InvalidDigest,
    /// Two semantic operations shared one `node_id`.
    DuplicateNodeId,
    /// Two structural regions shared one `region_id`.
    DuplicateRegionId,
    /// A source-map span referenced coordinates that are not a forward range.
    NonForwardSpan,
    /// An artifact requirement declared a scope other than `artifact_semantic`.
    RequirementScopeNotArtifactSemantic,
    /// An evidence sequence was not strictly monotonic by event sequence.
    NonMonotonicEvidence,
    /// An uncertain effect fact claimed a committed/success outcome.
    OutcomeUnknownClaimsSuccess,
    /// An atomic write set was not the exact canonical five-member set.
    NonAtomicWriteSet,
}

impl DiagnosticCode {
    /// The stable, machine-readable slug for this reason.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::SchemaViolation => "schema_violation",
            Self::InvalidIdentifier => "invalid_identifier",
            Self::InvalidDigest => "invalid_digest",
            Self::DuplicateNodeId => "duplicate_node_id",
            Self::DuplicateRegionId => "duplicate_region_id",
            Self::NonForwardSpan => "non_forward_span",
            Self::RequirementScopeNotArtifactSemantic => "requirement_scope_not_artifact_semantic",
            Self::NonMonotonicEvidence => "non_monotonic_evidence",
            Self::OutcomeUnknownClaimsSuccess => "outcome_unknown_claims_success",
            Self::NonAtomicWriteSet => "non_atomic_write_set",
        }
    }
}

/// One rejection: a closed code, the location it applies to, and a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The closed reason for the rejection.
    pub code: DiagnosticCode,
    /// The identifier or field the rejection is anchored to. Empty when the
    /// rejection applies to the document as a whole.
    pub location: String,
    /// A human-readable explanation. Never used for control flow.
    pub message: String,
}

impl Diagnostic {
    pub fn new(
        code: DiagnosticCode,
        location: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            location: location.into(),
            message: message.into(),
        }
    }

    fn sort_key(&self) -> (DiagnosticCode, &str, &str) {
        (self.code, self.location.as_str(), self.message.as_str())
    }
}

impl PartialOrd for Diagnostic {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Diagnostic {
    fn cmp(&self, other: &Self) -> Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

/// The outcome of verifying one document. Ordering of diagnostics is
/// deterministic and independent of discovery order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    diagnostics: Vec<Diagnostic>,
}

impl Verdict {
    /// An accepting verdict.
    #[must_use]
    pub fn accepted() -> Self {
        Self {
            diagnostics: Vec::new(),
        }
    }

    /// A rejecting verdict carrying exactly one diagnostic.
    #[must_use]
    pub fn rejected(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
        }
    }

    /// Record one diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Whether the document is accepted (no diagnostics).
    #[must_use]
    pub fn is_accepted(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// The diagnostics, in deterministic order.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Consume the verdict and return its ordered diagnostics.
    #[must_use]
    pub fn into_diagnostics(mut self) -> Vec<Diagnostic> {
        self.diagnostics.sort();
        self.diagnostics
    }

    /// Sort the diagnostics into their deterministic order.
    pub(crate) fn finish(mut self) -> Self {
        self.diagnostics.sort();
        self
    }
}

/// Reject a `serde` decode failure at the fail-closed schema boundary.
pub(crate) fn schema_violation(location: &str, error: &serde_json::Error) -> Verdict {
    Verdict::rejected(Diagnostic::new(
        DiagnosticCode::SchemaViolation,
        location,
        error.to_string(),
    ))
}
