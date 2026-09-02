//! Host-fulfilled Capability declarations, requests and outcomes.
//!
//! A host capability is the second carrier for a non-builtin Capability
//! (ADR-0025). A package declares one in `agent.toml` as
//! `[[capabilities.host]]`; authored source refers to it as
//! `Capability("host:<id>")`; the runtime never executes it. This module owns
//! the one typed projection of `apxm.host-capability.v1` that the manifest
//! loader, the compilation service, the runtime and the CLI all decode, so the
//! reserved prefix and the declaration grammar are stated exactly once.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Published schema identity of every document in this module.
pub const HOST_CAPABILITY_SCHEMA: &str = "apxm.host-capability.v1";

/// The reserved namespace an authored reference uses for a declared host
/// capability. No builtin id and no shipped handler directory may occupy it.
pub const HOST_CAPABILITY_REF_PREFIX: &str = "host:";

/// The greatest number of host capabilities one package may declare.
pub const MAX_HOST_CAPABILITIES: usize = 256;

/// The greatest length of a declared id, matching the published schema.
pub const MAX_HOST_CAPABILITY_ID_LENGTH: usize = 200;

/// What one call does to the system behind the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCapabilityEffect {
    /// The call observes and changes nothing.
    Read,
    /// The call changes the system behind the host.
    Write,
}

impl HostCapabilityEffect {
    /// The canonical wire string.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// One host capability a package declares it needs.
///
/// The two schemas are carried verbatim: APXM does not evaluate them, and the
/// host is the party that holds an input against `input_schema` and an output
/// against `output_schema`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilityDeclaration {
    /// The declared id, without the reserved prefix.
    pub id: String,
    /// What one call does to the system behind the host.
    pub effect: HostCapabilityEffect,
    /// The JSON Schema the host holds one input against.
    pub input_schema: serde_json::Map<String, serde_json::Value>,
    /// The JSON Schema the host holds one output against.
    pub output_schema: serde_json::Map<String, serde_json::Value>,
}

impl HostCapabilityDeclaration {
    /// The authored reference form of this declaration.
    #[must_use]
    pub fn capability_ref(&self) -> String {
        host_capability_ref(&self.id)
    }
}

/// Why one declaration set is not admissible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostCapabilityDeclarationError {
    /// The id is not the published lowercase dotted grammar.
    InvalidId { id: String },
    /// Two declarations claim the same id.
    DuplicateId { id: String },
    /// The set is larger than the published ceiling.
    TooMany { actual: usize },
}

impl std::fmt::Display for HostCapabilityDeclarationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidId { id } => write!(
                formatter,
                "host capability id '{id}' must be lowercase dotted segments of \
                 [a-z0-9_-] beginning with a letter or digit"
            ),
            Self::DuplicateId { id } => {
                write!(formatter, "host capability id '{id}' is declared twice")
            }
            Self::TooMany { actual } => write!(
                formatter,
                "a package declares at most {MAX_HOST_CAPABILITIES} host capabilities, not {actual}"
            ),
        }
    }
}

impl std::error::Error for HostCapabilityDeclarationError {}

/// Whether `id` is the published declared-id grammar.
#[must_use]
pub fn is_host_capability_id(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_HOST_CAPABILITY_ID_LENGTH {
        return false;
    }
    id.split('.').all(|segment| {
        let mut characters = segment.chars();
        let Some(first) = characters.next() else {
            return false;
        };
        (first.is_ascii_lowercase() || first.is_ascii_digit())
            && characters.all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || character == '_'
                    || character == '-'
            })
    })
}

/// The authored reference form of one declared id.
#[must_use]
pub fn host_capability_ref(id: &str) -> String {
    format!("{HOST_CAPABILITY_REF_PREFIX}{id}")
}

/// Whether `capability_ref` names the host-fulfilled namespace at all. A
/// reference in this namespace is never executed by APXM, whether or not the
/// package declared it.
#[must_use]
pub fn is_host_capability_ref(capability_ref: &str) -> bool {
    capability_ref.starts_with(HOST_CAPABILITY_REF_PREFIX)
}

/// The declared id inside one authored reference, if it is in the namespace.
#[must_use]
pub fn host_capability_id_of(capability_ref: &str) -> Option<&str> {
    capability_ref.strip_prefix(HOST_CAPABILITY_REF_PREFIX)
}

/// Admit one declaration set and project it to the references it mints.
///
/// # Errors
///
/// Returns the first reason the set is not admissible: an id outside the
/// published grammar, a duplicated id, or a set past the published ceiling.
pub fn minted_host_capability_refs(
    declarations: &[HostCapabilityDeclaration],
) -> Result<BTreeSet<String>, HostCapabilityDeclarationError> {
    if declarations.len() > MAX_HOST_CAPABILITIES {
        return Err(HostCapabilityDeclarationError::TooMany {
            actual: declarations.len(),
        });
    }
    let mut minted = BTreeSet::new();
    for declaration in declarations {
        if !is_host_capability_id(&declaration.id) {
            return Err(HostCapabilityDeclarationError::InvalidId {
                id: declaration.id.clone(),
            });
        }
        if !minted.insert(declaration.capability_ref()) {
            return Err(HostCapabilityDeclarationError::DuplicateId {
                id: declaration.id.clone(),
            });
        }
    }
    Ok(minted)
}

/// The prefix every host capability request identity carries.
pub const HOST_CAPABILITY_REQUEST_PREFIX: &str = "capability-request.";

/// The deterministic identity of one host-fulfilled Capability request.
///
/// Derived from the invocation and node-execution coordinates of the node that
/// published it, so a re-park after a restart republishes the same id rather
/// than minting a second request for one effect. That is what makes a host's
/// settlement idempotent without the host having to remember anything.
#[must_use]
pub fn host_capability_request_id(program_invocation_ref: &str, node_execution_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"apxm.host-capability-request\0");
    hasher.update(program_invocation_ref.as_bytes());
    hasher.update(b"\0");
    hasher.update(node_execution_id.as_bytes());
    format!("{HOST_CAPABILITY_REQUEST_PREFIX}{:x}", hasher.finalize())
}

/// Whether `value` is shaped like a host capability request identity. The
/// runtime recognizes a parked continuation as a host capability park by this
/// alone, so the shape is stated once here.
#[must_use]
pub fn is_host_capability_request_id(value: &str) -> bool {
    value
        .strip_prefix(HOST_CAPABILITY_REQUEST_PREFIX)
        .is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
}

/// How one host-fulfilled Capability request settled.
///
/// `Cancelled` is recorded by APXM when the request or its invocation was
/// cancelled; it is not a settlement a host sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCapabilityOutcomeKind {
    Ok,
    Denied,
    Failed,
    Unknown,
    Cancelled,
}

impl HostCapabilityOutcomeKind {
    /// The canonical wire string.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "denied",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::Cancelled => "cancelled",
        }
    }
}

/// One settled host capability request, as the runtime delivers it into the
/// parked node. This is the `HostCapabilityOutcome` document of
/// `apxm.host-capability.v1`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilitySettlement {
    pub schema_version: String,
    pub capability_request_id: String,
    pub outcome: HostCapabilityOutcomeKind,
    /// Present only for `ok`: the exact canonical JSON output bytes, as text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl HostCapabilitySettlement {
    /// Seal one settlement for `capability_request_id`.
    #[must_use]
    pub fn new(
        capability_request_id: impl Into<String>,
        outcome: HostCapabilityOutcomeKind,
        output: Option<String>,
        receipt_ref: Option<String>,
        message: Option<String>,
    ) -> Self {
        Self {
            schema_version: HOST_CAPABILITY_SCHEMA.to_owned(),
            capability_request_id: capability_request_id.into(),
            outcome,
            output: if outcome == HostCapabilityOutcomeKind::Ok {
                output
            } else {
                None
            },
            receipt_ref,
            message,
        }
    }

    /// Whether this settlement states the shape its outcome requires.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        self.schema_version == HOST_CAPABILITY_SCHEMA
            && is_host_capability_request_id(&self.capability_request_id)
            && (self.outcome == HostCapabilityOutcomeKind::Ok) == self.output.is_some()
    }
}

/// The `[capabilities]` table of `agent.toml`.
///
/// Only `host` is a member. A shipped handler stays declared by shipping
/// `capabilities/<id>/handler.py` or `.ts`, so the two carriers cannot be
/// confused for one another.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestCapabilities {
    /// The host capabilities this package declares it needs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host: Vec<HostCapabilityDeclaration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(id: &str) -> HostCapabilityDeclaration {
        HostCapabilityDeclaration {
            id: id.to_owned(),
            effect: HostCapabilityEffect::Read,
            input_schema: serde_json::Map::new(),
            output_schema: serde_json::Map::new(),
        }
    }

    #[test]
    fn a_declared_id_mints_the_prefixed_reference() {
        assert_eq!(
            declaration("notes.search").capability_ref(),
            "host:notes.search"
        );
        assert_eq!(
            host_capability_id_of("host:notes.search"),
            Some("notes.search")
        );
        assert_eq!(host_capability_id_of("read"), None);
    }

    #[test]
    fn the_declared_id_grammar_is_lowercase_dotted_segments() {
        assert!(is_host_capability_id("notes"));
        assert!(is_host_capability_id("sale_order.confirm"));
        assert!(is_host_capability_id("sale-orders.list"));
        assert!(!is_host_capability_id("Notes.Search"));
        assert!(!is_host_capability_id("host:notes"));
        assert!(!is_host_capability_id("notes."));
        assert!(!is_host_capability_id(".notes"));
        assert!(!is_host_capability_id("_notes"));
        assert!(!is_host_capability_id(""));
    }

    #[test]
    fn a_duplicated_declaration_is_refused_rather_than_collapsed() {
        let error = minted_host_capability_refs(&[declaration("notes"), declaration("notes")])
            .expect_err("a duplicate id is not admissible");
        assert_eq!(
            error,
            HostCapabilityDeclarationError::DuplicateId {
                id: "notes".to_owned()
            }
        );
    }

    #[test]
    fn a_request_identity_is_derived_from_the_node_that_published_it() {
        let first = host_capability_request_id("invocation.1", "node-execution.1");
        assert!(is_host_capability_request_id(&first));
        assert_eq!(
            first,
            host_capability_request_id("invocation.1", "node-execution.1"),
            "a re-park republishes one request rather than minting a second"
        );
        assert_ne!(
            first,
            host_capability_request_id("invocation.1", "node-execution.2")
        );
        assert_ne!(
            first,
            host_capability_request_id("invocation.2", "node-execution.1")
        );
        assert!(!is_host_capability_request_id("capability-request.short"));
        assert!(!is_host_capability_request_id("evt-1"));
    }

    #[test]
    fn only_an_ok_settlement_carries_an_output() {
        let request_id = host_capability_request_id("invocation.1", "node-execution.1");
        let settled = HostCapabilitySettlement::new(
            request_id.clone(),
            HostCapabilityOutcomeKind::Ok,
            Some("{}".to_owned()),
            None,
            None,
        );
        assert!(settled.is_well_formed());
        let denied = HostCapabilitySettlement::new(
            request_id,
            HostCapabilityOutcomeKind::Denied,
            Some("{}".to_owned()),
            None,
            Some("policy refused the write effect".to_owned()),
        );
        assert!(denied.output.is_none());
        assert!(denied.is_well_formed());
    }

    #[test]
    fn an_id_outside_the_grammar_is_refused_before_it_mints_anything() {
        let error = minted_host_capability_refs(&[declaration("Notes")])
            .expect_err("an invalid id is not admissible");
        assert_eq!(
            error,
            HostCapabilityDeclarationError::InvalidId {
                id: "Notes".to_owned()
            }
        );
    }
}
