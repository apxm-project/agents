//! Correlated diagnostic signals that never replace canonical evidence.
//!
//! Structured logs, metrics, and traces may carry the same correlation ids as
//! owner evidence, but they remain `diagnostic_only`. When they disagree with
//! sealed usage lineage, evidence stays authoritative.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::effect::Usage;
use crate::lineage::InferenceUsageLineage;
use crate::target::InferenceTargetCommitment;
use apxm_program::grammar::is_digest;

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
    pub correlation_digest: String,
    pub commit_id: String,
    pub evidence_fact_ids: Vec<String>,
    pub request_digest: String,
    pub model_target_ref: String,
    pub model_target_digest: String,
    pub model_deployment_ref: String,
    pub exact_port_binding_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_commitment_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_cohort_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_port_contract_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_composition_digest: Option<String>,
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
    InvalidDigest(&'static str),
    InvalidAuthority,
    MissingEvidence,
    EvidenceMismatch,
    TargetCommitmentMismatch,
    DigestMismatch,
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
            Self::InvalidDigest(field) => {
                write!(f, "diagnostic field {field} is not a sha256 digest")
            }
            Self::InvalidAuthority => {
                write!(f, "diagnostic correlation cannot grant authority")
            }
            Self::MissingEvidence => {
                write!(f, "diagnostic correlation requires owner evidence ids")
            }
            Self::EvidenceMismatch => {
                write!(f, "diagnostic correlation does not match owner evidence")
            }
            Self::TargetCommitmentMismatch => {
                write!(f, "diagnostic target commitment does not match its fields")
            }
            Self::DigestMismatch => {
                write!(f, "diagnostic correlation digest does not match its facts")
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

impl DiagnosticCorrelation {
    /// Validate privacy bounds, committed-target identity, and the immutable
    /// digest used to deduplicate or replay the diagnostic projection.
    pub fn validate(&self) -> Result<(), DiagnosticError> {
        if self.schema_version != DIAGNOSTIC_CORRELATION_SCHEMA {
            return Err(DiagnosticError::EmptyField("schema_version"));
        }
        if self.authority != "diagnostic_only" {
            return Err(DiagnosticError::InvalidAuthority);
        }
        for (field, value) in [
            ("correlation_id", self.correlation_id.as_str()),
            ("commit_id", self.commit_id.as_str()),
            ("model_target_ref", self.model_target_ref.as_str()),
            ("model_deployment_ref", self.model_deployment_ref.as_str()),
        ] {
            validate_identifier(field, value)?;
        }
        for (field, value) in [
            ("request_digest", self.request_digest.as_str()),
            ("model_target_digest", self.model_target_digest.as_str()),
            (
                "exact_port_binding_digest",
                self.exact_port_binding_digest.as_str(),
            ),
            ("correlation_digest", self.correlation_digest.as_str()),
        ] {
            if !is_digest(value) {
                return Err(DiagnosticError::InvalidDigest(field));
            }
        }
        if self.evidence_fact_ids.is_empty() {
            return Err(DiagnosticError::MissingEvidence);
        }
        validate_refs("evidence_fact_ids", &self.evidence_fact_ids)?;
        validate_refs("log_refs", &self.log_refs)?;
        validate_refs("metric_refs", &self.metric_refs)?;
        validate_refs("trace_refs", &self.trace_refs)?;
        let commitment_fields = (
            self.target_commitment_digest.as_deref(),
            self.generation_cohort_digest.as_deref(),
            self.target_generation,
            self.target_port_contract_digest.as_deref(),
            self.target_composition_digest.as_deref(),
        );
        match commitment_fields {
            (Some(commit), Some(cohort), Some(generation), Some(port), Some(composition)) => {
                let commitment = InferenceTargetCommitment::commit(
                    &self.model_target_ref,
                    &self.model_target_digest,
                    &self.model_deployment_ref,
                    &self.exact_port_binding_digest,
                    port,
                    composition,
                    generation,
                )
                .map_err(|_| DiagnosticError::TargetCommitmentMismatch)?;
                if commitment.commit_digest != commit
                    || commitment.generation_cohort_digest != cohort
                {
                    return Err(DiagnosticError::TargetCommitmentMismatch);
                }
            }
            (None, None, None, None, None) => {}
            _ => return Err(DiagnosticError::TargetCommitmentMismatch),
        }
        let expected = diagnostic_digest(self);
        if expected != self.correlation_digest {
            return Err(DiagnosticError::DigestMismatch);
        }
        Ok(())
    }
}

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
            .is_none_or(|fact_id| {
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
    let mut diagnostic = DiagnosticCorrelation {
        schema_version: DIAGNOSTIC_CORRELATION_SCHEMA.to_string(),
        correlation_id: request.correlation_id,
        correlation_digest: String::new(),
        commit_id: request.commit_id,
        evidence_fact_ids: request.evidence_fact_ids,
        request_digest: request.lineage.request_digest.clone(),
        model_target_ref: request.lineage.model_target_ref.clone(),
        model_target_digest: request.lineage.model_target_digest.clone(),
        model_deployment_ref: request.lineage.model_deployment_ref.clone(),
        exact_port_binding_digest: request.lineage.exact_port_binding_digest.clone(),
        target_commitment_digest: request.lineage.target_commitment_digest.clone(),
        generation_cohort_digest: request.lineage.generation_cohort_digest.clone(),
        target_generation: request.lineage.target_generation,
        target_port_contract_digest: request.lineage.target_port_contract_digest.clone(),
        target_composition_digest: request.lineage.target_composition_digest.clone(),
        log_refs: request.log_refs,
        metric_refs: request.metric_refs,
        trace_refs: request.trace_refs,
        authority: "diagnostic_only".to_string(),
        agreement,
        claimed_input_tokens: request.claimed_usage.map(|usage| usage.input_tokens),
        claimed_output_tokens: request.claimed_usage.map(|usage| usage.output_tokens),
    };
    diagnostic.correlation_digest = diagnostic_digest(&diagnostic);
    diagnostic.validate()?;
    Ok(diagnostic)
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

fn validate_identifier(field: &'static str, value: &str) -> Result<(), DiagnosticError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
    {
        return Err(DiagnosticError::InvalidReference(field));
    }
    Ok(())
}

fn diagnostic_digest(diagnostic: &DiagnosticCorrelation) -> String {
    let mut hasher = Sha256::new();
    hasher.update(DIAGNOSTIC_CORRELATION_SCHEMA.as_bytes());
    let target_generation = diagnostic
        .target_generation
        .map(|generation| generation.to_string())
        .unwrap_or_default();
    let claimed_input_tokens = diagnostic
        .claimed_input_tokens
        .map(|value| value.to_string())
        .unwrap_or_default();
    let claimed_output_tokens = diagnostic
        .claimed_output_tokens
        .map(|value| value.to_string())
        .unwrap_or_default();
    for value in [
        diagnostic.correlation_id.as_str(),
        diagnostic.commit_id.as_str(),
        diagnostic.request_digest.as_str(),
        diagnostic.model_target_ref.as_str(),
        diagnostic.model_target_digest.as_str(),
        diagnostic.model_deployment_ref.as_str(),
        diagnostic.exact_port_binding_digest.as_str(),
        diagnostic.target_commitment_digest.as_deref().unwrap_or(""),
        diagnostic.generation_cohort_digest.as_deref().unwrap_or(""),
        target_generation.as_str(),
        diagnostic
            .target_port_contract_digest
            .as_deref()
            .unwrap_or(""),
        diagnostic
            .target_composition_digest
            .as_deref()
            .unwrap_or(""),
        diagnostic.authority.as_str(),
        agreement_name(diagnostic.agreement),
        claimed_input_tokens.as_str(),
        claimed_output_tokens.as_str(),
    ] {
        hasher.update(b"\0");
        hasher.update(value.as_bytes());
    }
    for refs in [
        &diagnostic.evidence_fact_ids,
        &diagnostic.log_refs,
        &diagnostic.metric_refs,
        &diagnostic.trace_refs,
    ] {
        hasher.update(b"\0");
        hasher.update(refs.len().to_string().as_bytes());
        for value in refs {
            hasher.update(b"\0");
            hasher.update(value.as_bytes());
        }
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn agreement_name(agreement: DiagnosticAgreement) -> &'static str {
    match agreement {
        DiagnosticAgreement::AgreesWithEvidence => "agrees_with_evidence",
        DiagnosticAgreement::DisagreesEvidenceAuthoritative => "disagrees_evidence_authoritative",
    }
}
