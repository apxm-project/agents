//! Capability registry configuration for the runtime.

use crate::{config::ApXmConfig, error::DriverError};
use apxm_core::types::AISOperationType;
use apxm_runtime::CapabilitySystem;

pub fn configure_capability_registry(
    capability_system: std::sync::Arc<CapabilitySystem>,
    config: &ApXmConfig,
) -> Result<(), DriverError> {
    let tools_config = config.tools_config();
    apxm_runtime::capability::builtins::register_standard_tools(&capability_system, &tools_config)
        .map_err(DriverError::Runtime)?;

    register_agent_management_tools(&capability_system);

    if let Err(e) = register_user_tools(&capability_system) {
        tracing::warn!("Failed to load user tools from ~/.apxm/tools.json: {}", e);
    }

    // NOTE: ACP agents are invoked via SPAWN_AGENT + COMMUNICATE.
    // The ACP tool entry is intentionally not registered.

    Ok(())
}

/// Register the durable agent-management tools (`schedule`, `manage_task`).
///
/// These are always-on builtins (BUILTINS allowlist parity). The driver has no
/// in-process schedule firer — that runs inside `apxm-server` — so in a
/// driver-only context schedules are persisted but fire only while a server is
/// running against the same state home. `manage_task` requires an AAM handle on
/// the capability system; without one it is skipped.
fn register_agent_management_tools(capability_system: &CapabilitySystem) {
    use apxm_runtime::capability::builtins::{
        ManageTaskCapability, ScheduleCapability, ToolsStore,
    };
    use std::sync::Arc;

    let store_path =
        apxm_core::env::state_home().join(apxm_core::constants::agent_tools::STORE_FILENAME);
    let store = match ToolsStore::open(&store_path) {
        Ok(store) => store,
        Err(error) => {
            tracing::warn!(
                "failed to open agent tools store; schedule/manage_task disabled: {error}"
            );
            return;
        }
    };

    match capability_system.aam() {
        Some(aam) => {
            if let Err(e) = capability_system.register(Arc::new(ManageTaskCapability::new(
                aam.clone(),
                store.clone(),
            ))) {
                tracing::warn!("failed to register manage_task capability: {e}");
            }
        }
        None => tracing::warn!("capability system has no AAM; manage_task not registered"),
    }

    let arm = Arc::new(tokio::sync::Notify::new());
    if let Err(e) = capability_system.register(Arc::new(ScheduleCapability::new(store, arm))) {
        tracing::warn!("failed to register schedule capability: {e}");
    }
}

fn register_user_tools(capability_system: &CapabilitySystem) -> Result<(), DriverError> {
    let home = dirs::home_dir()
        .ok_or_else(|| DriverError::Driver("Could not determine home directory".to_string()))?;
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
        let cap = UserToolCapability {
            metadata,
            command: tool.command,
            args: tool.args,
            timeout_ms: tool.timeout_ms,
        };
        capability_system
            .register(std::sync::Arc::new(cap))
            .map_err(DriverError::Runtime)?;
    }

    Ok(())
}

fn default_timeout_ms() -> u64 {
    30_000
}

#[derive(serde::Deserialize)]
struct UserToolEntry {
    name: String,
    description: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

struct UserToolCapability {
    metadata: apxm_runtime::capability::metadata::CapabilityMetadata,
    command: String,
    args: Vec<String>,
    timeout_ms: u64,
}

impl UserToolCapability {
    fn cap_err(&self, message: String) -> apxm_core::error::RuntimeError {
        apxm_core::error::RuntimeError::Capability {
            capability: self.metadata.name.clone(),
            message,
        }
    }
}

#[async_trait::async_trait]
impl apxm_runtime::capability::executor::CapabilityExecutor for UserToolCapability {
    async fn execute(
        &self,
        _args: std::collections::HashMap<String, apxm_core::types::values::Value>,
    ) -> apxm_runtime::capability::executor::CapabilityResult<apxm_core::types::values::Value> {
        // User tools are process-spawning capabilities: `to_exec_request`
        // returns an ExecRequest, so `CapabilitySystem` always routes them
        // through the sandbox registry and never calls this method. Refuse
        // direct execution so a stray caller can never spawn one unsandboxed.
        Err(self.cap_err(
            "user tools must be invoked through the sandbox (to_exec_request path), \
             not executed directly"
                .to_string(),
        ))
    }

    fn metadata(&self) -> &apxm_runtime::capability::metadata::CapabilityMetadata {
        &self.metadata
    }

    fn to_exec_request(
        &self,
        args: &std::collections::HashMap<String, apxm_core::types::values::Value>,
    ) -> Option<apxm_runtime::sandbox::ExecRequest> {
        let json_input = serde_json::to_string(&args).ok()?;

        Some(apxm_runtime::sandbox::ExecRequest {
            min_isolation: apxm_runtime::sandbox::IsolationLevel::OsLevel,
            program: self.command.clone(),
            args: self.args.clone(),
            stdin_data: Some(json_input),
            timeout: std::time::Duration::from_millis(self.timeout_ms),
            needs_network: true,
            origin_op: Some(AISOperationType::InvTool.to_string()),
            ..apxm_runtime::sandbox::ExecRequest::default()
        })
    }
}

