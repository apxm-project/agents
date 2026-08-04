//! Immutable inference usage lineage.
//!
//! Owner-recorded native usage, timing, and typed-error facts for one effect
//! attempt. Downstream callers may verify membership but must not recompute or
//! replace sealed usage.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::effect::{ErrorCategory, TypedError, Usage};
use apxm_program::grammar::is_digest;

/// Schema identity for the frozen usage-lineage contract.
pub const INFERENCE_USAGE_LINEAGE_SCHEMA: &str = "apxm.inference-usage-lineage.v1";

/// One sealed usage lineage record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceUsageLineage {
    pub schema_version: String,
    pub lineage_id: String,
    pub effect_id: String,
    pub attempt_index: u32,
    pub request_digest: String,
    pub model_target_ref: String,
    pub model_target_digest: String,
    pub model_deployment_ref: String,
    pub exact_port_binding_digest: String,
    pub native_input_tokens: u64,
    pub native_output_tokens: u64,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_fact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typed_error: Option<TypedError>,
    pub sealed: bool,
}

/// Why lineage cannot be minted, sealed, or mutated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineageError {
    EmptyField(&'static str),
    InvalidDigest(&'static str),
    AlreadySealed,
    NotSealed,
    DigestMismatch,
    RecomputeRejected,
    CommitMismatch,
}

impl std::fmt::Display for LineageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "usage lineage field {field} is empty"),
            Self::InvalidDigest(field) => {
                write!(f, "usage lineage field {field} is not a sha256 digest")
            }
            Self::AlreadySealed => write!(f, "usage lineage is already sealed"),
            Self::NotSealed => write!(f, "usage lineage is not sealed"),
            Self::DigestMismatch => write!(f, "usage lineage digest does not match its facts"),
            Self::RecomputeRejected => {
                write!(f, "downstream usage recomputation is rejected")
            }
            Self::CommitMismatch => {
                write!(f, "usage lineage commit binding does not match evidence")
            }
        }
    }
}

impl std::error::Error for LineageError {}

impl InferenceUsageLineage {
    /// Mint and seal one immutable lineage record from owner-observed facts.
    pub fn seal(
        effect_id: impl Into<String>,
        attempt_index: u32,
        request_digest: impl Into<String>,
        model_target_ref: impl Into<String>,
        model_target_digest: impl Into<String>,
        model_deployment_ref: impl Into<String>,
        exact_port_binding_digest: impl Into<String>,
        usage: Usage,
        duration_ms: u64,
        typed_error: Option<TypedError>,
    ) -> Result<Self, LineageError> {
        let effect_id = effect_id.into();
        let request_digest = request_digest.into();
        let model_target_ref = model_target_ref.into();
        let model_target_digest = model_target_digest.into();
        let model_deployment_ref = model_deployment_ref.into();
        let exact_port_binding_digest = exact_port_binding_digest.into();
        if effect_id.trim().is_empty() {
            return Err(LineageError::EmptyField("effect_id"));
        }
        if !is_digest(&request_digest) {
            return Err(LineageError::InvalidDigest("request_digest"));
        }
        if model_target_ref.trim().is_empty() {
            return Err(LineageError::EmptyField("model_target_ref"));
        }
        if !is_digest(&model_target_digest) {
            return Err(LineageError::InvalidDigest("model_target_digest"));
        }
        if model_deployment_ref.trim().is_empty() {
            return Err(LineageError::EmptyField("model_deployment_ref"));
        }
        if !is_digest(&exact_port_binding_digest) {
            return Err(LineageError::InvalidDigest("exact_port_binding_digest"));
        }
        let lineage_id = lineage_digest(
            &effect_id,
            attempt_index,
            &request_digest,
            &model_target_ref,
            &model_target_digest,
            &model_deployment_ref,
            &exact_port_binding_digest,
            usage.input_tokens,
            usage.output_tokens,
            duration_ms,
            typed_error.as_ref(),
        );
        let lineage = Self {
            schema_version: INFERENCE_USAGE_LINEAGE_SCHEMA.to_string(),
            lineage_id,
            effect_id,
            attempt_index,
            request_digest,
            model_target_ref,
            model_target_digest,
            model_deployment_ref,
            exact_port_binding_digest,
            native_input_tokens: usage.input_tokens,
            native_output_tokens: usage.output_tokens,
            duration_ms,
            evidence_fact_id: None,
            commit_id: None,
            typed_error,
            sealed: true,
        };
        lineage.validate()?;
        Ok(lineage)
    }

    /// Verify that the sealed record still matches its owner-observed facts.
    pub fn validate(&self) -> Result<(), LineageError> {
        if self.schema_version != INFERENCE_USAGE_LINEAGE_SCHEMA {
            return Err(LineageError::EmptyField("schema_version"));
        }
        if !self.sealed {
            return Err(LineageError::NotSealed);
        }
        if self.effect_id.trim().is_empty() {
            return Err(LineageError::EmptyField("effect_id"));
        }
        if !is_digest(&self.request_digest) {
            return Err(LineageError::InvalidDigest("request_digest"));
        }
        if self.model_target_ref.trim().is_empty() {
            return Err(LineageError::EmptyField("model_target_ref"));
        }
        if !is_digest(&self.model_target_digest) {
            return Err(LineageError::InvalidDigest("model_target_digest"));
        }
        if self.model_deployment_ref.trim().is_empty() {
            return Err(LineageError::EmptyField("model_deployment_ref"));
        }
        if !is_digest(&self.exact_port_binding_digest) {
            return Err(LineageError::InvalidDigest("exact_port_binding_digest"));
        }
        if !is_digest(&self.lineage_id) {
            return Err(LineageError::InvalidDigest("lineage_id"));
        }
        if let Some(error) = &self.typed_error {
            if error.code.trim().is_empty() {
                return Err(LineageError::EmptyField("typed_error.code"));
            }
            if error.message.trim().is_empty() {
                return Err(LineageError::EmptyField("typed_error.message"));
            }
        }
        let expected = lineage_digest(
            &self.effect_id,
            self.attempt_index,
            &self.request_digest,
            &self.model_target_ref,
            &self.model_target_digest,
            &self.model_deployment_ref,
            &self.exact_port_binding_digest,
            self.native_input_tokens,
            self.native_output_tokens,
            self.duration_ms,
            self.typed_error.as_ref(),
        );
        if expected != self.lineage_id {
            return Err(LineageError::DigestMismatch);
        }
        Ok(())
    }

    /// Bind lineage to the committed evidence coordinates without altering usage.
    pub fn bind_evidence(
        &mut self,
        evidence_fact_id: impl Into<String>,
        commit_id: impl Into<String>,
    ) -> Result<(), LineageError> {
        self.validate()?;
        if self.commit_id.is_some() || self.evidence_fact_id.is_some() {
            return Err(LineageError::AlreadySealed);
        }
        let evidence_fact_id = evidence_fact_id.into();
        let commit_id = commit_id.into();
        if evidence_fact_id.trim().is_empty() {
            return Err(LineageError::EmptyField("evidence_fact_id"));
        }
        if commit_id.trim().is_empty() {
            return Err(LineageError::EmptyField("commit_id"));
        }
        self.evidence_fact_id = Some(evidence_fact_id);
        self.commit_id = Some(commit_id);
        Ok(())
    }

    /// Reject any attempt to replace sealed native usage from a downstream caller.
    pub fn reject_recompute(&self, claimed: Usage) -> Result<(), LineageError> {
        self.validate()?;
        if claimed.input_tokens != self.native_input_tokens
            || claimed.output_tokens != self.native_output_tokens
        {
            return Err(LineageError::RecomputeRejected);
        }
        Ok(())
    }

    /// Verify that an exporter-presented usage claim matches sealed owner lineage.
    pub fn authorize_exporter_claim(
        &self,
        commit_id: &str,
        claimed: Usage,
    ) -> Result<(), LineageError> {
        self.validate()?;
        if self.commit_id.as_deref() != Some(commit_id) {
            return Err(LineageError::CommitMismatch);
        }
        self.reject_recompute(claimed)
    }
}

/// Map a closed error category for lineage serialization stability.
#[must_use]
pub fn lineage_error_category(category: ErrorCategory) -> &'static str {
    match category {
        ErrorCategory::Validation => "validation",
        ErrorCategory::Admission => "admission",
        ErrorCategory::Authority => "authority",
        ErrorCategory::Configuration => "configuration",
        ErrorCategory::Unavailable => "unavailable",
        ErrorCategory::Conflict => "conflict",
        ErrorCategory::OutcomeUnknown => "outcome_unknown",
        ErrorCategory::Internal => "internal",
    }
}

#[allow(clippy::too_many_arguments)]
fn lineage_digest(
    effect_id: &str,
    attempt_index: u32,
    request_digest: &str,
    model_target_ref: &str,
    model_target_digest: &str,
    model_deployment_ref: &str,
    exact_port_binding_digest: &str,
    input_tokens: u64,
    output_tokens: u64,
    duration_ms: u64,
    typed_error: Option<&TypedError>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(INFERENCE_USAGE_LINEAGE_SCHEMA.as_bytes());
    hasher.update(b"\0");
    hasher.update(effect_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(attempt_index.to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(request_digest.as_bytes());
    hasher.update(b"\0");
    hasher.update(model_target_ref.as_bytes());
    hasher.update(b"\0");
    hasher.update(model_target_digest.as_bytes());
    hasher.update(b"\0");
    hasher.update(model_deployment_ref.as_bytes());
    hasher.update(b"\0");
    hasher.update(exact_port_binding_digest.as_bytes());
    hasher.update(b"\0");
    hasher.update(input_tokens.to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(output_tokens.to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(duration_ms.to_string().as_bytes());
    if let Some(error) = typed_error {
        hasher.update(b"\0");
        hasher.update(lineage_error_category(error.category).as_bytes());
        hasher.update(b"\0");
        hasher.update(error.code.as_bytes());
        hasher.update(b"\0");
        hasher.update(error.message.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}
