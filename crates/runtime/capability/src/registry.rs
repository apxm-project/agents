//! Capability registry for registration and lookup

use super::{executor::CapabilityExecutor, metadata::RuntimeCapability};
use apxm_backends::JsonSchema;
use apxm_core::error::RuntimeError;
use dashmap::DashMap;
use std::sync::Arc;

/// Result type for registry operations
type RegistryResult<T> = Result<T, RuntimeError>;

/// Concurrent capability registry
///
/// Thread-safe registry for storing and looking up capabilities.
/// Uses DashMap for lock-free concurrent access.
pub struct CapabilityRegistry {
    capabilities: Arc<DashMap<String, Arc<dyn CapabilityExecutor>>>,
    schemas: Arc<DashMap<String, Arc<JsonSchema>>>,
}

impl CapabilityRegistry {
    /// Create a new empty capability registry
    pub fn new() -> Self {
        Self {
            capabilities: Arc::new(DashMap::new()),
            schemas: Arc::new(DashMap::new()),
        }
    }

    /// Register a capability, compiling its JSON schema for validation.
    pub fn register(&self, capability: Arc<dyn CapabilityExecutor>) -> RegistryResult<()> {
        let metadata = capability.metadata();
        let name = metadata.name.clone();

        // Check for duplicate registration
        if self.capabilities.contains_key(&name) {
            return Err(RuntimeError::Capability {
                capability: name.clone(),
                message: format!("Capability '{}' is already registered", name),
            });
        }

        // Compile JSON schema for validation
        let schema = JsonSchema::from_value(metadata.parameters_schema.clone()).map_err(|e| {
            RuntimeError::Capability {
                capability: name.clone(),
                message: format!("Invalid parameter schema: {}", e),
            }
        })?;

        // Register capability and schema
        self.schemas.insert(name.clone(), Arc::new(schema));
        self.capabilities.insert(name.clone(), capability);

        tracing::info!("Registered capability: {}", name);
        Ok(())
    }

    /// Register a capability, replacing any existing one of the same name.
    ///
    /// Dynamic HTTP capabilities are re-registered on every Invocation by
    /// hosts whose tool surface is context-scoped (e.g. a host app
    /// re-registers the project's tools per Invocation). Re-registration must
    /// overwrite rather than fail so the latest schema/endpoint wins.
    pub fn register_or_replace(
        &self,
        capability: Arc<dyn CapabilityExecutor>,
    ) -> RegistryResult<()> {
        let metadata = capability.metadata();
        let name = metadata.name.clone();
        let schema = JsonSchema::from_value(metadata.parameters_schema.clone()).map_err(|e| {
            RuntimeError::Capability {
                capability: name.clone(),
                message: format!("Invalid parameter schema: {}", e),
            }
        })?;
        self.schemas.insert(name.clone(), Arc::new(schema));
        self.capabilities.insert(name.clone(), capability);
        tracing::info!("Registered capability (replace): {}", name);
        Ok(())
    }

    /// Get a capability by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn CapabilityExecutor>> {
        self.capabilities
            .get(name)
            .map(|entry| Arc::clone(entry.value()))
    }

    /// Get the compiled schema for a capability.
    pub fn get_schema(&self, name: &str) -> Option<Arc<JsonSchema>> {
        self.schemas
            .get(name)
            .map(|entry| Arc::clone(entry.value()))
    }

    /// Check if a capability is registered
    pub fn contains(&self, name: &str) -> bool {
        self.capabilities.contains_key(name)
    }

    /// List all registered capability names
    pub fn list_names(&self) -> Vec<String> {
        self.capabilities
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// List all capability metadata
    pub fn list_metadata(&self) -> Vec<RuntimeCapability> {
        self.capabilities
            .iter()
            .map(|entry| entry.value().metadata().clone())
            .collect()
    }

    /// Return registered executors for construction of an execution-local facade.
    pub fn list_capabilities(&self) -> Vec<Arc<dyn CapabilityExecutor>> {
        self.capabilities
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect()
    }

    /// Unregister a capability.
    pub fn unregister(&self, name: &str) -> bool {
        let cap_removed = self.capabilities.remove(name).is_some();
        let schema_removed = self.schemas.remove(name).is_some();
        cap_removed && schema_removed
    }

    /// Get the number of registered capabilities
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}
