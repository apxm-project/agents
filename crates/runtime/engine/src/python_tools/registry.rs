//! Python handler registry backed by the portable artifact sidecar.

use super::constants::CAPABILITY_NAME;
use apxm_core::error::RuntimeError;
use apxm_core::types::{HandlerDescriptor, HandlerKind, HandlerLanguage, HandlerManifest};
use std::collections::HashMap;
use std::path::Path;

/// One portable handler descriptor.
pub type ToolDescriptor = HandlerDescriptor;

/// Registry of Python handlers, keyed by capability name and handler ID.
pub struct PythonHandlerRegistry {
    tools: HashMap<String, ToolDescriptor>,
    handlers: HashMap<String, ToolDescriptor>,
}

impl PythonHandlerRegistry {
    /// Build a Python registry from the shared cross-language manifest.
    pub fn from_manifest(manifest: HandlerManifest) -> Result<Self, RuntimeError> {
        manifest
            .validate()
            .map_err(|error| RuntimeError::Capability {
                capability: CAPABILITY_NAME.into(),
                message: format!("Invalid handler manifest: {error}"),
            })?;

        let mut tools = HashMap::new();
        let mut handlers = HashMap::new();
        for descriptor in manifest
            .handlers
            .into_iter()
            .filter(|descriptor| descriptor.language == HandlerLanguage::Python)
        {
            if descriptor.kind == HandlerKind::Tool {
                tools.insert(descriptor.name.clone(), descriptor.clone());
            }
            handlers.insert(descriptor.handler_id.clone(), descriptor);
        }
        Ok(Self { tools, handlers })
    }

    /// Load a shared manifest from disk.
    pub fn from_file(path: &Path) -> Result<Self, RuntimeError> {
        let content = std::fs::read(path).map_err(|error| RuntimeError::Capability {
            capability: CAPABILITY_NAME.into(),
            message: format!(
                "Failed to read handler manifest at {}: {error}",
                path.display()
            ),
        })?;
        Self::from_json_bytes(&content)
    }

    /// Parse a shared manifest from JSON.
    pub fn from_json(json: &str) -> Result<Self, RuntimeError> {
        Self::from_json_bytes(json.as_bytes())
    }

    /// Parse a shared manifest from UTF-8 JSON bytes.
    pub fn from_json_bytes(json: &[u8]) -> Result<Self, RuntimeError> {
        let manifest =
            HandlerManifest::from_json_slice(json).map_err(|error| RuntimeError::Capability {
                capability: CAPABILITY_NAME.into(),
                message: format!("Failed to parse handler manifest: {error}"),
            })?;
        Self::from_manifest(manifest)
    }

    /// Look up a Python tool by capability name.
    pub fn resolve(&self, capability_name: &str) -> Option<&ToolDescriptor> {
        self.tools.get(capability_name)
    }

    /// Look up a Python tool or hook by stable handler ID.
    pub fn resolve_handler_id(&self, handler_id: &str) -> Option<&ToolDescriptor> {
        self.handlers.get(handler_id)
    }

    /// Iterate over Python-backed tools.
    pub fn descriptors(&self) -> impl Iterator<Item = &ToolDescriptor> {
        self.tools.values()
    }

    /// Check whether a capability name refers to a Python tool.
    pub fn contains(&self, capability_name: &str) -> bool {
        self.tools.contains_key(capability_name)
    }

    /// All registered Python tool names.
    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(String::as_str).collect()
    }

    /// Serialize the Python subset for the worker subprocess.
    pub fn manifest_json(&self) -> Result<String, RuntimeError> {
        serde_json::to_string(&HandlerManifest::new(
            self.handlers.values().cloned().collect(),
        ))
        .map_err(|error| RuntimeError::Serialization(error.to_string()))
    }

    /// Number of Python-backed tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether no Python-backed tools are registered.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
