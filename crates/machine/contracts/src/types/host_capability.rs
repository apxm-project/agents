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
        assert_eq!(host_capability_id_of("host:notes.search"), Some("notes.search"));
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
