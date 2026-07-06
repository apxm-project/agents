//! TypeScript tool registry — resolves capability names to handler IDs.
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
    pub handler_id: String,
    pub module: String,
    pub qualname: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
}

/// Registry of TypeScript-backed tools, keyed by capability name.
pub struct TypeScriptHandlerRegistry {
    tools: HashMap<String, ToolDescriptor>,
}

impl TypeScriptHandlerRegistry {
    pub fn from_descriptors(descriptors: Vec<ToolDescriptor>) -> Self {
        let tools = descriptors
            .into_iter()
            .map(|d| (d.name.clone(), d))
            .collect();
        Self { tools }
    }

    pub fn from_file(path: &Path) -> Result<Self, RuntimeError> {
        let content = std::fs::read_to_string(path).map_err(|e| RuntimeError::Capability {
            capability: CAPABILITY_NAME.into(),
            message: format!("Failed to read tools.json at {}: {}", path.display(), e),
        })?;
        let mut registry = Self::from_json(&content)?;
        if let Some(base_dir) = path.parent() {
            registry.resolve_relative_sources(base_dir);
        }
        Ok(registry)
    }

    pub fn from_json(json: &str) -> Result<Self, RuntimeError> {
        let descriptors: Vec<ToolDescriptor> =
            serde_json::from_str(json).map_err(|e| RuntimeError::Capability {
                capability: CAPABILITY_NAME.into(),
                message: format!("Failed to parse tools.json: {}", e),
            })?;
        Ok(Self::from_descriptors(descriptors))
    }

    pub fn resolve(&self, capability_name: &str) -> Option<&ToolDescriptor> {
        self.tools.get(capability_name)
    }

    pub fn resolve_handler_id(&self, handler_id: &str) -> Option<&ToolDescriptor> {
        self.tools.values().find(|d| d.handler_id == handler_id)
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &ToolDescriptor> {
        self.tools.values()
    }

    pub fn contains(&self, capability_name: &str) -> bool {
        self.tools.contains_key(capability_name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    pub fn manifest_json(&self) -> Result<String, RuntimeError> {
        let descriptors: Vec<&ToolDescriptor> = self.tools.values().collect();
        serde_json::to_string(&descriptors).map_err(|e| RuntimeError::Serialization(e.to_string()))
    }

    fn resolve_relative_sources(&mut self, base_dir: &Path) {
        for descriptor in self.tools.values_mut() {
            let Some(source_file) = descriptor.source_file.as_ref() else {
                continue;
            };
            let source_path = Path::new(source_file);
            if source_path.is_absolute() {
                continue;
            }
            descriptor.source_file =
                Some(base_dir.join(source_path).to_string_lossy().into_owned());
        }
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_resolves_by_capability_name() {
        let registry = TypeScriptHandlerRegistry::from_json(
            r#"[{"handler_id":"sha256:abc","module":"mod","qualname":"fn","name":"echo","schema":{}}]"#,
        )
        .unwrap();
        assert!(registry.contains("echo"));
        assert_eq!(registry.resolve("echo").unwrap().handler_id, "sha256:abc");
    }

    #[test]
    fn registry_resolves_by_handler_id() {
        let registry = TypeScriptHandlerRegistry::from_json(
            r#"[{"handler_id":"sha256:abc","module":"mod","qualname":"fn","name":"echo","schema":{}}]"#,
        )
        .unwrap();
        assert!(registry.resolve_handler_id("sha256:abc").is_some());
    }

    #[test]
    fn from_file_resolves_relative_source_files_from_manifest_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest_dir = tmp.path().join("capabilities/handlers");
        std::fs::create_dir_all(&manifest_dir).unwrap();
        let manifest = manifest_dir.join("tools.json");
        std::fs::write(
            &manifest,
            r#"[{"handler_id":"sha256:abc","module":"capabilities/example/handler","qualname":"fn","name":"echo","schema":{},"source_file":"../example/handler.ts"}]"#,
        )
        .unwrap();

        let registry = TypeScriptHandlerRegistry::from_file(&manifest).unwrap();
        let source_file = registry
            .resolve("echo")
            .and_then(|descriptor| descriptor.source_file.as_deref())
            .unwrap();

        assert_eq!(
            source_file,
            manifest_dir.join("../example/handler.ts").to_string_lossy()
        );
    }
}
