//! Portable frontend handler-manifest contract.
//!
//! TypeScript package build tooling emits this versioned artifact sidecar for
//! every packaged tool and hook. The manifest carries only artifact-local
//! source, never a build-host path.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Component, Path};
use thiserror::Error;

/// Version identifier for the frontend handler sidecar.
pub const HANDLER_MANIFEST_VERSION: &str = "apxm.handler-manifest.v1";
/// Artifact section kind carrying the frontend handler sidecar.
pub const HANDLER_MANIFEST_ARTIFACT_SECTION: &str = "apxm.handler_manifest.v1";
/// AIR comment prefix carrying a handler manifest before artifact creation.
pub const HANDLER_MANIFEST_AIR_SIDECAR_PREFIX: &str = "; __apxm_handler_manifest__ ";
/// Artifact-local directory holding embedded executable handler modules.
pub const HANDLER_MANIFEST_SOURCE_DIRECTORY: &str = "handlers";
/// Prefix for content-addressed handler identities.
pub const HANDLER_MANIFEST_HANDLER_ID_PREFIX: &str = "sha256:";
/// Hexadecimal character count in a SHA-256 handler identity.
pub const HANDLER_MANIFEST_HANDLER_ID_HEX_LENGTH: usize = 64;
/// Maximum character count of a `module`, `qualname`, or `name` identifier.
pub const HANDLER_MANIFEST_IDENTIFIER_MAX_LENGTH: usize = 256;
/// The closed set of hook dispatch modes.
pub const HANDLER_MANIFEST_HOOK_MODES: [&str; 2] = ["observe", "gate"];

/// The authoring language for the private package-handler worker.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandlerLanguage {
    /// TypeScript handler source.
    #[serde(rename = "typescript")]
    TypeScript,
}

/// The runtime role of a handler.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandlerKind {
    /// A capability-addressable tool.
    Tool,
    /// A lifecycle hook addressed by handler ID.
    Hook,
}

/// Source material stored beside a handler descriptor in an artifact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HandlerSource {
    /// Relative path used when materializing this source for a worker.
    pub artifact_path: String,
    /// UTF-8 source text transported inside the compiled artifact.
    pub content: String,
}

/// One portable tool or hook descriptor.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HandlerDescriptor {
    /// Tool versus hook dispatch semantics.
    pub kind: HandlerKind,
    /// Language worker that evaluates the source.
    pub language: HandlerLanguage,
    /// Content-addressed handler identity.
    pub handler_id: String,
    /// Author-facing module identifier.
    pub module: String,
    /// Exported callable name inside the module.
    pub qualname: String,
    /// Capability name for tools or display name for hooks.
    pub name: String,
    /// Artifact-local executable source.
    pub source: HandlerSource,
    /// Handler description. Absent when the producer supplies none; an absent
    /// description is never materialized as an empty one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Tool argument schema. Absent for hooks, which carry no argument shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
    /// Whether a tool is guaranteed not to mutate external state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    /// Whether invoking a tool requires an explicit approval decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_approval: Option<bool>,
    /// Hook lifecycle event. Absent for tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// Hook matcher. Absent for tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#match: Option<String>,
    /// Hook mode. Absent for tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// The sole serialized shape for frontend tool and hook metadata.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HandlerManifest {
    /// Fixed schema version.
    pub version: String,
    /// Every tool and hook referenced by the compiled artifact.
    pub handlers: Vec<HandlerDescriptor>,
}

/// Handler-manifest validation failure.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum HandlerManifestError {
    /// The schema version is not supported.
    #[error("unsupported handler manifest version '{0}'")]
    UnsupportedVersion(String),
    /// A descriptor lacks a required identifying field.
    #[error("handler descriptor has empty {0}")]
    EmptyField(&'static str),
    /// A handler ID is not a canonical SHA-256 identity.
    #[error("handler '{0}' has an invalid handler_id")]
    InvalidHandlerId(String),
    /// The artifact path is not a safe local relative path.
    #[error("handler '{0}' has an invalid artifact-local source path '{1}'")]
    InvalidArtifactPath(String, String),
    /// Two descriptors claim the same stable identity.
    #[error("duplicate handler_id '{0}'")]
    DuplicateHandlerId(String),
    /// Two tools claim the same capability name.
    #[error("duplicate tool name '{0}'")]
    DuplicateToolName(String),
    /// A tool descriptor is missing a JSON object schema.
    #[error("tool '{0}' must carry an object schema")]
    InvalidToolSchema(String),
    /// A hook descriptor omits an event, matcher, or mode.
    #[error("hook '{0}' must carry event, match, and mode")]
    IncompleteHook(String),
    /// A `module`, `qualname`, or `name` is not a contract identifier.
    #[error("handler '{0}' has an invalid {1} '{2}'")]
    InvalidIdentifier(String, &'static str, String),
    /// A hook mode is outside the closed dispatch set.
    #[error("hook '{0}' has an unsupported mode '{1}'")]
    InvalidHookMode(String, String),
    /// A tool carries hook lifecycle fields, or a hook carries tool fields.
    #[error("handler '{0}' carries the field '{1}', which belongs only to a {2}")]
    CrossKindField(String, &'static str, &'static str),
}

impl HandlerManifest {
    /// Create a manifest using the current schema version.
    pub fn new(handlers: Vec<HandlerDescriptor>) -> Self {
        Self {
            version: HANDLER_MANIFEST_VERSION.to_string(),
            handlers,
        }
    }

    /// Decode and validate a serialized manifest.
    pub fn from_json_slice(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
    }

    /// Enforce the portable cross-language sidecar contract.
    pub fn validate(&self) -> Result<(), HandlerManifestError> {
        if self.version != HANDLER_MANIFEST_VERSION {
            return Err(HandlerManifestError::UnsupportedVersion(
                self.version.clone(),
            ));
        }

        let mut handler_ids = HashSet::with_capacity(self.handlers.len());
        let mut tool_names = HashSet::new();
        for descriptor in &self.handlers {
            validate_descriptor(descriptor)?;
            if !handler_ids.insert(descriptor.handler_id.as_str()) {
                return Err(HandlerManifestError::DuplicateHandlerId(
                    descriptor.handler_id.clone(),
                ));
            }
            if descriptor.kind == HandlerKind::Tool && !tool_names.insert(descriptor.name.as_str())
            {
                return Err(HandlerManifestError::DuplicateToolName(
                    descriptor.name.clone(),
                ));
            }
        }
        Ok(())
    }
}

fn validate_descriptor(descriptor: &HandlerDescriptor) -> Result<(), HandlerManifestError> {
    for (field, value) in [
        ("handler_id", descriptor.handler_id.as_str()),
        ("module", descriptor.module.as_str()),
        ("qualname", descriptor.qualname.as_str()),
        ("name", descriptor.name.as_str()),
        (
            "source.artifact_path",
            descriptor.source.artifact_path.as_str(),
        ),
    ] {
        if value.trim().is_empty() {
            return Err(HandlerManifestError::EmptyField(field));
        }
    }
    if descriptor.source.content.is_empty() {
        return Err(HandlerManifestError::EmptyField("source.content"));
    }
    if !is_handler_id(&descriptor.handler_id) {
        return Err(HandlerManifestError::InvalidHandlerId(
            descriptor.handler_id.clone(),
        ));
    }
    if !is_artifact_relative_path(&descriptor.source.artifact_path) {
        return Err(HandlerManifestError::InvalidArtifactPath(
            descriptor.handler_id.clone(),
            descriptor.source.artifact_path.clone(),
        ));
    }
    for (field, value) in [
        ("module", descriptor.module.as_str()),
        ("qualname", descriptor.qualname.as_str()),
        ("name", descriptor.name.as_str()),
    ] {
        if !is_identifier(value) {
            return Err(HandlerManifestError::InvalidIdentifier(
                descriptor.handler_id.clone(),
                field,
                value.to_string(),
            ));
        }
    }

    // A descriptor is exactly one of the two closed kinds. Each kind requires
    // its own fields and forbids the other kind's: a tool carrying lifecycle
    // fields, or a hook carrying an argument schema or authority flags, is a
    // shape no consumer may dispatch on.
    match descriptor.kind {
        HandlerKind::Tool => {
            if !descriptor.schema.as_ref().is_some_and(Value::is_object) {
                return Err(HandlerManifestError::InvalidToolSchema(
                    descriptor.name.clone(),
                ));
            }
            for (field, present) in [
                ("event", descriptor.event.is_some()),
                ("match", descriptor.r#match.is_some()),
                ("mode", descriptor.mode.is_some()),
            ] {
                if present {
                    return Err(HandlerManifestError::CrossKindField(
                        descriptor.handler_id.clone(),
                        field,
                        "hook",
                    ));
                }
            }
        }
        HandlerKind::Hook => {
            if descriptor.event.as_deref().is_none_or(str::is_empty)
                || descriptor.r#match.as_deref().is_none_or(str::is_empty)
                || descriptor.mode.as_deref().is_none_or(str::is_empty)
            {
                return Err(HandlerManifestError::IncompleteHook(
                    descriptor.handler_id.clone(),
                ));
            }
            let mode = descriptor.mode.as_deref().unwrap_or_default();
            if !HANDLER_MANIFEST_HOOK_MODES.contains(&mode) {
                return Err(HandlerManifestError::InvalidHookMode(
                    descriptor.handler_id.clone(),
                    mode.to_string(),
                ));
            }
            for (field, present) in [
                ("schema", descriptor.schema.is_some()),
                ("read_only", descriptor.read_only.is_some()),
                ("requires_approval", descriptor.requires_approval.is_some()),
            ] {
                if present {
                    return Err(HandlerManifestError::CrossKindField(
                        descriptor.handler_id.clone(),
                        field,
                        "tool",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn is_handler_id(value: &str) -> bool {
    let Some(hex) = value.strip_prefix(HANDLER_MANIFEST_HANDLER_ID_PREFIX) else {
        return false;
    };
    hex.len() == HANDLER_MANIFEST_HANDLER_ID_HEX_LENGTH
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// The published contract identifier grammar: a leading ASCII alphanumeric
/// followed by up to 255 further alphanumerics or `. _ : / @ -`.
fn is_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && value.chars().count() <= HANDLER_MANIFEST_IDENTIFIER_MAX_LENGTH
        && characters
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '@' | '-'))
}

fn is_artifact_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path.components().all(|component| {
            !matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(kind: HandlerKind, name: &str) -> HandlerDescriptor {
        HandlerDescriptor {
            kind,
            language: HandlerLanguage::TypeScript,
            handler_id: format!(
                "{HANDLER_MANIFEST_HANDLER_ID_PREFIX}{}",
                "a".repeat(HANDLER_MANIFEST_HANDLER_ID_HEX_LENGTH)
            ),
            module: "example.handlers".to_string(),
            qualname: name.to_string(),
            name: name.to_string(),
            source: HandlerSource {
                artifact_path: format!("handlers/{name}.mjs"),
                content: "export function handler() {}\n".to_string(),
            },
            description: Some("example".to_string()),
            schema: (kind == HandlerKind::Tool).then(|| Value::Object(serde_json::Map::new())),
            read_only: (kind == HandlerKind::Tool).then_some(true),
            requires_approval: (kind == HandlerKind::Tool).then_some(false),
            event: (kind == HandlerKind::Hook).then(|| "pre_turn".to_string()),
            r#match: (kind == HandlerKind::Hook).then(|| "*".to_string()),
            mode: (kind == HandlerKind::Hook).then(|| "observe".to_string()),
        }
    }

    #[test]
    fn accepts_versioned_tool_and_hook_manifest() {
        let mut hook = descriptor(HandlerKind::Hook, "hook");
        hook.handler_id = format!(
            "{HANDLER_MANIFEST_HANDLER_ID_PREFIX}{}",
            "b".repeat(HANDLER_MANIFEST_HANDLER_ID_HEX_LENGTH)
        );
        HandlerManifest::new(vec![descriptor(HandlerKind::Tool, "echo"), hook])
            .validate()
            .expect("manifest is valid");
    }

    #[test]
    fn rejects_host_paths_and_duplicate_tool_names() {
        let mut first = descriptor(HandlerKind::Tool, "echo");
        first.source.artifact_path = "/tmp/handler.py".to_string();
        assert!(matches!(
            HandlerManifest::new(vec![first]).validate(),
            Err(HandlerManifestError::InvalidArtifactPath(_, _))
        ));

        let mut second = descriptor(HandlerKind::Tool, "echo");
        second.handler_id = format!(
            "{HANDLER_MANIFEST_HANDLER_ID_PREFIX}{}",
            "b".repeat(HANDLER_MANIFEST_HANDLER_ID_HEX_LENGTH)
        );
        assert!(matches!(
            HandlerManifest::new(vec![descriptor(HandlerKind::Tool, "echo"), second]).validate(),
            Err(HandlerManifestError::DuplicateToolName(_))
        ));
    }
}
