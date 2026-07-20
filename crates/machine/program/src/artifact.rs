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
use sha2::{Digest, Sha256};

use crate::air::{AirModule, SemanticOpKind};
use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict, schema_violation};
use crate::grammar::{is_digest, is_identifier, is_schema_id};
use crate::source_map::SourceMap;

/// The canonical model-target Port contract an artifact's `model.call` binds to.
const MODEL_TARGET_PORT_CONTRACT: &str = "apxm.model-target.v1";

/// Lowercase `sha256:<hex>` digest of `bytes`, matching the contract Digest
/// grammar (`^sha256:[0-9a-f]{64}$`).
fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

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
    /// Derive the canonical `apxm.executable-artifact.v1` for a compiled
    /// `apxm.air.v1` module: the digest-bound AIR + source bundle, the AIR's
    /// source map, a single `main` entrypoint, and one `artifact_semantic` Port
    /// Requirement per distinct `model.call` model target (bound to the
    /// `apxm.model-target.v1` port contract). The artifact carries no deployment
    /// implementation, Exact Port Binding, or authority — only the semantic
    /// requirements the program declares. `artifact_digest` binds every other
    /// field, so the encoding is a stable content address.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError`] when the module or artifact cannot be serialized
    /// for digesting.
    pub fn from_air(air: &AirModule) -> Result<Self, CodecError> {
        let air_bytes = serde_json::to_vec(air).map_err(CodecError)?;
        let air_digest = sha256_digest(&air_bytes);
        let source_map_bytes = serde_json::to_vec(&air.source_map).map_err(CodecError)?;
        let source_bundle_digest = sha256_digest(&source_map_bytes);

        let entrypoints = vec![Entrypoint {
            entrypoint: "main".to_string(),
            program_id: "program.main".to_string(),
            input_type_ref: "input".to_string(),
            output_type_ref: "output".to_string(),
            context_type_ref: None,
        }];

        let mut seen = std::collections::BTreeSet::new();
        let mut artifact_semantic_requirements = Vec::new();
        for op in &air.semantic_operations {
            if op.op != SemanticOpKind::ModelCall {
                continue;
            }
            let Some(target) = op
                .operands
                .as_ref()
                .and_then(|operands| operands.get("model_target_ref"))
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            if seen.insert(target.to_string()) {
                artifact_semantic_requirements.push(model_target_requirement(target));
            }
        }

        let mut artifact = Self {
            schema_version: ArtifactVersion::V1,
            artifact_digest: String::new(),
            air_digest,
            source_bundle_digest,
            source_map: air.source_map.clone(),
            entrypoints,
            artifact_semantic_requirements,
            integrity_algorithm: IntegrityAlgorithm::Sha256,
        };
        // The content address binds every other field; hash with the digest
        // field blanked so it is reproducible from the artifact content.
        let content = serde_json::to_vec(&artifact).map_err(CodecError)?;
        artifact.artifact_digest = sha256_digest(&content);
        Ok(artifact)
    }

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
        check_digest(
            &mut verdict,
            &self.source_bundle_digest,
            "source_bundle_digest",
        );

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
            check_digest(
                &mut verdict,
                &requirement.required_port_contract.digest,
                slot,
            );
            check_digest(&mut verdict, &requirement.semantic_limits_digest, slot);
            check_digest(&mut verdict, &requirement.source_digest, slot);
            check_digest(&mut verdict, &requirement.requirement_digest, slot);
        }

        self.source_map.collect(&mut verdict);
        verdict.finish()
    }
}

/// Build the `artifact_semantic` Port Requirement a `model.call` declares for
/// its model target, bound to the `apxm.model-target.v1` port contract. It
/// selects no implementation and carries no authority — only the typed slot and
/// content digests.
fn model_target_requirement(target: &str) -> PortRequirement {
    let contract = SchemaDigestRef {
        schema_id: MODEL_TARGET_PORT_CONTRACT.to_string(),
        digest: sha256_digest(MODEL_TARGET_PORT_CONTRACT.as_bytes()),
    };
    let source_digest = sha256_digest(target.as_bytes());
    let semantic_limits_digest = sha256_digest(b"");
    let requirement_digest = sha256_digest(
        format!(
            "{target}|{}|{}|{source_digest}",
            contract.schema_id, contract.digest
        )
        .as_bytes(),
    );
    PortRequirement {
        schema_version: PortRequirementVersion::V1,
        typed_port_slot: target.to_string(),
        required_port_contract: contract,
        required_feature_set: Vec::new(),
        semantic_limits_digest,
        source_scope: PortSourceScope::ArtifactSemantic,
        source_owner: SemanticOwner::Agents,
        source_digest,
        requirement_digest,
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

#[cfg(test)]
mod from_air_tests {
    use super::*;

    fn air(model_targets: &[&str]) -> AirModule {
        let ops: Vec<serde_json::Value> = model_targets
            .iter()
            .enumerate()
            .map(|(i, target)| {
                serde_json::json!({
                    "node_id": format!("node.model.{i}"),
                    "op": "model.call",
                    "operands": { "model_target_ref": target }
                })
            })
            .chain(std::iter::once(serde_json::json!({
                "node_id": "node.await",
                "op": "await.event",
                "operands": { "event_selector": "session-input" }
            })))
            .collect();
        serde_json::from_value(serde_json::json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": ops,
            "structural_ir": [{ "region_id": "region.return", "kind": "return" }],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": []
            }
        }))
        .expect("valid canonical AIR")
    }

    #[test]
    fn from_air_produces_a_validating_digest_bound_artifact() {
        let artifact = ExecutableArtifact::from_air(&air(&["model.default"])).expect("from_air");
        assert_eq!(artifact.schema_version, ArtifactVersion::V1);
        assert_eq!(artifact.integrity_algorithm, IntegrityAlgorithm::Sha256);
        assert!(is_digest(&artifact.artifact_digest));
        assert!(is_digest(&artifact.air_digest));
        assert!(is_digest(&artifact.source_bundle_digest));
        assert_eq!(artifact.entrypoints.len(), 1);
        // The verdict is the real conformance consumer: validate accepts it.
        assert!(
            artifact.validate().is_accepted(),
            "from_air artifact must validate: {:?}",
            artifact.validate()
        );
    }

    #[test]
    fn from_air_emits_one_artifact_semantic_requirement_per_distinct_model_target() {
        let artifact =
            ExecutableArtifact::from_air(&air(&["model.default", "model.default", "model.fast"]))
                .expect("from_air");
        assert_eq!(artifact.artifact_semantic_requirements.len(), 2);
        for requirement in &artifact.artifact_semantic_requirements {
            assert_eq!(requirement.source_scope, PortSourceScope::ArtifactSemantic);
            assert_eq!(requirement.source_owner, SemanticOwner::Agents);
            assert_eq!(
                requirement.required_port_contract.schema_id,
                MODEL_TARGET_PORT_CONTRACT
            );
        }
        let slots: Vec<&str> = artifact
            .artifact_semantic_requirements
            .iter()
            .map(|r| r.typed_port_slot.as_str())
            .collect();
        assert!(slots.contains(&"model.default") && slots.contains(&"model.fast"));
    }

    #[test]
    fn from_air_is_deterministic_and_content_addressed() {
        let a = ExecutableArtifact::from_air(&air(&["model.default"])).expect("from_air");
        let b = ExecutableArtifact::from_air(&air(&["model.default"])).expect("from_air");
        assert_eq!(a, b, "from_air is a deterministic content address");
        // A different program yields a different artifact digest.
        let c = ExecutableArtifact::from_air(&air(&["model.other"])).expect("from_air");
        assert_ne!(a.artifact_digest, c.artifact_digest);
    }

    #[test]
    fn model_free_air_has_no_requirements_but_still_validates() {
        let artifact = ExecutableArtifact::from_air(&air(&[])).expect("from_air");
        assert!(artifact.artifact_semantic_requirements.is_empty());
        assert!(artifact.validate().is_accepted());
    }
}
