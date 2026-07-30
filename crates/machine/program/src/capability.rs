//! `apxm.capability-invocation.v1` — the request contract at the exact
//! non-model Capability Port boundary.
//!
//! Application arguments are canonical typed data. Auth-owned identity and
//! authority remain separate opaque references supplied by Invocation
//! Admission; neither the runtime nor an adapter reconstructs them from the
//! arguments. The runtime adds stable Program Invocation, NodeExecution,
//! effect, request-digest, and idempotency facts before dispatch.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::common::{IdempotencyKey, TypedRef};

/// Maximum exact canonical argument bytes carried by one Capability request.
pub const MAX_CAPABILITY_ARGUMENT_BYTES: usize = 1_048_576;

/// Why a Capability request could not be prepared at the runtime boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapabilityRequestError {
    ArgumentsTooLarge {
        actual: usize,
        maximum: usize,
    },
    EmptyField {
        field: &'static str,
    },
    InvalidReferenceType {
        field: &'static str,
        expected: &'static str,
        actual: String,
    },
    InvalidIdentifier {
        field: &'static str,
    },
    NonCanonicalApprovalRefs,
    ArgumentDigestMismatch,
    EffectIdMismatch,
    RequestDigestMismatch,
    IdempotencyKeyMismatch {
        field: &'static str,
    },
}

impl std::fmt::Display for CapabilityRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArgumentsTooLarge { actual, maximum } => write!(
                f,
                "canonical Capability arguments are {actual} bytes; maximum is {maximum}"
            ),
            Self::EmptyField { field } => write!(f, "Capability request field {field} is empty"),
            Self::InvalidReferenceType {
                field,
                expected,
                actual,
            } => write!(
                f,
                "Capability request field {field} has reference type {actual}; expected {expected}"
            ),
            Self::InvalidIdentifier { field } => {
                write!(
                    f,
                    "Capability request field {field} is not a canonical identifier"
                )
            }
            Self::NonCanonicalApprovalRefs => {
                f.write_str("Capability approval references must be sorted and unique")
            }
            Self::ArgumentDigestMismatch => {
                f.write_str("Capability argument digest does not cover its canonical JSON bytes")
            }
            Self::EffectIdMismatch => {
                f.write_str("Capability effect id does not match its invocation coordinates")
            }
            Self::RequestDigestMismatch => {
                f.write_str("Capability request digest does not match its canonical identity")
            }
            Self::IdempotencyKeyMismatch { field } => {
                write!(
                    f,
                    "Capability idempotency field {field} does not match the request"
                )
            }
        }
    }
}

impl std::error::Error for CapabilityRequestError {}

/// The single accepted capability-invocation request schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityInvocationVersion {
    #[serde(rename = "apxm.capability-invocation.v1")]
    V1,
}

/// Canonical typed application arguments for one Capability effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalCapabilityArguments {
    type_ref: String,
    canonical_json: String,
    digest: String,
}

impl CanonicalCapabilityArguments {
    /// Canonicalize one typed application value and bind its exact bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityRequestError::ArgumentsTooLarge`] when the exact
    /// canonical bytes exceed the owner-contract ceiling.
    pub fn new(type_ref: impl Into<String>, value: Value) -> Result<Self, CapabilityRequestError> {
        let type_ref = type_ref.into();
        require_identifier("arguments.type_ref", &type_ref)?;
        let canonical_json = serde_json::to_string(&canonicalize_json(value))
            .expect("a serde_json::Value always serializes");
        if canonical_json.len() > MAX_CAPABILITY_ARGUMENT_BYTES {
            return Err(CapabilityRequestError::ArgumentsTooLarge {
                actual: canonical_json.len(),
                maximum: MAX_CAPABILITY_ARGUMENT_BYTES,
            });
        }
        let digest = sha256_digest(canonical_json.as_bytes());
        Ok(Self {
            type_ref,
            canonical_json,
            digest,
        })
    }

    /// The authored argument type carried by the AIR operand.
    #[must_use]
    pub fn type_ref(&self) -> &str {
        &self.type_ref
    }

    /// Exact canonical JSON application bytes exposed to the implementation.
    #[must_use]
    pub fn canonical_json(&self) -> &str {
        &self.canonical_json
    }

    /// Digest of the exact canonical argument bytes.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Decode the canonical application value.
    ///
    /// # Errors
    ///
    /// Returns the JSON decoder error if a value was constructed by unchecked
    /// deserialization rather than [`Self::new`].
    pub fn value(&self) -> Result<Value, serde_json::Error> {
        serde_json::from_str(&self.canonical_json)
    }

    fn validate(&self) -> Result<(), CapabilityRequestError> {
        require_identifier("arguments.type_ref", &self.type_ref)?;
        require_nonempty("arguments.canonical_json", &self.canonical_json)?;
        if self.canonical_json.len() > MAX_CAPABILITY_ARGUMENT_BYTES {
            return Err(CapabilityRequestError::ArgumentsTooLarge {
                actual: self.canonical_json.len(),
                maximum: MAX_CAPABILITY_ARGUMENT_BYTES,
            });
        }
        let value = serde_json::from_str(&self.canonical_json)
            .map_err(|_| CapabilityRequestError::ArgumentDigestMismatch)?;
        let canonical = serde_json::to_string(&canonicalize_json(value))
            .expect("a serde_json::Value always serializes");
        if canonical != self.canonical_json
            || self.digest != sha256_digest(self.canonical_json.as_bytes())
        {
            return Err(CapabilityRequestError::ArgumentDigestMismatch);
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for CanonicalCapabilityArguments {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            type_ref: String,
            canonical_json: String,
            digest: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let arguments = Self {
            type_ref: wire.type_ref,
            canonical_json: wire.canonical_json,
            digest: wire.digest,
        };
        arguments.validate().map_err(serde::de::Error::custom)?;
        Ok(arguments)
    }
}

/// Program Invocation and NodeExecution coordinates generated by the runtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityInvocationCorrelation {
    pub program_invocation_ref: TypedRef,
    pub node_execution_id: String,
}

/// Admitted authority and identity references for one Capability effect.
///
/// The references are opaque to Agents. Auth and Server own their contents and
/// validity; the runtime carries them to the effect boundary without widening
/// or reconstructing them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityInvocationAuthority {
    acting_principal_ref: TypedRef,
    agent_identity_ref: TypedRef,
    capability_grant_ref: TypedRef,
    approval_refs: Vec<TypedRef>,
}

impl CapabilityInvocationAuthority {
    /// Build the admitted reference set. Approval references are sorted and
    /// deduplicated so equivalent admission facts have one canonical request.
    ///
    /// # Errors
    ///
    /// Returns an error when any identity or authority reference is empty.
    pub fn new(
        acting_principal_ref: impl Into<String>,
        agent_identity_ref: impl Into<String>,
        capability_grant_ref: impl Into<String>,
        approval_refs: impl IntoIterator<Item = String>,
    ) -> Result<Self, CapabilityRequestError> {
        let acting_principal_ref = acting_principal_ref.into();
        let agent_identity_ref = agent_identity_ref.into();
        let capability_grant_ref = capability_grant_ref.into();
        require_identifier("authority.acting_principal_ref", &acting_principal_ref)?;
        require_identifier("authority.agent_identity_ref", &agent_identity_ref)?;
        require_identifier("authority.capability_grant_ref", &capability_grant_ref)?;
        let mut approval_refs: Vec<String> = approval_refs.into_iter().collect();
        for reference in &approval_refs {
            require_identifier("authority.approval_refs[]", reference)?;
        }
        approval_refs.sort();
        approval_refs.dedup();
        Ok(Self {
            acting_principal_ref: typed_ref("ActingPrincipalRef", acting_principal_ref),
            agent_identity_ref: typed_ref("AgentIdentityRef", agent_identity_ref),
            capability_grant_ref: typed_ref("CapabilityGrantRef", capability_grant_ref),
            approval_refs: approval_refs
                .into_iter()
                .map(|reference| typed_ref("ApprovalRef", reference))
                .collect(),
        })
    }

    #[must_use]
    pub fn acting_principal_ref(&self) -> &TypedRef {
        &self.acting_principal_ref
    }

    #[must_use]
    pub fn agent_identity_ref(&self) -> &TypedRef {
        &self.agent_identity_ref
    }

    #[must_use]
    pub fn capability_grant_ref(&self) -> &TypedRef {
        &self.capability_grant_ref
    }

    #[must_use]
    pub fn approval_refs(&self) -> &[TypedRef] {
        &self.approval_refs
    }

    /// Verify the exact reference vocabulary and canonical approval order.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty reference, wrong typed-reference kind, or
    /// approval list that is not strictly sorted and unique.
    pub fn validate(&self) -> Result<(), CapabilityRequestError> {
        validate_typed_ref(
            "authority.acting_principal_ref",
            "ActingPrincipalRef",
            &self.acting_principal_ref,
        )?;
        validate_typed_ref(
            "authority.agent_identity_ref",
            "AgentIdentityRef",
            &self.agent_identity_ref,
        )?;
        validate_typed_ref(
            "authority.capability_grant_ref",
            "CapabilityGrantRef",
            &self.capability_grant_ref,
        )?;
        for reference in &self.approval_refs {
            validate_typed_ref("authority.approval_refs[]", "ApprovalRef", reference)?;
        }
        if self
            .approval_refs
            .windows(2)
            .any(|pair| pair[0].target.as_str() >= pair[1].target.as_str())
        {
            return Err(CapabilityRequestError::NonCanonicalApprovalRefs);
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for CapabilityInvocationAuthority {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            acting_principal_ref: TypedRef,
            agent_identity_ref: TypedRef,
            capability_grant_ref: TypedRef,
            approval_refs: Vec<TypedRef>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let authority = Self {
            acting_principal_ref: wire.acting_principal_ref,
            agent_identity_ref: wire.agent_identity_ref,
            capability_grant_ref: wire.capability_grant_ref,
            approval_refs: wire.approval_refs,
        };
        authority.validate().map_err(serde::de::Error::custom)?;
        Ok(authority)
    }
}

/// Stable runtime-owned effect identity and replay facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityEffectFacts {
    pub effect_id: String,
    pub request_digest: String,
    pub idempotency_key: IdempotencyKey,
}

/// The complete request delivered to one exact admitted Capability Port.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequest {
    pub schema_version: CapabilityInvocationVersion,
    capability_ref: String,
    arguments: CanonicalCapabilityArguments,
    correlation: CapabilityInvocationCorrelation,
    authority: CapabilityInvocationAuthority,
    effect: CapabilityEffectFacts,
}

impl CapabilityRequest {
    /// Prepare one immutable Capability request from application data and
    /// already-admitted authority references.
    pub fn prepare(
        capability_ref: impl Into<String>,
        argument_type_ref: impl Into<String>,
        arguments: Value,
        program_invocation_ref: impl Into<String>,
        node_execution_id: impl Into<String>,
        authority: CapabilityInvocationAuthority,
    ) -> Result<Self, CapabilityRequestError> {
        let capability_ref = capability_ref.into();
        require_identifier("capability_ref", &capability_ref)?;
        let arguments = CanonicalCapabilityArguments::new(argument_type_ref, arguments)?;
        let program_invocation_ref = program_invocation_ref.into();
        require_identifier(
            "correlation.program_invocation_ref",
            &program_invocation_ref,
        )?;
        let node_execution_id = node_execution_id.into();
        require_identifier("correlation.node_execution_id", &node_execution_id)?;
        authority.validate()?;
        let correlation = CapabilityInvocationCorrelation {
            program_invocation_ref: typed_ref("ProgramInvocationRef", &program_invocation_ref),
            node_execution_id,
        };
        let effect_id = capability_effect_id(
            correlation.program_invocation_ref.target.as_str(),
            &correlation.node_execution_id,
        )?;
        let request_digest = capability_request_digest(&CapabilityRequestDigestInput {
            capability_ref: &capability_ref,
            arguments: &arguments,
            correlation: &correlation,
            authority: &authority,
            effect_id: &effect_id,
        })?;
        let effect = CapabilityEffectFacts {
            effect_id: effect_id.clone(),
            request_digest: request_digest.clone(),
            idempotency_key: IdempotencyKey {
                key_id: effect_id,
                scope_ref: program_invocation_ref,
                request_digest,
            },
        };
        let request = Self {
            schema_version: CapabilityInvocationVersion::V1,
            capability_ref,
            arguments,
            correlation,
            authority,
            effect,
        };
        request.validate()?;
        Ok(request)
    }

    #[must_use]
    pub fn capability_ref(&self) -> &str {
        &self.capability_ref
    }

    #[must_use]
    pub fn arguments(&self) -> &CanonicalCapabilityArguments {
        &self.arguments
    }

    #[must_use]
    pub fn correlation(&self) -> &CapabilityInvocationCorrelation {
        &self.correlation
    }

    #[must_use]
    pub fn authority(&self) -> &CapabilityInvocationAuthority {
        &self.authority
    }

    #[must_use]
    pub fn effect(&self) -> &CapabilityEffectFacts {
        &self.effect
    }

    /// Canonical owner-contract request digest with its `sha256:` prefix.
    #[must_use]
    pub fn request_digest(&self) -> &str {
        &self.effect.request_digest
    }

    /// The same request digest as bare lowercase SHA-256 hex for boundaries
    /// whose schema fixes the algorithm separately.
    #[must_use]
    pub fn request_digest_sha256_hex(&self) -> &str {
        self.effect
            .request_digest
            .strip_prefix("sha256:")
            .expect("validated Capability requests always carry a sha256 digest")
    }

    /// Validate identities, authority, canonical arguments, effect identity,
    /// request digest, and idempotency coordinates as one fail-closed unit.
    ///
    /// # Errors
    ///
    /// Returns the first contract invariant that the request violates.
    pub fn validate(&self) -> Result<(), CapabilityRequestError> {
        require_identifier("capability_ref", &self.capability_ref)?;
        self.arguments.validate()?;
        validate_typed_ref(
            "correlation.program_invocation_ref",
            "ProgramInvocationRef",
            &self.correlation.program_invocation_ref,
        )?;
        require_identifier(
            "correlation.node_execution_id",
            &self.correlation.node_execution_id,
        )?;
        self.authority.validate()?;
        let expected_effect_id = capability_effect_id(
            &self.correlation.program_invocation_ref.target,
            &self.correlation.node_execution_id,
        )?;
        if self.effect.effect_id != expected_effect_id {
            return Err(CapabilityRequestError::EffectIdMismatch);
        }
        let expected_request_digest = capability_request_digest(&CapabilityRequestDigestInput {
            capability_ref: &self.capability_ref,
            arguments: &self.arguments,
            correlation: &self.correlation,
            authority: &self.authority,
            effect_id: &self.effect.effect_id,
        })?;
        if self.effect.request_digest != expected_request_digest {
            return Err(CapabilityRequestError::RequestDigestMismatch);
        }
        if self.effect.idempotency_key.key_id != self.effect.effect_id {
            return Err(CapabilityRequestError::IdempotencyKeyMismatch { field: "key_id" });
        }
        if self.effect.idempotency_key.scope_ref != self.correlation.program_invocation_ref.target {
            return Err(CapabilityRequestError::IdempotencyKeyMismatch { field: "scope_ref" });
        }
        if self.effect.idempotency_key.request_digest != self.effect.request_digest {
            return Err(CapabilityRequestError::IdempotencyKeyMismatch {
                field: "request_digest",
            });
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for CapabilityRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema_version: CapabilityInvocationVersion,
            capability_ref: String,
            arguments: CanonicalCapabilityArguments,
            correlation: CapabilityInvocationCorrelation,
            authority: CapabilityInvocationAuthority,
            effect: CapabilityEffectFacts,
        }

        let wire = Wire::deserialize(deserializer)?;
        let request = Self {
            schema_version: wire.schema_version,
            capability_ref: wire.capability_ref,
            arguments: wire.arguments,
            correlation: wire.correlation,
            authority: wire.authority,
            effect: wire.effect,
        };
        request.validate().map_err(serde::de::Error::custom)?;
        Ok(request)
    }
}

/// The closed typed outcome of a Capability invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CapabilityOutcome {
    Completed { result: String },
    Failed { message: String },
    OutcomeUnknown { message: String },
}

/// Exact typed fields covered by the canonical Capability request digest.
///
/// This is the stable owner API for Server and other consumers that must bind
/// a Host or persistence request to the same Capability identity without
/// reproducing the JSON preimage or hash algorithm.
pub struct CapabilityRequestDigestInput<'a> {
    pub capability_ref: &'a str,
    pub arguments: &'a CanonicalCapabilityArguments,
    pub correlation: &'a CapabilityInvocationCorrelation,
    pub authority: &'a CapabilityInvocationAuthority,
    pub effect_id: &'a str,
}

#[derive(Serialize)]
struct CapabilityRequestIdentity<'a> {
    schema_version: &'static str,
    capability_ref: &'a str,
    arguments: &'a CanonicalCapabilityArguments,
    correlation: &'a CapabilityInvocationCorrelation,
    authority: &'a CapabilityInvocationAuthority,
    effect_id: &'a str,
}

/// Compute the canonical stable Capability effect identity.
///
/// # Errors
///
/// Returns an error when either invocation coordinate is empty.
pub fn capability_effect_id(
    program_invocation_ref: &str,
    node_execution_id: &str,
) -> Result<String, CapabilityRequestError> {
    require_identifier("correlation.program_invocation_ref", program_invocation_ref)?;
    require_identifier("correlation.node_execution_id", node_execution_id)?;
    let mut hasher = Sha256::new();
    hasher.update(b"apxm.capability-effect.v1\0");
    hasher.update(program_invocation_ref.as_bytes());
    hasher.update(b"\0");
    hasher.update(node_execution_id.as_bytes());
    Ok(format!("capability-effect.{:x}", hasher.finalize()))
}

/// Compute the SHA-256 digest of the exact canonical request-identity JSON.
///
/// # Errors
///
/// Returns an error when an identity input is empty, invalid, noncanonical, or
/// carries an effect id that does not match its invocation coordinates.
pub fn capability_request_digest(
    input: &CapabilityRequestDigestInput<'_>,
) -> Result<String, CapabilityRequestError> {
    require_identifier("capability_ref", input.capability_ref)?;
    input.arguments.validate()?;
    validate_typed_ref(
        "correlation.program_invocation_ref",
        "ProgramInvocationRef",
        &input.correlation.program_invocation_ref,
    )?;
    require_identifier(
        "correlation.node_execution_id",
        &input.correlation.node_execution_id,
    )?;
    input.authority.validate()?;
    if input.effect_id
        != capability_effect_id(
            &input.correlation.program_invocation_ref.target,
            &input.correlation.node_execution_id,
        )?
    {
        return Err(CapabilityRequestError::EffectIdMismatch);
    }
    let identity = CapabilityRequestIdentity {
        schema_version: "apxm.capability-request-identity.v1",
        capability_ref: input.capability_ref,
        arguments: input.arguments,
        correlation: input.correlation,
        authority: input.authority,
        effect_id: input.effect_id,
    };
    let value = serde_json::to_value(identity).expect("Capability request identity serializes");
    let bytes = serde_json::to_vec(&canonicalize_json(value))
        .expect("Capability request identity JSON serializes");
    Ok(sha256_digest(&bytes))
}

/// Compute the same canonical request digest as bare lowercase SHA-256 hex.
///
/// Use this representation only at a boundary whose exact contract fixes the
/// algorithm separately and therefore omits the `sha256:` prefix.
///
/// # Errors
///
/// Returns the same validation errors as [`capability_request_digest`].
pub fn capability_request_digest_sha256_hex(
    input: &CapabilityRequestDigestInput<'_>,
) -> Result<String, CapabilityRequestError> {
    let digest = capability_request_digest(input)?;
    Ok(digest
        .strip_prefix("sha256:")
        .expect("capability_request_digest always emits a sha256 prefix")
        .to_string())
}

fn typed_ref(ref_type: &str, reference: impl Into<String>) -> TypedRef {
    TypedRef {
        ref_type: ref_type.to_string(),
        target: reference.into(),
        digest: None,
    }
}

fn require_nonempty(field: &'static str, value: &str) -> Result<(), CapabilityRequestError> {
    if value.is_empty() {
        Err(CapabilityRequestError::EmptyField { field })
    } else {
        Ok(())
    }
}

fn require_identifier(field: &'static str, value: &str) -> Result<(), CapabilityRequestError> {
    require_nonempty(field, value)?;
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err(CapabilityRequestError::EmptyField { field });
    };
    let valid_first = first.is_ascii_alphanumeric();
    let valid_rest = bytes.all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'@' | b'-')
    });
    if value.len() > 256 || !valid_first || !valid_rest {
        return Err(CapabilityRequestError::InvalidIdentifier { field });
    }
    Ok(())
}

fn validate_typed_ref(
    field: &'static str,
    expected: &'static str,
    reference: &TypedRef,
) -> Result<(), CapabilityRequestError> {
    require_identifier(field, &reference.target)?;
    if reference.ref_type != expected {
        return Err(CapabilityRequestError::InvalidReferenceType {
            field,
            expected,
            actual: reference.ref_type.clone(),
        });
    }
    Ok(())
}

fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        Value::Object(values) => {
            let mut entries: Vec<(String, Value)> = values.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut canonical = Map::new();
            for (key, value) in entries {
                canonical.insert(key, canonicalize_json(value));
            }
            Value::Object(canonical)
        }
        scalar => scalar,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn authority() -> CapabilityInvocationAuthority {
        CapabilityInvocationAuthority::new(
            "principal.user.1",
            "agent.gao.1",
            "grant.search.1",
            [
                "approval.search.2".to_string(),
                "approval.search.1".to_string(),
            ],
        )
        .expect("valid authority")
    }

    #[test]
    fn request_separates_canonical_arguments_from_authority_and_identity() {
        let request = CapabilityRequest::prepare(
            "cap.search",
            "SearchArguments",
            json!({"z": 1, "query": "release checklist"}),
            "invocation.1",
            "node-execution.1",
            authority(),
        )
        .expect("bounded arguments");

        assert_eq!(
            request.arguments().canonical_json(),
            r#"{"query":"release checklist","z":1}"#
        );
        assert_eq!(
            request.authority().acting_principal_ref().ref_type,
            "ActingPrincipalRef"
        );
        assert_eq!(
            request.authority().agent_identity_ref().target,
            "agent.gao.1"
        );
        assert_eq!(
            request.authority().capability_grant_ref().target,
            "grant.search.1"
        );
        assert_eq!(
            request
                .authority()
                .approval_refs()
                .iter()
                .map(|reference| reference.target.as_str())
                .collect::<Vec<_>>(),
            vec!["approval.search.1", "approval.search.2"]
        );
    }

    #[test]
    fn effect_identity_is_stable_and_request_sensitive() {
        let first = CapabilityRequest::prepare(
            "cap.search",
            "SearchArguments",
            json!({"query": "one"}),
            "invocation.1",
            "node-execution.1",
            authority(),
        )
        .expect("bounded arguments");
        let replay = CapabilityRequest::prepare(
            "cap.search",
            "SearchArguments",
            json!({"query": "one"}),
            "invocation.1",
            "node-execution.1",
            authority(),
        )
        .expect("bounded arguments");
        let changed = CapabilityRequest::prepare(
            "cap.search",
            "SearchArguments",
            json!({"query": "two"}),
            "invocation.1",
            "node-execution.1",
            authority(),
        )
        .expect("bounded arguments");

        assert_eq!(first.effect(), replay.effect());
        assert_eq!(first.effect().effect_id, changed.effect().effect_id);
        assert_ne!(
            first.effect().request_digest,
            changed.effect().request_digest
        );
        assert_eq!(
            first.effect().idempotency_key.request_digest,
            first.effect().request_digest
        );
    }

    #[test]
    fn canonical_vector_pins_effect_and_request_digest_preimages() {
        let request = CapabilityRequest::prepare(
            "cap.search",
            "SearchArguments",
            json!({"query": "release checklist"}),
            "invocation.1",
            "node-execution.invocation.1.cap.search.4",
            CapabilityInvocationAuthority::new(
                "principal.user.1",
                "agent.gao.1",
                "grant.search.1",
                ["approval.search.1".to_string()],
            )
            .expect("valid vector authority"),
        )
        .expect("valid vector request");

        assert_eq!(
            request.effect().effect_id,
            "capability-effect.9b3e87bae20196a18df6675dc0c16806bd54ef328c002a0465a21aca5e78284b"
        );
        assert_eq!(
            request.effect().request_digest,
            "sha256:4efb35bfeb410d86cd72a35d4210f442a44cb7d0fc4dc676785184bc4be39fe6"
        );
        let digest_input = CapabilityRequestDigestInput {
            capability_ref: request.capability_ref(),
            arguments: request.arguments(),
            correlation: request.correlation(),
            authority: request.authority(),
            effect_id: &request.effect().effect_id,
        };
        assert_eq!(
            capability_request_digest_sha256_hex(&digest_input)
                .expect("valid bare digest representation"),
            "4efb35bfeb410d86cd72a35d4210f442a44cb7d0fc4dc676785184bc4be39fe6"
        );
    }

    #[test]
    fn constructors_reject_empty_identity_and_authority_references() {
        for (acting, agent, grant, approvals, expected_field) in [
            (
                "",
                "agent.1",
                "grant.1",
                Vec::new(),
                "authority.acting_principal_ref",
            ),
            (
                "principal.1",
                "",
                "grant.1",
                Vec::new(),
                "authority.agent_identity_ref",
            ),
            (
                "principal.1",
                "agent.1",
                "",
                Vec::new(),
                "authority.capability_grant_ref",
            ),
            (
                "principal.1",
                "agent.1",
                "grant.1",
                vec![String::new()],
                "authority.approval_refs[]",
            ),
        ] {
            assert_eq!(
                CapabilityInvocationAuthority::new(acting, agent, grant, approvals),
                Err(CapabilityRequestError::EmptyField {
                    field: expected_field,
                })
            );
        }

        assert_eq!(
            CapabilityInvocationAuthority::new(
                "principal with spaces",
                "agent.1",
                "grant.1",
                Vec::new(),
            ),
            Err(CapabilityRequestError::InvalidIdentifier {
                field: "authority.acting_principal_ref",
            })
        );
    }

    #[test]
    fn request_preparation_rejects_empty_owned_coordinates() {
        for (capability, argument_type, invocation, node, expected_field) in [
            ("", "Arguments", "invocation.1", "node.1", "capability_ref"),
            ("cap.1", "", "invocation.1", "node.1", "arguments.type_ref"),
            (
                "cap.1",
                "Arguments",
                "",
                "node.1",
                "correlation.program_invocation_ref",
            ),
            (
                "cap.1",
                "Arguments",
                "invocation.1",
                "",
                "correlation.node_execution_id",
            ),
        ] {
            assert_eq!(
                CapabilityRequest::prepare(
                    capability,
                    argument_type,
                    json!({}),
                    invocation,
                    node,
                    authority(),
                ),
                Err(CapabilityRequestError::EmptyField {
                    field: expected_field,
                })
            );
        }
    }

    #[test]
    fn deserialization_rejects_wrong_and_noncanonical_authority_refs() {
        let mut value = serde_json::to_value(authority()).expect("serialize authority");
        value["acting_principal_ref"]["ref_type"] = json!("AgentIdentityRef");
        assert!(serde_json::from_value::<CapabilityInvocationAuthority>(value).is_err());

        let mut value = serde_json::to_value(authority()).expect("serialize authority");
        value["approval_refs"] = json!([
            {"ref_type": "ApprovalRef", "ref": "approval.search.2"},
            {"ref_type": "ApprovalRef", "ref": "approval.search.1"}
        ]);
        assert!(serde_json::from_value::<CapabilityInvocationAuthority>(value).is_err());
    }

    #[test]
    fn request_deserialization_recomputes_effect_and_request_identity() {
        let request = CapabilityRequest::prepare(
            "cap.search",
            "SearchArguments",
            json!({"query": "release checklist"}),
            "invocation.1",
            "node-execution.1",
            authority(),
        )
        .expect("valid request");
        let mut value = serde_json::to_value(&request).expect("serialize request");
        value["effect"]["request_digest"] =
            json!("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(serde_json::from_value::<CapabilityRequest>(value).is_err());
    }

    #[test]
    fn capability_outcome_uses_the_closed_contract_discriminant() {
        assert_eq!(
            serde_json::to_value(CapabilityOutcome::OutcomeUnknown {
                message: "completion cannot be proven".into(),
            })
            .expect("serialize"),
            json!({
                "outcome": "outcome_unknown",
                "message": "completion cannot be proven"
            })
        );
    }

    #[test]
    fn oversized_arguments_fail_before_effect_identity_is_prepared() {
        let error = CapabilityRequest::prepare(
            "cap.search",
            "SearchArguments",
            Value::String("x".repeat(MAX_CAPABILITY_ARGUMENT_BYTES)),
            "invocation.1",
            "node-execution.1",
            authority(),
        )
        .expect_err("JSON quoting pushes the value over the byte ceiling");

        assert!(matches!(
            error,
            CapabilityRequestError::ArgumentsTooLarge {
                maximum: MAX_CAPABILITY_ARGUMENT_BYTES,
                ..
            }
        ));
    }
}
