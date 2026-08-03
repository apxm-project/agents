//! Product-neutral Execution Admission (agents synonym: Invocation Admission).
//!
//! This module freezes and enforces the P-006 admission contract from workspace
//! ADR-0030 / master-plan §5.2:
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
use apxm_program::grammar::is_digest;

use crate::bundle::{ExactPortBinding, PortBundleSpec, PortSlot};
use crate::confinement::ConfinementType;

/// Frozen schema id for the product-neutral Execution Admission envelope.
pub const EXECUTION_ADMISSION_SCHEMA: &str = "apxm.execution-admission.v1";

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

    pub fn from_keys(
        keys: impl IntoIterator<Item = IssuerKey>,
    ) -> Result<Self, AdmissionError> {
        let mut enrolled = BTreeMap::new();
        for key in keys {
            let key_ref = key.key_ref.clone();
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
    Expired { expires_at_ms: u64, now_ms: u64 },
    NonceReuse(String),
    AudienceMismatch { expected: String, actual: String },
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

/// Sole checkpoint advancer for one program instance.
///
/// Exactly one advancer token exists per instance. Only a winning atomic commit
/// may advance the checkpoint sequence; crash, replay, lost-reply, and
/// outcome_unknown leave the sequence unchanged.
#[derive(Debug)]
pub struct CheckpointAdvancer {
    program_instance_ref: String,
    sequence: Mutex<u64>,
    holders: Mutex<u32>,
}

impl CheckpointAdvancer {
    #[must_use]
    pub fn new(program_instance_ref: impl Into<String>) -> Self {
        Self {
            program_instance_ref: program_instance_ref.into(),
            sequence: Mutex::new(0),
            holders: Mutex::new(1),
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
        let holders = self.holders.lock().expect("advancer holders");
        if *holders != 1 {
            return Err(AdmissionError::AmbientAuthorityRefused(
                "multiple_checkpoint_advancers".into(),
            ));
        }
        Err(AdmissionError::AmbientAuthorityRefused(
            "checkpoint_advancer_fork_forbidden".into(),
        ))
    }

    /// Advance only after a winning atomic commit for this instance.
    pub fn advance_on_commit(&self, committed_instance_ref: &str) -> Result<u64, AdmissionError> {
        if committed_instance_ref != self.program_instance_ref {
            return Err(AdmissionError::AmbientAuthorityRefused(
                "checkpoint_advancer_instance_mismatch".into(),
            ));
        }
        let mut sequence = self.sequence.lock().expect("checkpoint sequence");
        *sequence = sequence.saturating_add(1);
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
    if admission.confinement.confinement_type.eq_ignore_ascii_case("unconfined")
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
    nonce_ledger.observe(&admission.nonce)?;

    let confinement_type = parse_confinement_type(&admission.confinement.confinement_type)?;
    let (port_bindings, bundle_spec) = resolve_exact_bindings(&admission.port_bindings)?;

    if require_model_target {
        let Some(target) = &admission.model_target else {
            return Err(AdmissionError::MissingModelTarget);
        };
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
    }

    // Confinement slot must be admitted exactly; absence fails closed.
    if !port_bindings
        .iter()
        .any(|binding| binding.slot == PortSlot::Confinement)
    {
        return Err(AdmissionError::ConfinementUnavailable);
    }

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
        return Err(AdmissionError::MissingRequiredSlot(PortSlot::ExecutionCommit));
    }
    if !seen.contains(&PortSlot::Confinement) {
        return Err(AdmissionError::MissingRequiredSlot(PortSlot::Confinement));
    }

    Ok((bindings, PortBundleSpec::new(required)))
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
        return Err(AdmissionError::Signature(SignatureRejection::EnvelopeAbsent));
    }
    if envelope.algorithm != ED25519 {
        return Err(AdmissionError::Signature(
            SignatureRejection::AlgorithmUnsupported,
        ));
    }
    let payload = signing_payload(admission)
        .ok_or(AdmissionError::Signature(SignatureRejection::RecordNotSignable))?;
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
        return Err(AdmissionError::Signature(SignatureRejection::DigestMismatch));
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
            port_contract_schema_id: "apxm.execution-commit.v1".into(),
            port_contract_digest: digest_char('1'),
            binding_digest: digest_char('2'),
            proof_digest: digest_char('3'),
        },
        AdmittedPortBinding {
            slot: "confinement".into(),
            port_contract_schema_id: "apxm.confinement.v1".into(),
            port_contract_digest: digest_char('4'),
            binding_digest: digest_char('5'),
            proof_digest: digest_char('6'),
        },
    ]
}
