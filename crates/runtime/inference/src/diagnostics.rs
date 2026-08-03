//! Correlated diagnostic signals that never replace canonical evidence.
//!
//! Structured logs, metrics, and traces may carry the same correlation ids as
//! owner evidence, but they remain `diagnostic_only`. When they disagree with
//! sealed usage lineage, evidence stays authoritative.

use serde::{Deserialize, Serialize};

use crate::effect::Usage;
use crate::lineage::InferenceUsageLineage;

/// Schema identity for diagnostic correlation.
pub const DIAGNOSTIC_CORRELATION_SCHEMA: &str = "apxm.diagnostic-correlation.v1";

/// Closed agreement vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticAgreement {
    AgreesWithEvidence,
    DisagreesEvidenceAuthoritative,
}

/// One correlated diagnostic bundle. Never authoritative.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticCorrelation {
    pub schema_version: String,
    pub correlation_id: String,
    pub commit_id: String,
    pub evidence_fact_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metric_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace_refs: Vec<String>,
    pub authority: String,
    pub agreement: DiagnosticAgreement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_output_tokens: Option<u64>,
}

/// Bounded, low-cardinality metric label set for inference diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedMetricLabels {
    pub driver_id: String,
    pub availability: String,
    pub outcome: String,
}

impl BoundedMetricLabels {
    /// Reject high-cardinality labels. Only the closed low-cardinality
    /// availability/outcome vocabularies and a short static driver id are
    /// permitted — never prompts, completions, credentials, or free-text.
    pub fn validate(&self) -> Result<(), DiagnosticError> {
        if self.driver_id.trim().is_empty() {
            return Err(DiagnosticError::EmptyField("driver_id"));
        }
        if self.driver_id.len() > 64 || self.driver_id.contains('/') || self.driver_id.contains(' ')
        {
            return Err(DiagnosticError::CardinalityExceeded("driver_id"));
        }
        let allowed_availability = ["available", "unavailable"];
        if !allowed_availability.contains(&self.availability.as_str()) {
            return Err(DiagnosticError::CardinalityExceeded("availability"));
        }
        let allowed_outcome = [
            "committed_success",
            "typed_failure",
            "cancelled",
            "model_outcome_unknown",
            "exporter_loss",
        ];
        if !allowed_outcome.contains(&self.outcome.as_str()) {
            return Err(DiagnosticError::CardinalityExceeded("outcome"));
        }
        Ok(())
    }
}

/// Diagnostic correlation failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticError {
    EmptyField(&'static str),
    CardinalityExceeded(&'static str),
    MissingEvidence,
    EvidenceMismatch,
    InvalidReference(&'static str),
    LineageIntegrity,
}

impl std::fmt::Display for DiagnosticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "diagnostic field {field} is empty"),
            Self::CardinalityExceeded(field) => {
                write!(f, "diagnostic label {field} exceeds bounded cardinality")
            }
            Self::MissingEvidence => {
                write!(f, "diagnostic correlation requires owner evidence ids")
            }
            Self::EvidenceMismatch => {
                write!(f, "diagnostic correlation does not match owner evidence")
            }
            Self::InvalidReference(field) => {
                write!(
                    f,
                    "diagnostic reference {field} is not a bounded identifier"
                )
            }
            Self::LineageIntegrity => {
                write!(f, "diagnostic correlation received invalid owner lineage")
            }
        }
    }
}

impl std::error::Error for DiagnosticError {}

/// Inputs for one diagnostic correlation against sealed owner lineage.
pub struct CorrelateDiagnosticsRequest<'a> {
    pub correlation_id: String,
    pub commit_id: String,
    pub evidence_fact_ids: Vec<String>,
    pub lineage: &'a InferenceUsageLineage,
    pub claimed_usage: Option<Usage>,
    pub log_refs: Vec<String>,
    pub metric_refs: Vec<String>,
    pub trace_refs: Vec<String>,
}

/// Correlate diagnostics to owner evidence and record disagreement without
/// granting diagnostic authority.
pub fn correlate_diagnostics(
    request: CorrelateDiagnosticsRequest<'_>,
) -> Result<DiagnosticCorrelation, DiagnosticError> {
    if request.correlation_id.trim().is_empty() {
        return Err(DiagnosticError::EmptyField("correlation_id"));
    }
    if request.commit_id.trim().is_empty() {
        return Err(DiagnosticError::EmptyField("commit_id"));
    }
    if request.evidence_fact_ids.is_empty() {
        return Err(DiagnosticError::MissingEvidence);
    }
    request
        .lineage
        .validate()
        .map_err(|_| DiagnosticError::LineageIntegrity)?;
    if request.lineage.commit_id.as_deref() != Some(request.commit_id.as_str())
        || request
            .lineage
            .evidence_fact_id
            .as_ref()
            .map_or(true, |fact_id| {
                !request
                    .evidence_fact_ids
                    .iter()
                    .any(|candidate| candidate == fact_id)
            })
    {
        return Err(DiagnosticError::EvidenceMismatch);
    }
    validate_refs("evidence_fact_ids", &request.evidence_fact_ids)?;
    validate_refs("log_refs", &request.log_refs)?;
    validate_refs("metric_refs", &request.metric_refs)?;
    validate_refs("trace_refs", &request.trace_refs)?;
    let agreement = match request.claimed_usage {
        Some(claimed)
            if claimed.input_tokens != request.lineage.native_input_tokens
                || claimed.output_tokens != request.lineage.native_output_tokens =>
        {
            DiagnosticAgreement::DisagreesEvidenceAuthoritative
        }
        _ => DiagnosticAgreement::AgreesWithEvidence,
    };
    Ok(DiagnosticCorrelation {
        schema_version: DIAGNOSTIC_CORRELATION_SCHEMA.to_string(),
        correlation_id: request.correlation_id,
        commit_id: request.commit_id,
        evidence_fact_ids: request.evidence_fact_ids,
        log_refs: request.log_refs,
        metric_refs: request.metric_refs,
        trace_refs: request.trace_refs,
        authority: "diagnostic_only".to_string(),
        agreement,
        claimed_input_tokens: request.claimed_usage.map(|usage| usage.input_tokens),
        claimed_output_tokens: request.claimed_usage.map(|usage| usage.output_tokens),
    })
}

/// Resolve usage authority: owner lineage always wins over diagnostic claims.
#[must_use]
pub fn authoritative_usage(
    lineage: &InferenceUsageLineage,
    diagnostic: &DiagnosticCorrelation,
) -> Usage {
    let _ = diagnostic;
    Usage {
        input_tokens: lineage.native_input_tokens,
        output_tokens: lineage.native_output_tokens,
    }
}

fn validate_refs(field: &'static str, refs: &[String]) -> Result<(), DiagnosticError> {
    if refs.len() > 64 {
        return Err(DiagnosticError::CardinalityExceeded(field));
    }
    if refs.iter().any(|value| {
        value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
    }) {
        return Err(DiagnosticError::InvalidReference(field));
    }
    Ok(())
}
