//! Product-neutral Execution Admission (agents synonym: Invocation Admission).
//!
//! This module freezes and enforces the product-neutral admission contract:
//!
//! - signed, expiring, nonce-bound envelope;
//! - exact Port bindings and exact model target when present;
//! - pinned confinement (fail closed when unavailable);
//! - opaque caller correlations only (no downstream product schema);
//! - no ambient credentials, filesystem, network, plugin, or unconfined fallback.
//!
//! Construction roots may seal admissions. The runtime verifies them before any
//! state mutation and never fabricates, rebinds, ranks, or falls back.

use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use apxm_program::artifact::SchemaDigestRef;
use apxm_program::grammar::{is_digest, is_identifier};

use crate::bundle::{
    BundleError, ExactPortBinding, PortBundle, PortBundleSpec, PortImplementation, PortSlot,
};
use crate::confinement::{
    ConfinementAttestation, ConfinementError, ConfinementRequest, ConfinementType,
};

/// Frozen schema id for the product-neutral Execution Admission envelope.
pub const EXECUTION_ADMISSION_SCHEMA: &str = "apxm.execution-admission.v1";

/// Frozen schema id for the product-neutral host-to-runtime invocation record.
///
/// This record is the transport-facing runtime authority used by an external
/// composition root. It is verified against the exact artifact, release,
/// provenance, Port bindings, resource ceilings, and confinement descriptors
/// before the immutable runtime bundle is constructed.
pub const INVOCATION_ADMISSION_SCHEMA: &str = "apxm.invocation-admission.v1";

/// Closed Port Contract schema IDs accepted by the runtime admission boundary.
pub const EXECUTION_COMMIT_PORT_SCHEMA: &str = "apxm.execution-commit.v1";
pub const CONFINEMENT_PORT_SCHEMA: &str = "apxm.confinement.v1";
pub const MODEL_INFERENCE_PORT_SCHEMA: &str = "apxm.model-inference.v1";
pub const CAPABILITY_PORT_SCHEMA: &str = "apxm.capability-invocation.v1";
pub const EXTERNAL_AGENT_PORT_SCHEMA: &str = "apxm.external-agent.v1";
pub const DURABLE_EVENT_PORT_SCHEMA: &str = "apxm.durable-event.v1";
pub const PROGRAM_COMPOSITION_PORT_SCHEMA: &str = "apxm.program-composition.v1";

/// Exact product-neutral authority supplied by an APXM host transport.
///
/// The release, artifact, and provenance fields remain separate because each
/// is checked against its own exact bytes at the runtime boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationAdmission {
    pub schema_version: String,
    pub invocation_id: String,
    pub artifact_digest: String,
    pub release_digest: String,
    pub port_bindings_digest: String,
    pub resource_ceiling_digest: String,
    pub provenance_digest: String,
}

/// Fail-closed validation errors for the transport-facing invocation record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvocationAdmissionError {
    SchemaMismatch(String),
    InvalidInvocationId,
    MalformedDigest(&'static str),
    ArtifactMismatch { expected: String, actual: String },
    ReleaseMismatch { expected: String, actual: String },
    ProvenanceMismatch { expected: String, actual: String },
    PortBindingsDigestMismatch { expected: String, actual: String },
    ResourceCeilingDigestMismatch { expected: String, actual: String },
    ConfinementUnavailable,
    UnconfinedForbidden,
    InvalidRuntimeDescriptor(&'static str),
}

impl std::fmt::Display for InvocationAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SchemaMismatch(actual) => {
                write!(
                    f,
                    "schema mismatch: expected {INVOCATION_ADMISSION_SCHEMA}, got {actual}"
                )
            }
            Self::InvalidInvocationId => f.write_str("invocation_id is not a valid identifier"),
            Self::MalformedDigest(field) => write!(f, "malformed digest: {field}"),
            Self::ArtifactMismatch { expected, actual } => {
                write!(
                    f,
                    "artifact digest mismatch: expected {expected}, got {actual}"
                )
            }
            Self::ReleaseMismatch { expected, actual } => {
                write!(
                    f,
                    "release digest mismatch: expected {expected}, got {actual}"
                )
            }
            Self::ProvenanceMismatch { expected, actual } => {
                write!(
                    f,
                    "provenance digest mismatch: expected {expected}, got {actual}"
                )
            }
            Self::PortBindingsDigestMismatch { expected, actual } => write!(
                f,
                "port binding digest mismatch: expected {expected}, got {actual}"
            ),
            Self::ResourceCeilingDigestMismatch { expected, actual } => write!(
                f,
                "resource ceiling digest mismatch: expected {expected}, got {actual}"
            ),
            Self::ConfinementUnavailable => f.write_str("confinement is not available"),
            Self::UnconfinedForbidden => f.write_str("unconfined execution is forbidden"),
            Self::InvalidRuntimeDescriptor(field) => {
                write!(f, "invalid runtime descriptor: {field}")
            }
        }
    }
}

impl std::error::Error for InvocationAdmissionError {}

impl InvocationAdmission {
    /// Validate the published wire contract independently of any host state.
    pub fn validate(&self) -> Result<(), InvocationAdmissionError> {
        if self.schema_version != INVOCATION_ADMISSION_SCHEMA {
            return Err(InvocationAdmissionError::SchemaMismatch(
                self.schema_version.clone(),
            ));
        }
        if !is_identifier(&self.invocation_id) {
            return Err(InvocationAdmissionError::InvalidInvocationId);
        }
        for (field, value) in [
            ("artifact_digest", self.artifact_digest.as_str()),
            ("release_digest", self.release_digest.as_str()),
            ("port_bindings_digest", self.port_bindings_digest.as_str()),
            (
                "resource_ceiling_digest",
                self.resource_ceiling_digest.as_str(),
            ),
            ("provenance_digest", self.provenance_digest.as_str()),
        ] {
            if !is_digest(value) {
                return Err(InvocationAdmissionError::MalformedDigest(field));
            }
        }
        Ok(())
    }

    /// Verify this record against the immutable digests published by a host.
    pub fn verify_against(
        &self,
        release_digest: &str,
        port_bindings_digest: &str,
        resource_ceiling_digest: &str,
    ) -> Result<(), InvocationAdmissionError> {
        self.validate()?;
        if self.release_digest != release_digest {
            return Err(InvocationAdmissionError::ReleaseMismatch {
                expected: release_digest.into(),
                actual: self.release_digest.clone(),
            });
        }
        if self.port_bindings_digest != port_bindings_digest {
            return Err(InvocationAdmissionError::PortBindingsDigestMismatch {
                expected: port_bindings_digest.into(),
                actual: self.port_bindings_digest.clone(),
            });
        }
        if self.resource_ceiling_digest != resource_ceiling_digest {
            return Err(InvocationAdmissionError::ResourceCeilingDigestMismatch {
                expected: resource_ceiling_digest.into(),
                actual: self.resource_ceiling_digest.clone(),
            });
        }
        Ok(())
    }
}

/// Hash one serialized APXM descriptor using canonical JSON object ordering.
pub fn digest_serializable<T: Serialize + ?Sized>(
    value: &T,
) -> Result<String, InvocationAdmissionError> {
    let value = serde_json::to_value(value)
        .map_err(|_| InvocationAdmissionError::InvalidRuntimeDescriptor("serialization"))?;
    Ok(content_digest(canonical_json(&value).as_bytes()))
}

/// The only signature algorithm the closed envelope admits.
const ED25519: &str = "ed25519";

/// Correlation / field names that would smuggle ambient or product-plane authority.
const FORBIDDEN_AMBIENT_KEYS: &[&str] = &[
    "company_ref",
    "budget_reservation_ref",
    "acting_principal_ref",
    "agent_identity_ref",
    "grant_fact_refs",
    "credential_lease_refs",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "HOME",
    "PATH",
    "SSH_AUTH_SOCK",
    "KUBECONFIG",
    "DOCKER_HOST",
    "ambient_filesystem",
    "ambient_network",
    "ambient_credentials",
    "plugin_catalogue",
    "unconfined",
    "fallback_binding",
    "best_available",
];

/// Closed Ed25519 signature envelope carried by one Execution Admission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignatureEnvelope {
    pub algorithm: String,
    pub key_ref: String,
    pub signature: String,
}

/// Exact Port binding carried inside one Execution Admission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedPortBinding {
    pub slot: String,
    pub port_contract_schema_id: String,
    pub port_contract_digest: String,
    pub binding_digest: String,
    pub proof_digest: String,
}

/// Exact model target when the admitted artifact requires inference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedModelTarget {
    pub model_target_ref: String,
    pub model_deployment_ref: String,
    pub exact_port_binding_digest: String,
}

/// Pinned confinement backend/policy for one admission. Absence is illegal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedConfinement {
    pub confinement_type: String,
    pub sandbox_digest: String,
    pub policy_digest: String,
}

/// Explicit resource ceilings carried by admission. Zero means refuse that class.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceCeilings {
    pub max_wall_ms: u64,
    pub max_memory_bytes: u64,
    pub max_effect_bytes: u64,
}

/// Product-neutral Execution Admission envelope.
///
/// Synonym in owner docs: Invocation Admission. Contains no company, budget,
/// principal, or other downstream product schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionAdmission {
    pub schema_version: String,
    pub admission_ref: String,
    pub admission_digest: String,
    pub artifact_ref: String,
    pub artifact_digest: String,
    pub invocation_ref: String,
    pub context_digest: String,
    pub resource_ceilings: ResourceCeilings,
    pub port_bindings: Vec<AdmittedPortBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_target: Option<AdmittedModelTarget>,
    pub confinement: AdmittedConfinement,
    pub expires_at_ms: u64,
    pub nonce: String,
    pub issuer: String,
    pub audience: String,
    /// Opaque caller correlations only. Forbidden ambient/product keys fail.
    #[serde(default)]
    pub caller_correlations: BTreeMap<String, String>,
    pub signature: SignatureEnvelope,
}

/// One enrolled issuer verifying key with explicit expiry and revocation.
#[derive(Debug, Clone)]
pub struct IssuerKey {
    pub key_ref: String,
    pub verifying_key: VerifyingKey,
    pub not_after_ms: u64,
    pub revoked: bool,
}

/// Explicit issuer keyring. Empty admits nothing.
#[derive(Debug, Clone, Default)]
pub struct IssuerKeyring {
    keys: BTreeMap<String, IssuerKey>,
}

impl IssuerKeyring {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            keys: BTreeMap::new(),
        }
    }

    pub fn from_keys(keys: impl IntoIterator<Item = IssuerKey>) -> Result<Self, AdmissionError> {
        let mut enrolled = BTreeMap::new();
        for key in keys {
            let key_ref = key.key_ref.clone();
            if key_ref.trim().is_empty() || !is_identifier(&key_ref) {
                return Err(AdmissionError::MalformedKeyring(format!(
                    "issuer key reference {key_ref:?} is invalid"
                )));
            }
            if enrolled.insert(key_ref.clone(), key).is_some() {
                return Err(AdmissionError::MalformedKeyring(format!(
                    "issuer key {key_ref} enrolled more than once"
                )));
            }
        }
        Ok(Self { keys: enrolled })
    }

    pub fn resolve(&self, key_ref: &str, now_ms: u64) -> Result<&VerifyingKey, AdmissionError> {
        let key = self
            .keys
            .get(key_ref)
            .ok_or(AdmissionError::Signature(SignatureRejection::KeyUnknown))?;
        if key.revoked {
            return Err(AdmissionError::Signature(SignatureRejection::KeyRevoked));
        }
        if now_ms >= key.not_after_ms {
            return Err(AdmissionError::Signature(SignatureRejection::KeyExpired));
        }
        Ok(&key.verifying_key)
    }
}

/// Issuer signing key used by construction-root fixtures and tests.
#[derive(Debug)]
pub struct IssuerSigningKey {
    key_ref: String,
    signing_key: SigningKey,
}

impl IssuerSigningKey {
    #[must_use]
    pub fn generate(key_ref: impl Into<String>) -> Self {
        let mut seed = [0u8; 32];
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        seed[..16].copy_from_slice(a.as_bytes());
        seed[16..].copy_from_slice(b.as_bytes());
        Self {
            key_ref: key_ref.into(),
            signing_key: SigningKey::from_bytes(&seed),
        }
    }

    #[must_use]
    pub fn key_ref(&self) -> &str {
        &self.key_ref
    }

    #[must_use]
    pub fn enrollment(&self, not_after_ms: u64, revoked: bool) -> IssuerKey {
        IssuerKey {
            key_ref: self.key_ref.clone(),
            verifying_key: self.signing_key.verifying_key(),
            not_after_ms,
            revoked,
        }
    }

    /// Seal an unsigned admission skeleton (digest/signature placeholders ok).
    pub fn seal_admission(&self, mut admission: ExecutionAdmission) -> ExecutionAdmission {
        admission.admission_digest = String::new();
        admission.signature = SignatureEnvelope {
            algorithm: ED25519.into(),
            key_ref: self.key_ref.clone(),
            signature: String::new(),
        };
        let payload = signing_payload(&admission).expect("admission is signable");
        let digest = content_digest(&payload);
        let signature = BASE64.encode(self.signing_key.sign(&payload).to_bytes());
        admission.admission_digest = digest;
        admission.signature = SignatureEnvelope {
            algorithm: ED25519.into(),
            key_ref: self.key_ref.clone(),
            signature,
        };
        admission
    }
}

/// Distinct signature verification failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureRejection {
    RecordNotSignable,
    EnvelopeAbsent,
    AlgorithmUnsupported,
    KeyUnknown,
    KeyExpired,
    KeyRevoked,
    SignatureMalformed,
    SignatureMismatch,
    DigestMismatch,
}

impl SignatureRejection {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RecordNotSignable => "record_not_signable",
            Self::EnvelopeAbsent => "envelope_absent",
            Self::AlgorithmUnsupported => "algorithm_unsupported",
            Self::KeyUnknown => "key_unknown",
            Self::KeyExpired => "key_expired",
            Self::KeyRevoked => "key_revoked",
            Self::SignatureMalformed => "signature_malformed",
            Self::SignatureMismatch => "signature_mismatch",
            Self::DigestMismatch => "digest_mismatch",
        }
    }
}

impl std::fmt::Display for SignatureRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why Execution Admission verification failed. Every variant fails closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    SchemaMismatch(String),
    Signature(SignatureRejection),
    Expired {
        expires_at_ms: u64,
        now_ms: u64,
    },
    NonceReuse(String),
    AudienceMismatch {
        expected: String,
        actual: String,
    },
    IssuerEmpty,
    MalformedDigest(&'static str),
    MalformedKeyring(String),
    MissingRequiredSlot(PortSlot),
    AmbiguousBinding(PortSlot),
    UnknownPortSlot(String),
    ConfinementUnavailable,
    ConfinementTypeUnknown(String),
    UnconfinedForbidden,
    AmbientAuthorityRefused(String),
    ProductPlaneField(String),
    MissingModelTarget,
    ModelBindingMismatch,
    EmptyNonce,
    EmptyAudience,
    EmptyReference(&'static str),
    InvalidReference(&'static str),
    PortContractSchemaMismatch {
        slot: PortSlot,
        expected: &'static str,
        actual: String,
    },
    CheckpointVersionMismatch {
        expected: u64,
        actual: u64,
    },
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SchemaMismatch(v) => write!(f, "schema mismatch: {v}"),
            Self::Signature(r) => write!(f, "signature rejected: {r}"),
            Self::Expired {
                expires_at_ms,
                now_ms,
            } => write!(f, "admission expired at {expires_at_ms} (now {now_ms})"),
            Self::NonceReuse(n) => write!(f, "nonce reuse: {n}"),
            Self::AudienceMismatch { expected, actual } => {
                write!(f, "audience mismatch: expected {expected}, got {actual}")
            }
            Self::IssuerEmpty => write!(f, "issuer must be non-empty"),
            Self::MalformedDigest(field) => write!(f, "malformed digest: {field}"),
            Self::MalformedKeyring(m) => write!(f, "malformed keyring: {m}"),
            Self::MissingRequiredSlot(slot) => {
                write!(f, "missing required port binding: {}", slot.as_str())
            }
            Self::AmbiguousBinding(slot) => {
                write!(f, "ambiguous duplicate port binding: {}", slot.as_str())
            }
            Self::UnknownPortSlot(slot) => write!(f, "unknown port slot: {slot}"),
            Self::ConfinementUnavailable => write!(f, "confinement unavailable"),
            Self::ConfinementTypeUnknown(t) => write!(f, "unknown confinement type: {t}"),
            Self::UnconfinedForbidden => write!(f, "unconfined execution is forbidden"),
            Self::AmbientAuthorityRefused(k) => {
                write!(f, "ambient or product-plane authority refused: {k}")
            }
            Self::ProductPlaneField(k) => write!(f, "product-plane field refused: {k}"),
            Self::MissingModelTarget => write!(f, "model target required but absent"),
            Self::ModelBindingMismatch => {
                write!(f, "model target does not match an admitted model binding")
            }
            Self::EmptyNonce => write!(f, "nonce must be non-empty"),
            Self::EmptyAudience => write!(f, "audience must be non-empty"),
            Self::EmptyReference(field) => write!(f, "{field} must be non-empty"),
            Self::InvalidReference(field) => write!(f, "{field} is not a valid opaque reference"),
            Self::PortContractSchemaMismatch {
                slot,
                expected,
                actual,
            } => write!(
                f,
                "port slot {} requires contract {expected}, got {actual}",
                slot.as_str()
            ),
            Self::CheckpointVersionMismatch { expected, actual } => write!(
                f,
                "checkpoint commit version must be {expected}, got {actual}"
            ),
        }
    }
}

impl std::error::Error for AdmissionError {}

/// In-memory nonce ledger for one runtime instance. A reused nonce fails closed.
#[derive(Debug, Default)]
pub struct NonceLedger {
    seen: Mutex<HashSet<String>>,
}

impl NonceLedger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe(&self, nonce: &str) -> Result<(), AdmissionError> {
        let mut seen = self.seen.lock().expect("nonce ledger");
        if !seen.insert(nonce.to_string()) {
            return Err(AdmissionError::NonceReuse(nonce.to_string()));
        }
        Ok(())
    }
}

/// Inputs the verifier constructs from one sealed admission.
#[derive(Clone, Debug)]
pub struct VerifiedExecutionAdmission {
    pub admission: ExecutionAdmission,
    pub port_bindings: Vec<ExactPortBinding>,
    pub bundle_spec: PortBundleSpec,
    pub confinement_type: ConfinementType,
    pub sandbox_digest: String,
    pub policy_digest: String,
}

/// Inputs verified directly from one transport Invocation Admission.
#[derive(Clone, Debug)]
pub struct VerifiedInvocationAdmission {
    pub admission: InvocationAdmission,
    pub port_bindings: Vec<ExactPortBinding>,
    pub bundle_spec: PortBundleSpec,
    pub resource_ceilings: ResourceCeilings,
    pub confinement_type: ConfinementType,
    pub sandbox_digest: String,
    pub policy_digest: String,
}

/// Verify the transport authority against the bytes and exact runtime
/// descriptors it is claiming. No signing key, nonce ledger, or synthetic
/// admission is introduced by this path.
pub fn verify_invocation_admission(
    admission: &InvocationAdmission,
    artifact_bytes: &[u8],
    release_bytes: &[u8],
    provenance_bytes: &[u8],
    admitted_port_bindings: &[AdmittedPortBinding],
    resource_ceilings: ResourceCeilings,
    confinement: &AdmittedConfinement,
) -> Result<VerifiedInvocationAdmission, InvocationAdmissionError> {
    admission.validate()?;
    let artifact_actual = content_digest(artifact_bytes);
    if admission.artifact_digest != artifact_actual {
        return Err(InvocationAdmissionError::ArtifactMismatch {
            expected: artifact_actual,
            actual: admission.artifact_digest.clone(),
        });
    }
    let release_actual = content_digest(release_bytes);
    if admission.release_digest != release_actual {
        return Err(InvocationAdmissionError::ReleaseMismatch {
            expected: release_actual,
            actual: admission.release_digest.clone(),
        });
    }
    let provenance_actual = content_digest(provenance_bytes);
    if admission.provenance_digest != provenance_actual {
        return Err(InvocationAdmissionError::ProvenanceMismatch {
            expected: provenance_actual,
            actual: admission.provenance_digest.clone(),
        });
    }
    let port_actual = digest_serializable(admitted_port_bindings)?;
    if admission.port_bindings_digest != port_actual {
        return Err(InvocationAdmissionError::PortBindingsDigestMismatch {
            expected: port_actual,
            actual: admission.port_bindings_digest.clone(),
        });
    }
    let ceilings_actual = digest_serializable(&resource_ceilings)?;
    if admission.resource_ceiling_digest != ceilings_actual {
        return Err(InvocationAdmissionError::ResourceCeilingDigestMismatch {
            expected: ceilings_actual,
            actual: admission.resource_ceiling_digest.clone(),
        });
    }
    if confinement
        .confinement_type
        .eq_ignore_ascii_case("unconfined")
        || confinement.sandbox_digest == "unconfined"
    {
        return Err(InvocationAdmissionError::UnconfinedForbidden);
    }
    if !is_digest(&confinement.sandbox_digest) || !is_digest(&confinement.policy_digest) {
        return Err(InvocationAdmissionError::InvalidRuntimeDescriptor(
            "confinement digest",
        ));
    }
    let confinement_type = parse_confinement_type(&confinement.confinement_type)
        .map_err(|_| InvocationAdmissionError::InvalidRuntimeDescriptor("confinement type"))?;
    let (port_bindings, bundle_spec) = resolve_exact_bindings(admitted_port_bindings)
        .map_err(|_| InvocationAdmissionError::InvalidRuntimeDescriptor("port bindings"))?;
    if !port_bindings
        .iter()
        .any(|binding| binding.slot == PortSlot::Confinement)
    {
        return Err(InvocationAdmissionError::ConfinementUnavailable);
    }
    Ok(VerifiedInvocationAdmission {
        admission: admission.clone(),
        port_bindings,
        bundle_spec,
        resource_ceilings,
        confinement_type,
        sandbox_digest: confinement.sandbox_digest.clone(),
        policy_digest: confinement.policy_digest.clone(),
    })
}

/// The only runtime construction result produced from a verified admission.
///
/// The composition root supplies implementations, but this boundary verifies
/// that every supplied descriptor is byte-for-byte the descriptor named by the
/// admission and attests the exact confinement sandbox and policy before the
/// bundle can be used by an instance.
pub struct RuntimeAdmission {
    authority: RuntimeAuthority,
    bundle: PortBundle,
    confinement_attestation: ConfinementAttestation,
    resource_ceilings: ResourceCeilings,
}

enum RuntimeAuthority {
    Signed(Box<VerifiedExecutionAdmission>),
    Invocation(Box<VerifiedInvocationAdmission>),
}

/// Why an exact runtime admission could not be constructed. Every variant fails
/// before an implementation can receive an execution request.
#[derive(Debug, PartialEq, Eq)]
pub enum RuntimeAdmissionError {
    Admission(AdmissionError),
    Bundle(BundleError),
    Confinement(ConfinementError),
    BindingMismatch(PortSlot),
    AttestationMismatch(&'static str),
    EmptyRuntimeIdentity(&'static str),
}

impl std::fmt::Display for RuntimeAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Admission(error) => error.fmt(f),
            Self::Bundle(error) => error.fmt(f),
            Self::Confinement(error) => error.fmt(f),
            Self::BindingMismatch(slot) => {
                write!(
                    f,
                    "supplied binding differs from admitted {} binding",
                    slot.as_str()
                )
            }
            Self::AttestationMismatch(field) => {
                write!(f, "confinement attestation does not match admitted {field}")
            }
            Self::EmptyRuntimeIdentity(field) => {
                write!(f, "{field} must be non-empty for confinement attestation")
            }
        }
    }
}

impl std::error::Error for RuntimeAdmissionError {}

impl From<AdmissionError> for RuntimeAdmissionError {
    fn from(error: AdmissionError) -> Self {
        Self::Admission(error)
    }
}

impl From<BundleError> for RuntimeAdmissionError {
    fn from(error: BundleError) -> Self {
        Self::Bundle(error)
    }
}

impl From<ConfinementError> for RuntimeAdmissionError {
    fn from(error: ConfinementError) -> Self {
        Self::Confinement(error)
    }
}

impl RuntimeAdmission {
    /// Construct the immutable runtime closure from one already-verified
    /// admission and exact implementations. The confinement port is called
    /// before this method returns; no un-attested bundle is executable.
    pub async fn admit(
        verified: VerifiedExecutionAdmission,
        entries: Vec<(ExactPortBinding, PortImplementation)>,
        host_id: impl Into<String>,
        execution_id: impl Into<String>,
    ) -> Result<Self, RuntimeAdmissionError> {
        let resource_ceilings = verified.admission.resource_ceilings.clone();
        Self::admit_authority(
            RuntimeAuthority::Signed(Box::new(verified)),
            resource_ceilings,
            entries,
            host_id,
            execution_id,
        )
        .await
    }

    /// Construct the immutable runtime closure directly from the verified
    /// transport Invocation Admission. This path preserves that authority and
    /// does not mint a second signed envelope or replay nonce.
    pub async fn admit_invocation(
        verified: VerifiedInvocationAdmission,
        entries: Vec<(ExactPortBinding, PortImplementation)>,
        host_id: impl Into<String>,
        execution_id: impl Into<String>,
    ) -> Result<Self, RuntimeAdmissionError> {
        let resource_ceilings = verified.resource_ceilings.clone();
        Self::admit_authority(
            RuntimeAuthority::Invocation(Box::new(verified)),
            resource_ceilings,
            entries,
            host_id,
            execution_id,
        )
        .await
    }

    async fn admit_authority(
        authority: RuntimeAuthority,
        resource_ceilings: ResourceCeilings,
        entries: Vec<(ExactPortBinding, PortImplementation)>,
        host_id: impl Into<String>,
        execution_id: impl Into<String>,
    ) -> Result<Self, RuntimeAdmissionError> {
        let host_id = host_id.into();
        let execution_id = execution_id.into();
        if host_id.trim().is_empty() {
            return Err(RuntimeAdmissionError::EmptyRuntimeIdentity("host_id"));
        }
        if execution_id.trim().is_empty() {
            return Err(RuntimeAdmissionError::EmptyRuntimeIdentity("execution_id"));
        }

        let (port_bindings, bundle_spec, confinement_type, sandbox_digest, policy_digest) =
            match &authority {
                RuntimeAuthority::Signed(verified) => (
                    &verified.port_bindings,
                    &verified.bundle_spec,
                    verified.confinement_type,
                    &verified.sandbox_digest,
                    &verified.policy_digest,
                ),
                RuntimeAuthority::Invocation(verified) => (
                    &verified.port_bindings,
                    &verified.bundle_spec,
                    verified.confinement_type,
                    &verified.sandbox_digest,
                    &verified.policy_digest,
                ),
            };

        for (supplied, _) in &entries {
            let expected = port_bindings
                .iter()
                .find(|admitted| admitted.slot == supplied.slot)
                .ok_or(RuntimeAdmissionError::BindingMismatch(supplied.slot))?;
            if expected != supplied {
                return Err(RuntimeAdmissionError::BindingMismatch(supplied.slot));
            }
        }

        let bundle = PortBundle::construct(bundle_spec, entries)?;
        let confinement = bundle
            .confinement()
            .ok_or(AdmissionError::ConfinementUnavailable)?
            .clone();
        let attestation = confinement
            .attest(ConfinementRequest {
                host_id: host_id.clone(),
                execution_id: execution_id.clone(),
                confinement_type,
                sandbox_digest: sandbox_digest.clone(),
                policy_digest: policy_digest.clone(),
            })
            .await?;

        if attestation.host_id != host_id {
            return Err(RuntimeAdmissionError::AttestationMismatch("host_id"));
        }
        if attestation.execution_id != execution_id {
            return Err(RuntimeAdmissionError::AttestationMismatch("execution_id"));
        }
        if attestation.confinement_type != confinement_type {
            return Err(RuntimeAdmissionError::AttestationMismatch(
                "confinement_type",
            ));
        }
        if attestation.sandbox_digest != *sandbox_digest {
            return Err(RuntimeAdmissionError::AttestationMismatch("sandbox_digest"));
        }
        if attestation.policy_digest != *policy_digest {
            return Err(RuntimeAdmissionError::AttestationMismatch("policy_digest"));
        }
        if attestation.attestation_id.trim().is_empty() {
            return Err(RuntimeAdmissionError::AttestationMismatch("attestation_id"));
        }
        if attestation.signature.trim().is_empty() {
            return Err(RuntimeAdmissionError::AttestationMismatch("signature"));
        }

        Ok(Self {
            authority,
            bundle,
            confinement_attestation: attestation,
            resource_ceilings,
        })
    }

    #[must_use]
    pub fn verified(&self) -> &VerifiedExecutionAdmission {
        match &self.authority {
            RuntimeAuthority::Signed(verified) => verified,
            RuntimeAuthority::Invocation(_) => {
                panic!("signed admission requested from invocation authority")
            }
        }
    }

    #[must_use]
    pub fn invocation_admission(&self) -> Option<&InvocationAdmission> {
        match &self.authority {
            RuntimeAuthority::Signed(_) => None,
            RuntimeAuthority::Invocation(verified) => Some(&verified.admission),
        }
    }

    #[must_use]
    pub fn resource_ceilings(&self) -> &ResourceCeilings {
        &self.resource_ceilings
    }

    #[must_use]
    pub fn confinement_attestation(&self) -> &ConfinementAttestation {
        &self.confinement_attestation
    }

    /// Transfer the frozen port bundle to the runtime instance constructor.
    #[must_use]
    pub fn into_bundle(self) -> PortBundle {
        self.bundle
    }
}

/// Sole checkpoint advancer for one program instance.
///
/// Exactly one advancer token exists per instance. Only a winning atomic commit
/// may advance the checkpoint sequence; crash, replay, lost-reply, and
/// outcome_unknown leave the sequence unchanged.
#[derive(Debug)]
pub struct CheckpointAdvancer {
    program_instance_ref: String,
    sequence: Mutex<u64>,
}

impl CheckpointAdvancer {
    #[must_use]
    pub fn new(program_instance_ref: impl Into<String>) -> Self {
        Self {
            program_instance_ref: program_instance_ref.into(),
            sequence: Mutex::new(0),
        }
    }

    #[must_use]
    pub fn program_instance_ref(&self) -> &str {
        &self.program_instance_ref
    }

    #[must_use]
    pub fn sequence(&self) -> u64 {
        *self.sequence.lock().expect("checkpoint sequence")
    }

    /// Attempt to mint a second advancer. Always refused — one advancer only.
    pub fn fork(&self) -> Result<(), AdmissionError> {
        Err(AdmissionError::AmbientAuthorityRefused(
            "checkpoint_advancer_fork_forbidden".into(),
        ))
    }

    /// Advance only after a winning atomic commit for this instance.
    ///
    /// Repeating the same committed version is idempotent, while skipping a
    /// version is rejected. This makes a lost reply safe for a caller that
    /// reconciles and re-presents the same commit result.
    pub fn advance_on_commit(
        &self,
        committed_instance_ref: &str,
        committed_state_version: u64,
    ) -> Result<u64, AdmissionError> {
        if committed_instance_ref != self.program_instance_ref {
            return Err(AdmissionError::AmbientAuthorityRefused(
                "checkpoint_advancer_instance_mismatch".into(),
            ));
        }
        let mut sequence = self.sequence.lock().expect("checkpoint sequence");
        if committed_state_version == *sequence {
            return Ok(*sequence);
        }
        let expected = sequence.saturating_add(1);
        if committed_state_version != expected {
            return Err(AdmissionError::CheckpointVersionMismatch {
                expected,
                actual: committed_state_version,
            });
        }
        *sequence = committed_state_version;
        Ok(*sequence)
    }
}

/// Parse a JSON value as Execution Admission, refusing product-plane fields first.
pub fn parse_execution_admission(value: &Value) -> Result<ExecutionAdmission, AdmissionError> {
    if let Some(object) = value.as_object() {
        for key in object.keys() {
            if FORBIDDEN_AMBIENT_KEYS.contains(&key.as_str()) {
                return Err(AdmissionError::ProductPlaneField(key.clone()));
            }
        }
    }
    serde_json::from_value(value.clone()).map_err(|error| {
        AdmissionError::SchemaMismatch(format!("execution admission decode failed: {error}"))
    })
}

/// Verify signature, expiry, nonce, audience, bindings, confinement, and ambient bans.
pub fn verify_execution_admission(
    admission: &ExecutionAdmission,
    keyring: &IssuerKeyring,
    nonce_ledger: &NonceLedger,
    expected_audience: &str,
    now_ms: u64,
    require_model_target: bool,
) -> Result<VerifiedExecutionAdmission, AdmissionError> {
    if admission.schema_version != EXECUTION_ADMISSION_SCHEMA {
        return Err(AdmissionError::SchemaMismatch(
            admission.schema_version.clone(),
        ));
    }
    if admission.issuer.trim().is_empty() {
        return Err(AdmissionError::IssuerEmpty);
    }
    for (field, value) in [
        ("admission_ref", admission.admission_ref.as_str()),
        ("artifact_ref", admission.artifact_ref.as_str()),
        ("invocation_ref", admission.invocation_ref.as_str()),
        ("issuer", admission.issuer.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(AdmissionError::EmptyReference(field));
        }
        if !is_identifier(value) {
            return Err(AdmissionError::InvalidReference(field));
        }
    }
    if admission.audience.trim().is_empty() {
        return Err(AdmissionError::EmptyAudience);
    }
    if admission.audience != expected_audience {
        return Err(AdmissionError::AudienceMismatch {
            expected: expected_audience.into(),
            actual: admission.audience.clone(),
        });
    }
    if admission.nonce.trim().is_empty() {
        return Err(AdmissionError::EmptyNonce);
    }
    if now_ms >= admission.expires_at_ms {
        return Err(AdmissionError::Expired {
            expires_at_ms: admission.expires_at_ms,
            now_ms,
        });
    }
    refuse_ambient(&admission.caller_correlations)?;
    if admission
        .confinement
        .confinement_type
        .eq_ignore_ascii_case("unconfined")
        || admission.confinement.sandbox_digest == "unconfined"
    {
        return Err(AdmissionError::UnconfinedForbidden);
    }
    if !is_digest(&admission.artifact_digest) {
        return Err(AdmissionError::MalformedDigest("artifact_digest"));
    }
    if !is_digest(&admission.context_digest) {
        return Err(AdmissionError::MalformedDigest("context_digest"));
    }
    if !is_digest(&admission.confinement.sandbox_digest) {
        return Err(AdmissionError::MalformedDigest("sandbox_digest"));
    }
    if !is_digest(&admission.confinement.policy_digest) {
        return Err(AdmissionError::MalformedDigest("policy_digest"));
    }

    verify_signature(keyring, admission, now_ms)?;
    let confinement_type = parse_confinement_type(&admission.confinement.confinement_type)?;
    let (port_bindings, bundle_spec) = resolve_exact_bindings(&admission.port_bindings)?;

    if let Some(target) = &admission.model_target {
        for (field, value) in [
            ("model_target_ref", target.model_target_ref.as_str()),
            ("model_deployment_ref", target.model_deployment_ref.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(AdmissionError::EmptyReference(field));
            }
            if !is_identifier(value) {
                return Err(AdmissionError::InvalidReference(field));
            }
        }
        if !is_digest(&target.exact_port_binding_digest) {
            return Err(AdmissionError::MalformedDigest("exact_port_binding_digest"));
        }
        let model_bound = port_bindings.iter().any(|binding| {
            binding.slot == PortSlot::ModelInference
                && binding.binding_digest == target.exact_port_binding_digest
        });
        if !model_bound {
            return Err(AdmissionError::ModelBindingMismatch);
        }
    } else if require_model_target {
        return Err(AdmissionError::MissingModelTarget);
    }

    // Confinement slot must be admitted exactly; absence fails closed.
    if !port_bindings
        .iter()
        .any(|binding| binding.slot == PortSlot::Confinement)
    {
        return Err(AdmissionError::ConfinementUnavailable);
    }

    // A nonce is consumed only after every admission invariant has passed. A
    // malformed or unbound record must not mutate the replay ledger.
    nonce_ledger.observe(&admission.nonce)?;

    Ok(VerifiedExecutionAdmission {
        admission: admission.clone(),
        port_bindings,
        bundle_spec,
        confinement_type,
        sandbox_digest: admission.confinement.sandbox_digest.clone(),
        policy_digest: admission.confinement.policy_digest.clone(),
    })
}

/// Resolve admitted Port bindings into exact runtime bindings. Missing required
/// slots and duplicate/ambiguous slots fail closed.
pub fn resolve_exact_bindings(
    admitted: &[AdmittedPortBinding],
) -> Result<(Vec<ExactPortBinding>, PortBundleSpec), AdmissionError> {
    let mut seen: HashSet<PortSlot> = HashSet::new();
    let mut bindings = Vec::with_capacity(admitted.len());
    let mut required = Vec::with_capacity(admitted.len());

    for entry in admitted {
        let slot = parse_port_slot(&entry.slot)?;
        if !seen.insert(slot) {
            return Err(AdmissionError::AmbiguousBinding(slot));
        }
        let expected_schema = expected_port_contract_schema(slot);
        if entry.port_contract_schema_id != expected_schema {
            return Err(AdmissionError::PortContractSchemaMismatch {
                slot,
                expected: expected_schema,
                actual: entry.port_contract_schema_id.clone(),
            });
        }
        if !is_digest(&entry.port_contract_digest) {
            return Err(AdmissionError::MalformedDigest("port_contract_digest"));
        }
        if !is_digest(&entry.binding_digest) {
            return Err(AdmissionError::MalformedDigest("binding_digest"));
        }
        if !is_digest(&entry.proof_digest) {
            return Err(AdmissionError::MalformedDigest("proof_digest"));
        }
        let contract = SchemaDigestRef {
            schema_id: entry.port_contract_schema_id.clone(),
            digest: entry.port_contract_digest.clone(),
        };
        required.push((slot, contract.clone()));
        bindings.push(ExactPortBinding {
            slot,
            port_contract: contract,
            binding_digest: entry.binding_digest.clone(),
            proof_digest: entry.proof_digest.clone(),
        });
    }

    if !seen.contains(&PortSlot::ExecutionCommit) {
        return Err(AdmissionError::MissingRequiredSlot(
            PortSlot::ExecutionCommit,
        ));
    }
    if !seen.contains(&PortSlot::Confinement) {
        return Err(AdmissionError::MissingRequiredSlot(PortSlot::Confinement));
    }

    Ok((bindings, PortBundleSpec::new(required)))
}

fn expected_port_contract_schema(slot: PortSlot) -> &'static str {
    match slot {
        PortSlot::ExecutionCommit => EXECUTION_COMMIT_PORT_SCHEMA,
        PortSlot::Confinement => CONFINEMENT_PORT_SCHEMA,
        PortSlot::ModelInference => MODEL_INFERENCE_PORT_SCHEMA,
        PortSlot::Capability => CAPABILITY_PORT_SCHEMA,
        PortSlot::ExternalAgentCapability => EXTERNAL_AGENT_PORT_SCHEMA,
        PortSlot::DurableEvent => DURABLE_EVENT_PORT_SCHEMA,
        PortSlot::ProgramComposition => PROGRAM_COMPOSITION_PORT_SCHEMA,
    }
}

fn refuse_ambient(correlations: &BTreeMap<String, String>) -> Result<(), AdmissionError> {
    for key in correlations.keys() {
        if FORBIDDEN_AMBIENT_KEYS.contains(&key.as_str()) {
            return Err(AdmissionError::AmbientAuthorityRefused(key.clone()));
        }
        let lower = key.to_ascii_lowercase();
        if lower.contains("secret")
            || lower.contains("password")
            || lower.contains("token")
            || lower.contains("credential")
        {
            return Err(AdmissionError::AmbientAuthorityRefused(key.clone()));
        }
    }
    for value in correlations.values() {
        let lower = value.to_ascii_lowercase();
        if lower == "unconfined" || lower.contains("://") || lower.starts_with('/') {
            return Err(AdmissionError::AmbientAuthorityRefused(value.clone()));
        }
    }
    Ok(())
}

fn parse_port_slot(slot: &str) -> Result<PortSlot, AdmissionError> {
    match slot {
        "execution_commit" => Ok(PortSlot::ExecutionCommit),
        "confinement" => Ok(PortSlot::Confinement),
        "model_inference" => Ok(PortSlot::ModelInference),
        "capability" => Ok(PortSlot::Capability),
        "external_agent_capability" => Ok(PortSlot::ExternalAgentCapability),
        "durable_event" => Ok(PortSlot::DurableEvent),
        "program_composition" => Ok(PortSlot::ProgramComposition),
        other => Err(AdmissionError::UnknownPortSlot(other.into())),
    }
}

fn parse_confinement_type(value: &str) -> Result<ConfinementType, AdmissionError> {
    match value {
        "WASM" => Ok(ConfinementType::Wasm),
        "DOCKER" => Ok(ConfinementType::Docker),
        "GVISOR" => Ok(ConfinementType::Gvisor),
        "FIRECRACKER" => Ok(ConfinementType::Firecracker),
        "NATIVE-SANDBOX" => Ok(ConfinementType::NativeSandbox),
        other => Err(AdmissionError::ConfinementTypeUnknown(other.into())),
    }
}

fn verify_signature(
    keyring: &IssuerKeyring,
    admission: &ExecutionAdmission,
    now_ms: u64,
) -> Result<(), AdmissionError> {
    let envelope = &admission.signature;
    if envelope.key_ref.is_empty() || envelope.signature.is_empty() {
        return Err(AdmissionError::Signature(
            SignatureRejection::EnvelopeAbsent,
        ));
    }
    if !is_identifier(&envelope.key_ref) {
        return Err(AdmissionError::Signature(
            SignatureRejection::SignatureMalformed,
        ));
    }
    if envelope.algorithm != ED25519 {
        return Err(AdmissionError::Signature(
            SignatureRejection::AlgorithmUnsupported,
        ));
    }
    let payload = signing_payload(admission).ok_or(AdmissionError::Signature(
        SignatureRejection::RecordNotSignable,
    ))?;
    let verifying_key = keyring.resolve(&envelope.key_ref, now_ms)?;
    let signature = BASE64
        .decode(&envelope.signature)
        .ok()
        .and_then(|bytes| Signature::from_slice(&bytes).ok())
        .ok_or(AdmissionError::Signature(
            SignatureRejection::SignatureMalformed,
        ))?;
    verifying_key
        .verify(&payload, &signature)
        .map_err(|_| AdmissionError::Signature(SignatureRejection::SignatureMismatch))?;
    let recomputed = content_digest(&payload);
    if recomputed != admission.admission_digest {
        return Err(AdmissionError::Signature(
            SignatureRejection::DigestMismatch,
        ));
    }
    Ok(())
}

fn signing_payload(admission: &ExecutionAdmission) -> Option<Vec<u8>> {
    let mut value = serde_json::to_value(admission).ok()?;
    let object = value.as_object_mut()?;
    object.remove("signature")?;
    object.remove("admission_digest")?;
    Some(canonical_json(&value).into_bytes())
}

fn content_digest(payload: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(payload))
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let body = entries
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_default(),
                        canonical_json(value)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        Value::Array(items) => {
            let body = items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{body}]")
        }
        _ => serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
    }
}

/// Helper for tests/fixtures: build an unsigned admission with placeholder digests.
#[must_use]
pub fn unsigned_admission_skeleton(
    invocation_ref: impl Into<String>,
    nonce: impl Into<String>,
    audience: impl Into<String>,
    expires_at_ms: u64,
    port_bindings: Vec<AdmittedPortBinding>,
) -> ExecutionAdmission {
    ExecutionAdmission {
        schema_version: EXECUTION_ADMISSION_SCHEMA.into(),
        admission_ref: "admission.1".into(),
        admission_digest: String::new(),
        artifact_ref: "artifact.1".into(),
        artifact_digest: digest_char('a'),
        invocation_ref: invocation_ref.into(),
        context_digest: digest_char('c'),
        resource_ceilings: ResourceCeilings {
            max_wall_ms: 60_000,
            max_memory_bytes: 64 * 1024 * 1024,
            max_effect_bytes: 1024 * 1024,
        },
        port_bindings,
        model_target: None,
        confinement: AdmittedConfinement {
            confinement_type: "NATIVE-SANDBOX".into(),
            sandbox_digest: digest_char('d'),
            policy_digest: digest_char('e'),
        },
        expires_at_ms,
        nonce: nonce.into(),
        issuer: "apxm.test-issuer".into(),
        audience: audience.into(),
        caller_correlations: BTreeMap::new(),
        signature: SignatureEnvelope {
            algorithm: ED25519.into(),
            key_ref: String::new(),
            signature: String::new(),
        },
    }
}

#[must_use]
pub fn digest_char(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

/// Build the minimal exact binding set required for G3 admission (commit + confinement).
#[must_use]
pub fn minimal_port_bindings() -> Vec<AdmittedPortBinding> {
    vec![
        AdmittedPortBinding {
            slot: "execution_commit".into(),
            port_contract_schema_id: EXECUTION_COMMIT_PORT_SCHEMA.into(),
            port_contract_digest: digest_char('1'),
            binding_digest: digest_char('2'),
            proof_digest: digest_char('3'),
        },
        AdmittedPortBinding {
            slot: "confinement".into(),
            port_contract_schema_id: CONFINEMENT_PORT_SCHEMA.into(),
            port_contract_digest: digest_char('4'),
            binding_digest: digest_char('5'),
            proof_digest: digest_char('6'),
        },
    ]
}
