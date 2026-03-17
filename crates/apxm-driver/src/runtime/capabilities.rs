//! Capability registry configuration for the runtime.

use crate::{config::ApXmConfig, error::DriverError};
use apxm_runtime::CapabilitySystem;

pub fn configure_capability_registry(
    capability_system: &CapabilitySystem,
    config: &ApXmConfig,
) -> Result<(), DriverError> {
    let tools_config = config.tools_config();
    apxm_tools::register_standard_tools(capability_system, &tools_config)
        .map_err(DriverError::Runtime)?;

    if let Err(e) = register_user_tools(capability_system) {
        tracing::warn!("Failed to load user tools from ~/.apxm/tools.json: {}", e);
    }
    Ok(())
}

fn register_user_tools(capability_system: &CapabilitySystem) -> Result<(), DriverError> {
    let home = dirs::home_dir().ok_or_else(|| {
        DriverError::Driver("Could not determine home directory".to_string())
    })?;
    let tools_path = home.join(".apxm").join("tools.json");
    if !tools_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&tools_path).map_err(|e| {
        DriverError::Driver(format!("Failed to read {}: {}", tools_path.display(), e))
    })?;

    let tools: Vec<UserToolEntry> = serde_json::from_str(&content).map_err(|e| {
        DriverError::Driver(format!("Failed to parse {}: {}", tools_path.display(), e))
    })?;

    for tool in tools {
        if capability_system.has_capability(&tool.name) {
            continue;
        }
        let metadata = apxm_runtime::capability::metadata::CapabilityMetadata::new(
            &tool.name,
            &tool.description,
            serde_json::json!({}),
        );
        let cap = UserToolCapability { metadata };
        capability_system
            .register(std::sync::Arc::new(cap))
            .map_err(DriverError::Runtime)?;
    }

    Ok(())
}

#[derive(serde::Deserialize)]
struct UserToolEntry {
    name: String,
    description: String,
}

struct UserToolCapability {
    metadata: apxm_runtime::capability::metadata::CapabilityMetadata,
}

#[async_trait::async_trait]
impl apxm_runtime::capability::executor::CapabilityExecutor for UserToolCapability {
    async fn execute(
        &self,
        _args: std::collections::HashMap<String, apxm_core::types::values::Value>,
    ) -> apxm_runtime::capability::executor::CapabilityResult<apxm_core::types::values::Value> {
        Err(apxm_core::error::RuntimeError::Capability {
            capability: self.metadata.name.clone(),
            message: "User tool is registered but has no built-in executor. Provide an implementation via a plugin.".to_string(),
        })
    }

    fn metadata(&self) -> &apxm_runtime::capability::metadata::CapabilityMetadata {
        &self.metadata
    }
}
