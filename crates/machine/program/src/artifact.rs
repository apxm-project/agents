//! `apxm.executable-artifact.v1` and `apxm.port-requirement.v1` — closed
//! consumer types, a canonical codec, and the artifact/binding validation API.
//!
//! An executable artifact pins the AIR, source bundle, source map, and
//! entrypoints, and emits only `artifact_semantic` Port Requirements. It carries
//! no deployment implementation, Exact Port Binding, Runtime Profile,
//! root-admission fact, authority grant, endpoint, credential, or placement.
//! `deny_unknown_fields` rejects any such content at decode, and the validation
//! API rejects any requirement whose scope is not `artifact_semantic`.

use serde::{Deserialize, Serialize};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict, schema_violation};
use crate::grammar::{is_digest, is_identifier, is_schema_id};
use crate::source_map::SourceMap;

/// The single accepted `schema_version` for an executable artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactVersion {
    #[serde(rename = "apxm.executable-artifact.v1")]
    V1,
}

/// The single accepted integrity algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrityAlgorithm {
    #[serde(rename = "sha256")]
    Sha256,
}

/// The single accepted `schema_version` for a Port Requirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortRequirementVersion {
    #[serde(rename = "apxm.port-requirement.v1")]
    V1,
}

/// The closed Port Requirement source-scope set. A requirement never selects an
/// implementation; only `artifact_semantic` requirements may appear in an
/// artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortSourceScope {
    ArtifactSemantic,
    DeploymentInfrastructure,
    InvocationAuthority,
}

/// The closed semantic-owner set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SemanticOwner {
    Contracts,
    Coordinator,
    Agents,
    Server,
    Os,
    Auth,
    Studio,
    Plugin,
    #[serde(rename = "host-sdk")]
    HostSdk,
    Adapters,
    Vllm,
    Eval,
}

/// A schema-id plus content digest reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaDigestRef {
    pub schema_id: String,
    pub digest: String,
}

/// One typed Port Requirement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortRequirement {
    pub schema_version: PortRequirementVersion,
    pub typed_port_slot: String,
    pub required_port_contract: SchemaDigestRef,
    pub required_feature_set: Vec<String>,
    pub semantic_limits_digest: String,
    pub source_scope: PortSourceScope,
    pub source_owner: SemanticOwner,
    pub source_digest: String,
    pub requirement_digest: String,
}

/// One artifact entrypoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entrypoint {
    pub entrypoint: String,
    pub program_id: String,
    pub input_type_ref: String,
    pub output_type_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_type_ref: Option<String>,
}

/// A decoded executable artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableArtifact {
    pub schema_version: ArtifactVersion,
    pub artifact_digest: String,
    pub air_digest: String,
    pub source_bundle_digest: String,
    pub source_map: SourceMap,
    pub entrypoints: Vec<Entrypoint>,
    pub artifact_semantic_requirements: Vec<PortRequirement>,
    pub integrity_algorithm: IntegrityAlgorithm,
}

/// A codec error: an artifact could not be decoded or re-encoded.
#[derive(Debug)]
pub struct CodecError(pub serde_json::Error);

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "artifact codec error: {}", self.0)
    }
}

impl std::error::Error for CodecError {}

impl ExecutableArtifact {
    /// Decode an artifact from its canonical JSON bytes, failing closed on any
    /// unknown field or out-of-closure value.
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        serde_json::from_slice(bytes).map_err(CodecError)
    }

    /// Encode an artifact to canonical JSON bytes. Field order is fixed by the
    /// type, so the encoding is deterministic.
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        serde_json::to_vec(self).map_err(CodecError)
    }

    /// Validate a decoded artifact and its bindings. The closed shape is already
    /// guaranteed by decode; this enforces the abstraction rule — every emitted
    /// requirement is `artifact_semantic` — plus grammar and digest checks.
    #[must_use]
    pub fn validate(&self) -> Verdict {
        let mut verdict = Verdict::accepted();

        check_digest(&mut verdict, &self.artifact_digest, "artifact_digest");
        check_digest(&mut verdict, &self.air_digest, "air_digest");
        check_digest(&mut verdict, &self.source_bundle_digest, "source_bundle_digest");

        if self.entrypoints.is_empty() {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                "entrypoints",
                "an artifact declares at least one entrypoint",
            ));
        }
        for entry in &self.entrypoints {
            check_identifier(&mut verdict, &entry.entrypoint, "entrypoint");
            check_identifier(&mut verdict, &entry.program_id, "program_id");
        }

        for requirement in &self.artifact_semantic_requirements {
            let slot = requirement.typed_port_slot.as_str();
            if requirement.source_scope != PortSourceScope::ArtifactSemantic {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::RequirementScopeNotArtifactSemantic,
                    slot.to_string(),
                    "an artifact emits only artifact_semantic Port Requirements",
                ));
            }
            check_identifier(&mut verdict, slot, "typed_port_slot");
            if !is_schema_id(&requirement.required_port_contract.schema_id) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    slot.to_string(),
                    "required_port_contract schema_id is not a contract schema id",
                ));
            }
            check_digest(&mut verdict, &requirement.required_port_contract.digest, slot);
            check_digest(&mut verdict, &requirement.semantic_limits_digest, slot);
            check_digest(&mut verdict, &requirement.source_digest, slot);
            check_digest(&mut verdict, &requirement.requirement_digest, slot);
        }

        self.source_map.collect(&mut verdict);
        verdict.finish()
    }
}

fn check_identifier(verdict: &mut Verdict, value: &str, what: &str) {
    if !is_identifier(value) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::InvalidIdentifier,
            value.to_string(),
            format!("{what} is not a contract identifier"),
        ));
    }
}

fn check_digest(verdict: &mut Verdict, value: &str, location: &str) {
    if !is_digest(value) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::InvalidDigest,
            location.to_string(),
            "digest is not a lowercase sha256 value",
        ));
    }
}

/// Validate an artifact presented as JSON, failing closed on decode errors. An
/// Exact Port Binding, deployment field, or authority content in the document is
/// an unknown field and is rejected here; a non-`artifact_semantic` requirement
/// is rejected by [`ExecutableArtifact::validate`].
#[must_use]
pub fn validate_artifact_json(value: &serde_json::Value) -> Verdict {
    match serde_json::from_value::<ExecutableArtifact>(value.clone()) {
        Ok(artifact) => artifact.validate(),
        Err(error) => schema_violation("executable_artifact", &error),
    }
}
