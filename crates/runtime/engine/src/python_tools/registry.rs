//! Python tool registry — resolves capability names to handler IDs.
//!
//! Built from the `tools.json` sidecar embedded in a compiled `.apxmobj` artifact.

use super::constants::CAPABILITY_NAME;
use apxm_core::error::RuntimeError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A single tool descriptor from the `tools.json` manifest.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolDescriptor {
    /// Unique handler identifier (`sha256:<hash>`).
    pub handler_id: String,
    /// Python module path (e.g. `myapp.tools`).
    pub module: String,
    /// Qualified name within the module (e.g. `add`).
    pub qualname: String,
    /// Human-friendly tool name (used as capability name).
    pub name: String,
    /// Human-friendly tool description.
    #[serde(default)]
    pub description: String,
    /// JSON Schema for the tool's parameters.
    pub schema: serde_json::Value,
    /// Source file for tools defined in executable scripts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
}

/// Registry of Python-backed tools, keyed by capability name.
pub struct PythonToolRegistry {
    /// name -> descriptor
    tools: HashMap<String, ToolDescriptor>,
}

impl PythonToolRegistry {
    /// Build a registry from a list of tool descriptors.
    pub fn from_descriptors(descriptors: Vec<ToolDescriptor>) -> Self {
        let tools = descriptors
            .into_iter()
            .map(|d| (d.name.clone(), d))
            .collect();
        Self { tools }
    }

    /// Load from a `tools.json` file path.
    pub fn from_file(path: &Path) -> Result<Self, RuntimeError> {
        let content = std::fs::read_to_string(path).map_err(|e| RuntimeError::Capability {
            capability: CAPABILITY_NAME.into(),
            message: format!("Failed to read tools.json at {}: {}", path.display(), e),
        })?;
        Self::from_json(&content)
    }

    /// Parse from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, RuntimeError> {
        let descriptors: Vec<ToolDescriptor> =
            serde_json::from_str(json).map_err(|e| RuntimeError::Capability {
                capability: CAPABILITY_NAME.into(),
                message: format!("Failed to parse tools.json: {}", e),
            })?;
        Ok(Self::from_descriptors(descriptors))
    }

    /// Look up a tool by capability name, returning its handler_id.
    pub fn resolve(&self, capability_name: &str) -> Option<&ToolDescriptor> {
        self.tools.get(capability_name)
    }

    /// Iterate over registered tool descriptors.
    pub fn descriptors(&self) -> impl Iterator<Item = &ToolDescriptor> {
        self.tools.values()
    }

    /// Check if a capability name refers to a Python tool.
    pub fn contains(&self, capability_name: &str) -> bool {
        self.tools.contains_key(capability_name)
    }

    /// All registered tool names.
    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Produce the manifest JSON array that gets passed to the worker on spawn.
    pub fn manifest_json(&self) -> Result<String, RuntimeError> {
        let descriptors: Vec<&ToolDescriptor> = self.tools.values().collect();
        serde_json::to_string(&descriptors).map_err(|e| RuntimeError::Serialization(e.to_string()))
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
