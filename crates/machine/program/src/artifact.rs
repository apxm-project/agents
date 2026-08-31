//! `apxm.executable-artifact` and `apxm.port-requirement.v1` — closed
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
use crate::frontend_graph::FrontendGraph;
use crate::grammar::{is_digest, is_identifier, is_schema_id};
use crate::lower::frontend_graph_to_air;
use crate::source_map::SourceMap;
use crate::{EXECUTION_LINEAGE_COMPILER_IDENTITY, execution_lineage_ref};

/// The canonical model-target Port contract an artifact's `model.call` binds to.
///
/// Public because the runtime admission boundary maps an artifact requirement
/// back onto the Port slot that must be admitted for it, and that mapping must
/// read the same string this module writes.
pub const MODEL_TARGET_PORT_CONTRACT: &str = "apxm.model-target";

/// The canonical Capability Port contract an artifact's `capability.invoke`
/// binds to. Same ownership rule as [`MODEL_TARGET_PORT_CONTRACT`].
pub const CAPABILITY_PORT_CONTRACT: &str = "apxm.capability-invocation";
const CAPABILITY_PORT_CONTRACT_DESCRIPTOR: &[u8] = include_bytes!(
    "../../../../contracts/port-contracts/apxm.capability-invocation.port-contract.json"
);

/// Lowercase `sha256:<hex>` digest of `bytes`, matching the contract Digest
/// grammar (`^sha256:[0-9a-f]{64}$`).
fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Digest-bound source bundle content lowered from a FrontendGraph.
///
/// The bundle pins program definitions, imports, static Hook bindings and handler
/// refs, and declared model/Capability requirements. It is not executable AIR.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBundle {
    pub program_definitions: Vec<crate::frontend_graph::ProgramDefinition>,
    pub imported_program_refs: Vec<crate::frontend_graph::ImportedProgramRef>,
    pub hook_bindings: Vec<crate::frontend_graph::HookBinding>,
    pub capability_requirements: Vec<crate::frontend_graph::CapabilityRequirement>,
    pub model_requirements: Vec<crate::frontend_graph::ModelRequirement>,
    /// Declared Agent Skills. An inline skill's instructions are carried here
    /// verbatim, which is what makes `source_bundle_digest` their integrity
    /// anchor: editing the body changes the artifact digest through this
    /// bundle, exactly as the contract says it does.
    pub skill_requirements: Vec<crate::frontend_graph::SkillRequirement>,
}

impl SourceBundle {
    /// Build the canonical source bundle for a verified FrontendGraph.
    #[must_use]
    pub fn from_graph(graph: &FrontendGraph) -> Self {
        Self {
            program_definitions: graph.program_definitions.clone(),
            imported_program_refs: graph.imported_program_refs.clone(),
            hook_bindings: graph.hook_bindings.clone(),
            capability_requirements: graph.capability_requirements.clone(),
            model_requirements: graph.model_requirements.clone(),
            skill_requirements: graph.skill_requirements.clone(),
        }
    }

    /// Canonical JSON bytes for digesting.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError`] when serialization fails.
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        serde_json::to_vec(self).map_err(CodecError)
    }

    /// Content address of the canonical source bundle.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError`] when serialization fails.
    pub fn digest(&self) -> Result<String, CodecError> {
        Ok(sha256_digest(&self.encode()?))
    }
}

/// The single accepted `schema_version` for an executable artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactVersion {
    #[serde(rename = "apxm.executable-artifact")]
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

impl PortSourceScope {
    /// Every source scope, so a drift guard reads the closure off the enum
    /// instead of restating it.
    pub const ALL: &'static [Self] = &[
        Self::ArtifactSemantic,
        Self::DeploymentInfrastructure,
        Self::InvocationAuthority,
    ];
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
    pub air: AirModule,
    pub source_bundle_digest: String,
    /// Opaque commitment to the exact source bundle, canonical AIR, and
    /// compiler identity used to produce this artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_lineage_ref: Option<String>,
    /// Exact static Hook bindings and handler identities executed by the runtime.
    #[serde(default)]
    pub hook_bindings: Vec<crate::frontend_graph::HookBinding>,
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
    /// Compute the content address of this artifact envelope.
    ///
    /// `artifact_digest` is a commitment to the complete envelope, including
    /// the AIR, source bundle commitment, source map, entrypoints, Hooks and
    /// semantic requirements. The digest field itself is blanked while
    /// computing the commitment so the value is not self-referential.
    pub fn canonical_digest(&self) -> Result<String, CodecError> {
        let mut unsigned = self.clone();
        unsigned.artifact_digest.clear();
        let content = serde_json::to_vec(&unsigned).map_err(CodecError)?;
        Ok(sha256_digest(&content))
    }

    /// Decode and verify the exact envelope the Runtime Service is allowed to
    /// execute. This is intentionally stricter than [`Self::validate`]: the
    /// latter is also used by schema vectors whose arbitrary digest values
    /// exercise structural rules, while an execution boundary must verify the
    /// digest and compiler lineage bound to the bytes it received.
    pub fn decode_for_execution(bytes: &[u8], expected_digest: &str) -> Result<Self, String> {
        let artifact = Self::decode(bytes).map_err(|error| error.to_string())?;
        let canonical_bytes = artifact.encode().map_err(|error| error.to_string())?;
        if bytes != canonical_bytes {
            return Err("noncanonical_artifact_encoding".to_owned());
        }
        if artifact.artifact_digest != expected_digest {
            return Err("artifact_digest_mismatch".to_owned());
        }
        if artifact.integrity_algorithm != IntegrityAlgorithm::Sha256 {
            return Err("unsupported_integrity_algorithm".to_owned());
        }
        let computed = artifact
            .canonical_digest()
            .map_err(|error| error.to_string())?;
        if computed != expected_digest {
            return Err("artifact_digest_mismatch".to_owned());
        }
        let verdict = artifact.validate();
        if !verdict.is_accepted() {
            return Err("invalid_artifact".to_owned());
        }
        let Some(lineage) = artifact.execution_lineage_ref.as_deref() else {
            return Err("missing_execution_lineage".to_owned());
        };
        let expected_lineage = execution_lineage_ref(
            &artifact.source_bundle_digest,
            &artifact.air_digest,
            EXECUTION_LINEAGE_COMPILER_IDENTITY,
        );
        if lineage != expected_lineage {
            return Err("execution_lineage_mismatch".to_owned());
        }
        Ok(artifact)
    }

    /// Derive the canonical executable artifact for a verified FrontendGraph.
    ///
    /// Lowers structural AIR, digest-binds the source bundle (programs, imports,
    /// Hook bindings/handlers, requirements), copies entrypoints and the source
    /// map, and emits only `artifact_semantic` Port Requirements.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError`] on serialization failure. Lowering and graph
    /// verification failures surface as [`ArtifactBuildError`].
    pub fn from_frontend_graph(graph: &FrontendGraph) -> Result<Self, ArtifactBuildError> {
        let air = frontend_graph_to_air(graph).map_err(ArtifactBuildError::Lowering)?;
        Self::from_graph_and_air(graph, &air)
    }

    /// Bind a lowered AIR module and its originating FrontendGraph into an artifact.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError`] when the module or artifact cannot be serialized
    /// for digesting.
    pub fn from_graph_and_air(
        graph: &FrontendGraph,
        air: &AirModule,
    ) -> Result<Self, ArtifactBuildError> {
        let bundle = SourceBundle::from_graph(graph);
        let source_bundle_digest = bundle.digest().map_err(ArtifactBuildError::Codec)?;
        let air_bytes = serde_json::to_vec(air)
            .map_err(|error| ArtifactBuildError::Codec(CodecError(error)))?;
        let air_digest = sha256_digest(&air_bytes);

        let entrypoints = graph
            .program_definitions
            .iter()
            .map(entrypoint_from_definition)
            .collect();

        reconcile_requirements_with_air(graph, air)?;

        let mut artifact_semantic_requirements = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for requirement in &graph.model_requirements {
            if seen.insert(requirement.model_target_ref.clone()) {
                artifact_semantic_requirements
                    .push(model_target_requirement(&requirement.model_target_ref));
            }
        }
        for requirement in &graph.capability_requirements {
            if seen.insert(requirement.capability_ref.clone()) {
                artifact_semantic_requirements
                    .push(capability_requirement(&requirement.capability_ref));
            }
        }

        let mut artifact = Self {
            schema_version: ArtifactVersion::V1,
            artifact_digest: String::new(),
            air_digest: air_digest.clone(),
            air: air.clone(),
            source_bundle_digest: source_bundle_digest.clone(),
            execution_lineage_ref: Some(execution_lineage_ref(
                &source_bundle_digest,
                &air_digest,
                EXECUTION_LINEAGE_COMPILER_IDENTITY,
            )),
            // The AIR's sequence, not the authoring sequence: `hook_execution_order`
            // places each Hook body, so AIR order is the schedule. The authored
            // order is still bound verbatim by the source bundle above, and each
            // binding carries its own `declaration_order`, so nothing is lost.
            hook_bindings: air_hook_bindings(air),
            source_map: air.source_map.clone(),
            entrypoints,
            artifact_semantic_requirements,
            integrity_algorithm: IntegrityAlgorithm::Sha256,
        };
        artifact.artifact_digest = artifact
            .canonical_digest()
            .map_err(ArtifactBuildError::Codec)?;
        // The producer runs the same validation the consumer runs. Without this
        // the `typed_port_slot` grammar and requirement-scope rules only ever
        // executed against an artifact someone had already written to disk.
        let verdict = artifact.validate();
        if !verdict.is_accepted() {
            return Err(ArtifactBuildError::Validation(verdict));
        }
        Ok(artifact)
    }

    /// Derive the canonical `apxm.executable-artifact` for a compiled
    /// `apxm.air` module when no FrontendGraph is available.
    ///
    /// On this path the module is the whole authority: requirements are inferred
    /// from the embedded typed operand slots and Hook bindings are read off the
    /// regions that carry them. Prefer [`Self::from_frontend_graph`] for complete
    /// source-bundle and requirement binding.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactBuildError::Codec`] when the module or artifact cannot
    /// be serialized for digesting, and [`ArtifactBuildError::Validation`] when
    /// the derived artifact does not satisfy [`Self::validate`].
    pub fn from_air(air: &AirModule) -> Result<Self, ArtifactBuildError> {
        let codec = |error| ArtifactBuildError::Codec(CodecError(error));
        let air_bytes = serde_json::to_vec(air).map_err(codec)?;
        let air_digest = sha256_digest(&air_bytes);
        let source_map_bytes = serde_json::to_vec(&air.source_map).map_err(codec)?;
        let source_bundle_digest = sha256_digest(&source_map_bytes);

        let entrypoints = vec![Entrypoint {
            entrypoint: "main".to_string(),
            program_id: "program.main".to_string(),
            input_type_ref: "input".to_string(),
            output_type_ref: "output".to_string(),
            context_type_ref: None,
        }];

        let artifact_semantic_requirements = air_semantic_requirements(air);

        let mut artifact = Self {
            schema_version: ArtifactVersion::V1,
            artifact_digest: String::new(),
            air_digest: air_digest.clone(),
            air: air.clone(),
            source_bundle_digest: source_bundle_digest.clone(),
            execution_lineage_ref: Some(execution_lineage_ref(
                &source_bundle_digest,
                &air_digest,
                EXECUTION_LINEAGE_COMPILER_IDENTITY,
            )),
            // Read from the AIR, not hardcoded empty. An empty vector here was
            // the original inert-Hook bug through a second door: the artifact
            // would seal a digest declaring no Hooks over an AIR that carries
            // them, and validation would then have to choose which copy to
            // believe. With no graph in hand the AIR is the only authority.
            hook_bindings: air_hook_bindings(air),
            source_map: air.source_map.clone(),
            entrypoints,
            artifact_semantic_requirements,
            integrity_algorithm: IntegrityAlgorithm::Sha256,
        };
        artifact.artifact_digest = artifact
            .canonical_digest()
            .map_err(ArtifactBuildError::Codec)?;
        // Same producer-side gate `from_graph_and_air` runs: a module whose
        // operand slots, source map, or Hooks do not satisfy the artifact
        // contract does not become a digest-sealed artifact.
        let verdict = artifact.validate();
        if !verdict.is_accepted() {
            return Err(ArtifactBuildError::Validation(verdict));
        }
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
        match serde_json::to_vec(&self.air) {
            Ok(air_bytes) if sha256_digest(&air_bytes) == self.air_digest => {}
            Ok(air_bytes) => verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                "air_digest",
                format!(
                    "embedded AIR bytes match air_digest: computed {} but artifact declares {}",
                    sha256_digest(&air_bytes),
                    self.air_digest
                ),
            )),
            Err(error) => verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                "air",
                format!("embedded AIR serializes canonically: {error}"),
            )),
        }
        reconcile_hook_bindings(&mut verdict, &self.hook_bindings, &self.air);
        if self.source_map != self.air.source_map {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                "source_map",
                "artifact source map matches embedded AIR source map",
            ));
        }
        check_digest(
            &mut verdict,
            &self.source_bundle_digest,
            "source_bundle_digest",
        );
        if let Some(lineage) = &self.execution_lineage_ref {
            check_digest(&mut verdict, lineage, "execution_lineage_ref");
            let expected = execution_lineage_ref(
                &self.source_bundle_digest,
                &self.air_digest,
                EXECUTION_LINEAGE_COMPILER_IDENTITY,
            );
            if lineage != &expected {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    "execution_lineage_ref",
                    "execution lineage reference does not match source, AIR, and compiler identity",
                ));
            }
        }

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

/// Compile serialized `apxm.frontend-graph` into one complete executable
/// artifact containing the exact structural AIR the artifact digest binds.
pub fn compile_frontend_graph_artifact_json(graph_json: &str) -> Result<String, String> {
    let graph: FrontendGraph =
        serde_json::from_str(graph_json).map_err(|error| error.to_string())?;
    let artifact =
        ExecutableArtifact::from_frontend_graph(&graph).map_err(|error| error.to_string())?;
    serde_json::to_string(&artifact).map_err(|error| error.to_string())
}

/// Failure to derive an artifact from a FrontendGraph.
#[derive(Debug)]
pub enum ArtifactBuildError {
    Lowering(Verdict),
    /// The graph's declared requirements and the lowered AIR's operands do not
    /// name the same set of models and Capabilities.
    Requirements(Verdict),
    /// The derived artifact does not satisfy the artifact validation contract.
    Validation(Verdict),
    Codec(CodecError),
}

impl std::fmt::Display for ArtifactBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lowering(verdict) => write!(f, "artifact lowering failed: {verdict:?}"),
            Self::Requirements(verdict) => write!(
                f,
                "artifact requirements do not reconcile with lowered AIR: {verdict:?}"
            ),
            Self::Validation(verdict) => {
                write!(f, "derived artifact failed validation: {verdict:?}")
            }
            Self::Codec(error) => write!(f, "{error}"),
        }
    }
}

/// The Hook bindings a compiled AIR module carries.
///
/// Every Hook rides on the region holding its captured body, and artifact
/// validation refuses any artifact whose declared `hook_bindings` differ from
/// these, so an execution boundary holding only AIR reads the same Hooks the
/// artifact declares. Exposed so that derivation exists exactly once, rather
/// than every caller rediscovering which structural nodes carry a binding.
#[must_use]
pub fn air_hook_bindings(air: &AirModule) -> Vec<crate::frontend_graph::HookBinding> {
    air.structural_ir
        .iter()
        .filter_map(|node| node.hook.clone())
        .collect()
}

/// The `artifact_semantic` Port Requirements a compiled AIR module states.
///
/// One requirement per distinct model target and per distinct Capability the
/// module's typed operand slots name. Exposed so an admission boundary can hold
/// a module's requirements against the Port bindings a Runtime Profile admits
/// without first materializing an artifact, and so that derivation exists
/// exactly once.
#[must_use]
pub fn air_semantic_requirements(air: &AirModule) -> Vec<PortRequirement> {
    // The target is carried by the typed operand slot (AIS operand catalogue);
    // the untyped operand bag is retired. Requirements keep first-appearance
    // order so an artifact digest is a function of the module, not of a
    // container's iteration order.
    let mut seen = std::collections::BTreeSet::new();
    let mut requirements = Vec::new();
    for target in ordered_operand_refs(air, SemanticOpKind::ModelCall, apxm_ais::SLOT_MODEL_REF) {
        if seen.insert(target.clone()) {
            requirements.push(model_target_requirement(&target));
        }
    }
    for capability_ref in ordered_operand_refs(
        air,
        SemanticOpKind::CapabilityInvoke,
        apxm_ais::SLOT_CAPABILITY_REF,
    ) {
        if seen.insert(capability_ref.clone()) {
            requirements.push(capability_requirement(&capability_ref));
        }
    }
    requirements
}

/// Every reference one semantic operand slot names, in module order.
fn ordered_operand_refs(air: &AirModule, op: SemanticOpKind, slot: &str) -> Vec<String> {
    air.semantic_operations
        .iter()
        .filter(|operation| operation.op == op)
        .filter_map(|operation| {
            operation
                .operands
                .iter()
                .find(|operand| operand.slot == slot)
                .map(|operand| operand.value_id.clone())
        })
        .collect()
}

/// The exact set of references one semantic operand slot names across a module.
fn air_operand_refs(
    air: &AirModule,
    op: SemanticOpKind,
    slot: &str,
) -> std::collections::BTreeSet<String> {
    ordered_operand_refs(air, op, slot).into_iter().collect()
}

/// Hold an artifact's `hook_bindings` and its own AIR to one story.
///
/// The two used to be independent copies: lowering kept four of a binding's ten
/// fields and the artifact re-attached the rest straight from the graph, so a
/// binding could name a scope, a handler, or a return mode that the executable
/// structure did not carry, and nothing would notice. Every Hook the AIR holds
/// is now the binding the artifact declares, field for field, once per
/// `hook_id`, and in the same order.
fn reconcile_hook_bindings(
    verdict: &mut Verdict,
    hook_bindings: &[crate::frontend_graph::HookBinding],
    air: &AirModule,
) {
    let carried: Vec<&crate::frontend_graph::HookBinding> = air
        .structural_ir
        .iter()
        .filter_map(|node| node.hook.as_ref())
        .collect();

    let mut declared_once = std::collections::BTreeSet::new();
    for binding in hook_bindings {
        if !declared_once.insert(binding.hook_id.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                binding.hook_id.clone(),
                "an artifact declares the same hook_id more than once",
            ));
        }
        match carried
            .iter()
            .find(|candidate| candidate.hook_id == binding.hook_id)
        {
            Some(candidate) if **candidate == *binding => {}
            Some(_) => verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                binding.hook_id.clone(),
                "declared Hook binding differs from the binding its own AIR carries",
            )),
            None => verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                binding.hook_id.clone(),
                "declared Hook binding has no captured body region in the artifact's AIR",
            )),
        }
    }
    for binding in &carried {
        if !hook_bindings
            .iter()
            .any(|declared| declared.hook_id == binding.hook_id)
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                binding.hook_id.clone(),
                "artifact AIR carries a Hook the artifact does not declare",
            ));
        }
    }

    // Order, which the two loops above cannot see: they match by `hook_id`, so a
    // declared list naming the same Hooks in a different sequence used to pass
    // while this comment claimed otherwise. AIR order is the schedule
    // (`lower::hook_execution_order` places each body), so a reordered
    // declaration is a second, disagreeing answer to when a Hook runs.
    let declared_sequence: Vec<&str> = hook_bindings
        .iter()
        .map(|binding| binding.hook_id.as_str())
        .collect();
    let carried_sequence: Vec<&str> = carried
        .iter()
        .map(|binding| binding.hook_id.as_str())
        .collect();
    if declared_sequence != carried_sequence
        && declared_sequence
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            == carried_sequence
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
    {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            "hook_bindings".to_string(),
            "declared Hook bindings are in a different order than the artifact's AIR carries them",
        ));
    }
}

/// Hold the graph's declared requirements against the operands the lowered AIR
/// actually names, in both directions.
///
/// A requirement no operand names is an authority the program never exercises;
/// an operand no requirement covers is an effect with no declared requirement
/// behind it, which is exactly how an unresolvable Capability reference used to
/// reach a running artifact. Neither is a warning: an artifact whose
/// requirement set and effect set disagree has no single answer to "what does
/// this program need", so it is not built at all.
fn reconcile_requirements_with_air(
    graph: &FrontendGraph,
    air: &AirModule,
) -> Result<(), ArtifactBuildError> {
    let mut verdict = Verdict::accepted();

    let declared_capabilities: std::collections::BTreeSet<&str> = graph
        .capability_requirements
        .iter()
        .map(|requirement| requirement.capability_ref.as_str())
        .collect();
    let declared_models: std::collections::BTreeSet<&str> = graph
        .model_requirements
        .iter()
        .map(|requirement| requirement.model_target_ref.as_str())
        .collect();

    let invoked = air_operand_refs(
        air,
        SemanticOpKind::CapabilityInvoke,
        apxm_ais::SLOT_CAPABILITY_REF,
    );
    let called = air_operand_refs(air, SemanticOpKind::ModelCall, apxm_ais::SLOT_MODEL_REF);

    for capability_ref in &invoked {
        if !declared_capabilities.contains(capability_ref.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                capability_ref.clone(),
                "a lowered capability.invoke names a Capability the graph declares no \
                 capability_requirement for",
            ));
        }
    }
    for capability_ref in &declared_capabilities {
        if !invoked.contains(*capability_ref) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                (*capability_ref).to_string(),
                "a declared capability_requirement is named by no lowered capability.invoke",
            ));
        }
    }
    for model_ref in &called {
        if !declared_models.contains(model_ref.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                model_ref.clone(),
                "a lowered model.call names a target the graph declares no model_requirement for",
            ));
        }
    }
    for model_ref in &declared_models {
        if !called.contains(*model_ref) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                (*model_ref).to_string(),
                "a declared model_requirement is named by no lowered model.call",
            ));
        }
    }

    // The authored permission request now exists twice inside one artifact:
    // on the digest-bound source bundle as declared, and on the AIR the runtime
    // resolves from. Holding them equal is what keeps the artifact's two
    // answers to "what did this program ask for" from becoming two facts, the
    // same reason `hook_bindings` is reconciled rather than trusted.
    let declared_requests = crate::lower::lower_capability_permission_requests(graph);
    if declared_requests != air.capability_permission_requests {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            "capability_permission_requests".to_string(),
            "the AIR's capability permission requests disagree with the authored capability \
             requirements the source bundle binds",
        ));
    }

    if verdict.is_accepted() {
        Ok(())
    } else {
        Err(ArtifactBuildError::Requirements(verdict))
    }
}

impl std::error::Error for ArtifactBuildError {}

fn entrypoint_from_definition(program: &crate::frontend_graph::ProgramDefinition) -> Entrypoint {
    Entrypoint {
        entrypoint: program.entrypoint.clone(),
        program_id: program.program_id.clone(),
        input_type_ref: program.input_type_ref.clone(),
        output_type_ref: program.output_type_ref.clone(),
        context_type_ref: program.context_type_ref.clone(),
    }
}

/// Build the `artifact_semantic` Port Requirement a `model.call` declares for
/// its model target, bound to the `apxm.model-target` port contract.
fn model_target_requirement(target: &str) -> PortRequirement {
    port_requirement(
        target,
        MODEL_TARGET_PORT_CONTRACT,
        sha256_digest(MODEL_TARGET_PORT_CONTRACT.as_bytes()),
    )
}

/// Build the `artifact_semantic` Port Requirement a `capability.invoke` declares.
fn capability_requirement(capability_ref: &str) -> PortRequirement {
    port_requirement(
        capability_ref,
        CAPABILITY_PORT_CONTRACT,
        sha256_digest(CAPABILITY_PORT_CONTRACT_DESCRIPTOR),
    )
}

fn port_requirement(
    typed_port_slot: &str,
    contract_schema_id: &str,
    contract_digest: String,
) -> PortRequirement {
    let contract = SchemaDigestRef {
        schema_id: contract_schema_id.to_string(),
        digest: contract_digest,
    };
    let source_digest = sha256_digest(typed_port_slot.as_bytes());
    let semantic_limits_digest = sha256_digest(b"");
    let requirement_digest = sha256_digest(
        format!(
            "{typed_port_slot}|{}|{}|{source_digest}",
            contract.schema_id, contract.digest
        )
        .as_bytes(),
    );
    PortRequirement {
        schema_version: PortRequirementVersion::V1,
        typed_port_slot: typed_port_slot.to_string(),
        required_port_contract: contract,
        required_feature_set: Vec::new(),
        semantic_limits_digest,
        source_owner: SemanticOwner::Agents,
        source_scope: PortSourceScope::ArtifactSemantic,
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

/// Validate an artifact presented as JSON, failing closed on decode errors.
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
                    "parent_region_id": "region.body",
                    "execution_order": i,
                    "operands": [
                        { "slot": "model_ref", "value_id": target, "type_ref": "ModelRef" }
                    ]
                })
            })
            .chain(std::iter::once(serde_json::json!({
                "node_id": "node.await",
                "op": "await.event",
                "parent_region_id": "region.body",
                "execution_order": model_targets.len(),
                "operands": [
                    { "slot": "event_ref", "value_id": "session-input", "type_ref": "EventRef" }
                ]
            })))
            .collect();
        serde_json::from_value(serde_json::json!({
            "schema_version": "apxm.air",
            "semantic_operations": ops,
            "structural_ir": [
                { "region_id": "region.body", "kind": "region", "execution_order": 0 },
                {
                    "region_id": "region.return",
                    "kind": "return",
                    "parent_region_id": "region.body",
                    "execution_order": model_targets.len() + 1
                }
            ],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": []
            }
        }))
        .expect("valid canonical AIR")
    }

    /// One carried Hook binding, riding on the region holding its captured body.
    fn carried_hook(hook_id: &str, phase: &str, declaration_order: u32) -> serde_json::Value {
        serde_json::json!({
            "hook_id": hook_id,
            "scope": "capability",
            "phase": phase,
            "target_selector": "node.cap",
            "declaration_order": declaration_order,
            "handler_ref": format!("handlers.{hook_id}"),
            "handler_digest":
                "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "input_type_ref": "HookContext",
            "output_type_ref": "Unit",
            "return_mode": "observe",
            "body_region_id": format!("{hook_id}.body")
        })
    }

    /// AIR that declares two Hooks around one `capability.invoke`, in schedule
    /// order: the before-Hook's body region precedes the after-Hook's.
    fn hooked_air() -> AirModule {
        serde_json::from_value(serde_json::json!({
            "schema_version": "apxm.air",
            "semantic_operations": [{
                "node_id": "node.cap",
                "op": "capability.invoke",
                "parent_region_id": "region.body",
                "execution_order": 2,
                "operands": [
                    {
                        "slot": "capability_ref",
                        "value_id": "cap.search",
                        "type_ref": "CapabilityRef"
                    }
                ]
            }],
            "structural_ir": [
                { "region_id": "region.body", "kind": "region", "execution_order": 0 },
                {
                    "region_id": "hook.before.body",
                    "kind": "region",
                    "parent_region_id": "region.body",
                    "execution_order": 1,
                    "hook": carried_hook("hook.before", "before", 0)
                },
                {
                    "region_id": "hook.after.body",
                    "kind": "region",
                    "parent_region_id": "region.body",
                    "execution_order": 3,
                    "hook": carried_hook("hook.after", "after", 1)
                },
                {
                    "region_id": "region.return",
                    "kind": "return",
                    "parent_region_id": "region.body",
                    "execution_order": 4
                }
            ],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": []
            }
        }))
        .expect("valid canonical AIR carrying Hook bindings")
    }

    /// The second door onto the inert-Hook bug: `from_air` used to hardcode an
    /// empty `hook_bindings`, so AIR that carried Hooks became a digest-sealed
    /// artifact declaring none.
    #[test]
    fn from_air_declares_the_hook_bindings_its_air_carries() {
        let air = hooked_air();
        let artifact = ExecutableArtifact::from_air(&air).expect("from_air");

        assert_eq!(
            artifact
                .hook_bindings
                .iter()
                .map(|binding| binding.hook_id.as_str())
                .collect::<Vec<_>>(),
            vec!["hook.before", "hook.after"],
            "an artifact built from Hook-carrying AIR declares those Hooks, in AIR order"
        );
        assert_eq!(artifact.hook_bindings, air_hook_bindings(&air));
        assert!(
            artifact.validate().is_accepted(),
            "{:?}",
            artifact.validate()
        );
    }

    /// The producer runs the consumer's validation. A module whose typed operand
    /// slot is not a contract identifier has no valid artifact, so none is minted.
    #[test]
    fn from_air_refuses_a_module_whose_artifact_would_not_validate() {
        let error = ExecutableArtifact::from_air(&air(&["model target"]))
            .expect_err("a model target that is not a contract identifier is not an artifact");
        assert!(
            matches!(error, ArtifactBuildError::Validation(_)),
            "expected a validation refusal, got {error}"
        );
    }

    /// Hook order is the schedule, so reordering the declared list is a real
    /// disagreement with the AIR even though every binding is still present.
    #[test]
    fn a_reordered_hook_declaration_does_not_validate() {
        let mut artifact = ExecutableArtifact::from_air(&hooked_air()).expect("from_air");
        artifact.hook_bindings.reverse();
        assert!(
            !artifact.validate().is_accepted(),
            "declared Hook order must match the order the AIR carries"
        );
    }

    /// Two bindings under one `hook_id` make every by-id lookup answer for
    /// whichever copy is found first.
    #[test]
    fn a_duplicated_declared_hook_id_does_not_validate() {
        let mut artifact = ExecutableArtifact::from_air(&hooked_air()).expect("from_air");
        artifact.hook_bindings[1].hook_id = artifact.hook_bindings[0].hook_id.clone();
        assert!(!artifact.validate().is_accepted());
    }

    #[test]
    fn from_air_produces_a_validating_digest_bound_artifact() {
        let artifact = ExecutableArtifact::from_air(&air(&["model.target"])).expect("from_air");
        assert_eq!(artifact.schema_version, ArtifactVersion::V1);
        assert_eq!(artifact.integrity_algorithm, IntegrityAlgorithm::Sha256);
        assert!(is_digest(&artifact.artifact_digest));
        assert!(is_digest(&artifact.air_digest));
        assert!(is_digest(&artifact.source_bundle_digest));
        assert_eq!(artifact.entrypoints.len(), 1);
        assert!(
            artifact.validate().is_accepted(),
            "from_air artifact must validate: {:?}",
            artifact.validate()
        );
    }

    #[test]
    fn from_air_emits_one_artifact_semantic_requirement_per_distinct_model_target() {
        let artifact =
            ExecutableArtifact::from_air(&air(&["model.target", "model.target", "model.fast"]))
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
        assert!(slots.contains(&"model.target") && slots.contains(&"model.fast"));
    }

    #[test]
    fn from_air_is_deterministic_and_content_addressed() {
        let a = ExecutableArtifact::from_air(&air(&["model.target"])).expect("from_air");
        let b = ExecutableArtifact::from_air(&air(&["model.target"])).expect("from_air");
        assert_eq!(a, b, "from_air is a deterministic content address");
        let c = ExecutableArtifact::from_air(&air(&["model.other"])).expect("from_air");
        assert_ne!(a.artifact_digest, c.artifact_digest);
    }

    #[test]
    fn artifact_rejects_mismatched_execution_lineage() {
        let mut artifact = ExecutableArtifact::from_air(&air(&["model.target"])).expect("from_air");
        artifact.execution_lineage_ref = Some(sha256_digest(b"wrong-lineage"));
        assert!(!artifact.validate().is_accepted());
    }

    #[test]
    fn model_free_air_has_no_requirements_but_still_validates() {
        let artifact = ExecutableArtifact::from_air(&air(&[])).expect("from_air");
        assert!(artifact.artifact_semantic_requirements.is_empty());
        assert!(artifact.validate().is_accepted());
    }

    #[test]
    fn execution_decode_rejects_envelope_tampering_and_digest_mismatch() {
        let artifact = ExecutableArtifact::from_air(&air(&["model.target"])).expect("from_air");
        let bytes = artifact.encode().expect("encode");
        assert!(
            ExecutableArtifact::decode_for_execution(&bytes, &artifact.artifact_digest).is_ok()
        );

        let mut tampered: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        tampered["source_map"]["source_language"] = serde_json::json!("typescript");
        let tampered_bytes = serde_json::to_vec(&tampered).expect("tampered json");
        assert!(
            ExecutableArtifact::decode_for_execution(&tampered_bytes, &artifact.artifact_digest)
                .is_err()
        );
        assert!(ExecutableArtifact::decode_for_execution(&bytes, "sha256:wrong").is_err());
    }

    #[test]
    fn execution_decode_requires_compiler_lineage() {
        let mut artifact = ExecutableArtifact::from_air(&air(&[])).expect("from_air");
        artifact.execution_lineage_ref = None;
        artifact.artifact_digest = artifact.canonical_digest().expect("digest");
        let bytes = artifact.encode().expect("encode");
        assert!(matches!(
            ExecutableArtifact::decode_for_execution(&bytes, &artifact.artifact_digest),
            Err(error) if error == "missing_execution_lineage"
        ));
    }
}

#[cfg(test)]
mod from_graph_tests {
    use super::*;
    use crate::frontend_graph::FrontendGraph;

    fn specialist_graph() -> FrontendGraph {
        serde_json::from_value(serde_json::json!({
            "schema_version": "apxm.frontend-graph",
            "source_language": "python",
            "program_definitions": [{
                "program_id": "Specialist",
                "entrypoint": "run",
                "input_type_ref": "SpecialistInput",
                "output_type_ref": "SpecialistOutput",
                "context_type_ref": "SpecialistContext",
                "has_default_context": true
            }],
            "imported_program_refs": [{
                "program_ref": "Summarizer",
                "artifact_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "entrypoint": "run",
                "target_agent_identity_requirement": "summarizer-identity"
            }],
            "declarations": [
                {
                    "decl_id": "decl.model.target",
                    "decl_kind": "model_binding",
                    "input_type_ref": "ModelRequest",
                    "output_type_ref": "ModelResponse",
                    "target_ref": "model.target"
                },
                {
                    "decl_id": "decl.cap.search",
                    "decl_kind": "capability_binding",
                    "input_type_ref": "SearchArguments",
                    "output_type_ref": "SearchResult",
                    "target_ref": "cap.search"
                }
            ],
            "functions": [{
                "function_id": "run",
                "parameters": [
                    {"value_id": "value.input", "type_ref": "SpecialistInput", "role": "input"}
                ],
                "result_type_ref": "SpecialistOutput",
                "body_region_id": "region.body",
                "is_entrypoint": true
            }],
            "values": [
                {"value_id": "value.input", "type_ref": "SpecialistInput", "origin": "parameter", "origin_id": "run"},
                {"value_id": "value.model.out", "type_ref": "ModelResponse", "origin": "call_result", "origin_id": "node.model.1"},
                {"value_id": "value.cap.out", "type_ref": "SearchResult", "origin": "call_result", "origin_id": "node.cap.1"}
            ],
            "blocks": [],
            "regions": [
                { "region_id": "region.body", "region_role": "function_body", "execution_order": 0 },
                {
                    "region_id": "region.loop.1",
                    "region_role": "loop_body",
                    "parent_region_id": "region.body",
                    "execution_order": 0
                }
            ],
            "data_edges": [
                {"from_value": "value.input", "to_consumer": "node.model.1", "consumer_slot": "request"},
                {"from_value": "value.model.out", "to_consumer": "node.cap.1", "consumer_slot": "arguments"}
            ],
            "call_intents": [
                {
                    "node_id": "node.model.1",
                    "intent_kind": "model_invocation",
                    "parent_region_id": "region.loop.1",
                    "execution_order": 0,
                    "binding_ref": "decl.model.target",
                    "operand_values": ["value.input"],
                    "result_value": "value.model.out"
                },
                {
                    "node_id": "node.cap.1",
                    "intent_kind": "capability_invocation",
                    "parent_region_id": "region.loop.1",
                    "execution_order": 1,
                    "binding_ref": "decl.cap.search",
                    "operand_values": ["value.model.out"],
                    "result_value": "value.cap.out"
                }
            ],
            "control_intents": [{
                "node_id": "node.loop.1",
                "control_kind": "loop",
                "parent_region_id": "region.body",
                "execution_order": 0,
                "body_region_ids": ["region.loop.1"]
            }],
            "context_flow": [],
            "hook_bindings": [],
            "capability_requirements": [{ "capability_ref": "cap.search" }],
            "model_requirements": [{ "model_target_ref": "model.target" }],
            "skill_requirements": [],
            "source_map": {
                "schema_version": "apxm.source-map",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": [
                    {"region_id": "region.loop.1", "annotation": "structural_loop"}
                ]
            }
        }))
        .expect("fixture graph")
    }

    #[test]
    fn from_frontend_graph_binds_entrypoints_handlers_and_requirements() {
        let graph = specialist_graph();
        let artifact =
            ExecutableArtifact::from_frontend_graph(&graph).expect("artifact from graph");
        assert_eq!(artifact.entrypoints.len(), 1);
        assert_eq!(artifact.entrypoints[0].program_id, "Specialist");
        assert_eq!(artifact.entrypoints[0].entrypoint, "run");
        assert_eq!(artifact.artifact_semantic_requirements.len(), 2);
        assert_eq!(artifact.hook_bindings.len(), graph.hook_bindings.len());
        assert!(artifact.validate().is_accepted());
        let bundle = SourceBundle::from_graph(&graph);
        assert_eq!(
            artifact.source_bundle_digest,
            bundle.digest().expect("bundle digest")
        );
    }

    #[test]
    fn a_requirement_no_lowered_operand_names_is_refused() {
        let mut graph = specialist_graph();
        graph
            .capability_requirements
            .push(crate::frontend_graph::CapabilityRequirement {
                capability_ref: "cap.unexercised".to_string(),
                tool_schema_present: None,
                requested_permission: None,
            });
        let error = ExecutableArtifact::from_frontend_graph(&graph)
            .expect_err("a requirement the program never exercises is not an artifact");
        assert!(
            matches!(error, ArtifactBuildError::Requirements(_)),
            "expected a requirement reconciliation failure, got {error}"
        );
    }

    #[test]
    fn an_operand_no_requirement_covers_is_refused() {
        let mut graph = specialist_graph();
        graph.capability_requirements.clear();
        // The graph verifier refuses this too, so lowering is the failure that
        // surfaces first; either way the artifact is not built.
        assert!(
            ExecutableArtifact::from_frontend_graph(&graph).is_err(),
            "a capability.invoke with no declared requirement is not an artifact"
        );
    }

    #[test]
    fn capability_requirement_binds_the_exact_owned_port_contract() {
        let artifact = ExecutableArtifact::from_frontend_graph(&specialist_graph())
            .expect("artifact from graph");
        let requirement = artifact
            .artifact_semantic_requirements
            .iter()
            .find(|requirement| requirement.typed_port_slot == "cap.search")
            .expect("Capability requirement");

        assert_eq!(
            requirement.required_port_contract.schema_id,
            CAPABILITY_PORT_CONTRACT
        );
        assert_eq!(
            requirement.required_port_contract.digest,
            sha256_digest(CAPABILITY_PORT_CONTRACT_DESCRIPTOR)
        );
    }

    #[test]
    fn rejects_mixed_requirement_scope_in_validation() {
        let mut artifact =
            ExecutableArtifact::from_frontend_graph(&specialist_graph()).expect("artifact");
        artifact
            .artifact_semantic_requirements
            .push(PortRequirement {
                schema_version: PortRequirementVersion::V1,
                typed_port_slot: "deployment".to_string(),
                required_port_contract: SchemaDigestRef {
                    schema_id: "apxm.execution-commit".to_string(),
                    digest: sha256_digest(b"apxm.execution-commit"),
                },
                required_feature_set: Vec::new(),
                semantic_limits_digest: sha256_digest(b""),
                source_scope: PortSourceScope::DeploymentInfrastructure,
                source_owner: SemanticOwner::Server,
                source_digest: sha256_digest(b"deployment"),
                requirement_digest: sha256_digest(b"deployment-req"),
            });
        assert!(
            artifact
                .validate()
                .diagnostics()
                .iter()
                .any(|d| d.code == DiagnosticCode::RequirementScopeNotArtifactSemantic)
        );
    }
}
