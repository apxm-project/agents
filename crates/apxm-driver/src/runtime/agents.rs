//! Agent registry configuration for the runtime.
//!
//! Follows the same pattern as `configure_llm_registry` and
//! `configure_capability_registry`: called during `RuntimeExecutor::new()`
//! to set up the agent subsystem from configuration.

use std::sync::Arc;

use apxm_acp::constants::result_keys;
use apxm_acp::registry::AgentRegistry;
use apxm_acp::reverse::CapabilityReverseHandler;
use apxm_acp::session::AcpSession;
use apxm_core::apxm_acp;
use apxm_core::error::RuntimeError;
use apxm_core::types::aam::AamContext;
use apxm_core::types::values::Value;
use apxm_runtime::process::AgentProcess;
use apxm_runtime::process_table::{AgentPrompter, AgentSpawner};
use apxm_runtime::{CapabilitySystem, ProcessTable};

use crate::error::DriverError;

/// Configure the agent process table with ACP spawner and prompter.
///
/// This wires the `AgentSpawner` and `AgentPrompter` trait implementations
/// into the `ProcessTable`, enabling `SPAWN_AGENT` to create real ACP
/// subprocesses and `COMMUNICATE(protocol="acp")` to send prompts to them.
pub async fn configure_agent_registry(
    process_table: &ProcessTable,
    capability_system: Arc<CapabilitySystem>,
) -> Result<(), DriverError> {
    let spawner = Arc::new(AcpAgentSpawner::new());
    let prompter = Arc::new(AcpAgentPrompter::new(capability_system));

    process_table.set_agent_spawner(spawner).await;
    process_table.set_agent_prompter(prompter).await;

    Ok(())
}

// ─── AgentSpawner implementation ─────────────────────────────────────────────

/// Spawns external ACP agent subprocesses.
struct AcpAgentSpawner {
    registry: AgentRegistry,
}

impl AcpAgentSpawner {
    fn new() -> Self {
        Self {
            registry: AgentRegistry::load(),
        }
    }
}

#[async_trait::async_trait]
impl AgentSpawner for AcpAgentSpawner {
    async fn spawn_external(
        &self,
        agent_name: &str,
        profile_name: &str,
        cwd: &std::path::Path,
        mode: Option<&str>,
        model: Option<&str>,
        aam_context: &AamContext,
    ) -> Result<Arc<tokio::sync::Mutex<dyn std::any::Any + Send + Sync>>, RuntimeError> {
        let profile = self
            .registry
            .get(profile_name)
            .ok_or_else(|| {
                let available: Vec<String> = self
                    .registry
                    .list()
                    .into_iter()
                    .map(|(name, _, _)| name)
                    .collect();
                RuntimeError::Operation {
                    op_type: apxm_core::types::operations::AISOperationType::SpawnAgent,
                    message: format!(
                        "Unknown agent profile '{}'. Register with: apxm agent add {}. Registered: [{}]",
                        profile_name,
                        profile_name,
                        available.join(", ")
                    ),
                }
            })?
            .clone();

        // Apply profile defaults as fallbacks for mode/model
        let effective_mode = mode.or(profile.default_mode.as_deref());
        let effective_model = model.or(profile.default_model.as_deref());

        apxm_acp!(info,
            agent_name = agent_name,
            profile = profile_name,
            cwd = %cwd.display(),
            "Spawning ACP agent subprocess"
        );

        let mut session =
            AcpSession::spawn(agent_name, &profile, cwd, aam_context)
                .await
                .map_err(|e| RuntimeError::Operation {
                    op_type: apxm_core::types::operations::AISOperationType::SpawnAgent,
                    message: format!("ACP spawn failed for '{}': {}", agent_name, e),
                })?;

        // Apply session controls if specified (or from profile defaults)
        if let Some(mode_id) = effective_mode {
            apxm_acp::controls::SessionControls::set_mode(&mut session, mode_id)
                .await
                .map_err(|e| RuntimeError::Operation {
                    op_type: apxm_core::types::operations::AISOperationType::SpawnAgent,
                    message: format!("set_mode('{}') failed: {}", mode_id, e),
                })?;
        }
        if let Some(model_id) = effective_model {
            apxm_acp::controls::SessionControls::set_model(&mut session, model_id)
                .await
                .map_err(|e| RuntimeError::Operation {
                    op_type: apxm_core::types::operations::AISOperationType::SpawnAgent,
                    message: format!("set_model('{}') failed: {}", model_id, e),
                })?;
        }

        apxm_acp!(info,
            agent_name = agent_name,
            session_id = %session.session_id(),
            agent_session_id = ?session.agent_session_id(),
            "ACP agent process spawned successfully"
        );

        let session_arc: Arc<tokio::sync::Mutex<dyn std::any::Any + Send + Sync>> =
            Arc::new(tokio::sync::Mutex::new(session));
        Ok(session_arc)
    }
}

// ─── AgentPrompter implementation ────────────────────────────────────────────

/// Sends prompts to live ACP agent sessions via the ProcessTable.
struct AcpAgentPrompter {
    capability_system: Arc<CapabilitySystem>,
    registry: AgentRegistry,
}

impl AcpAgentPrompter {
    fn new(capability_system: Arc<CapabilitySystem>) -> Self {
        Self {
            capability_system,
            registry: AgentRegistry::load(),
        }
    }
}

#[async_trait::async_trait]
impl AgentPrompter for AcpAgentPrompter {
    async fn prompt(
        &self,
        process: &AgentProcess,
        message: &str,
    ) -> Result<Value, RuntimeError> {
        use apxm_runtime::process::ProcessKind;

        let (session_arc, profile_name) = match &process.kind {
            ProcessKind::External {
                session,
                profile_name,
            } => (Arc::clone(session), profile_name.clone()),
            ProcessKind::Local => {
                return Err(RuntimeError::Operation {
                    op_type: apxm_core::types::operations::AISOperationType::Communicate,
                    message: format!(
                        "Agent '{}' is a local process, not an ACP agent. \
                         Use protocol 'local' instead of 'acp'.",
                        process.name
                    ),
                });
            }
        };

        // Resolve permission mode from cached registry
        let permission_mode = self
            .registry
            .get(&profile_name)
            .map(|p| p.permission_mode.clone())
            .unwrap_or_default();

        let handler = CapabilityReverseHandler::new(
            Arc::clone(&self.capability_system),
            permission_mode,
        );

        // Lock the session and send the prompt
        let mut guard = session_arc.lock().await;
        let session = guard
            .downcast_mut::<AcpSession>()
            .ok_or_else(|| RuntimeError::Operation {
                op_type: apxm_core::types::operations::AISOperationType::Communicate,
                message: format!(
                    "Agent '{}' session has unexpected type (expected AcpSession)",
                    process.name
                ),
            })?;

        let result = session
            .prompt(message, &handler)
            .await
            .map_err(|e| RuntimeError::Operation {
                op_type: apxm_core::types::operations::AISOperationType::Communicate,
                message: format!("ACP prompt to '{}' failed: {}", process.name, e),
            })?;

        apxm_acp!(info,
            agent = %process.name,
            session_id = %session.session_id(),
            turn = session.turn_count(),
            stop_reason = %result.stop_reason,
            response_len = result.text.len(),
            "ACP COMMUNICATE prompt completed"
        );

        // Build structured response (same keys as AcpCapability::execute)
        let mut result_map = std::collections::HashMap::new();
        result_map.insert(result_keys::TEXT.to_string(), Value::String(result.text));
        result_map.insert(
            result_keys::AGENT.to_string(),
            Value::String(process.name.clone()),
        );
        result_map.insert(
            result_keys::STOP_REASON.to_string(),
            Value::String(result.stop_reason),
        );
        result_map.insert(
            result_keys::SESSION_ID.to_string(),
            Value::String(session.session_id().to_string()),
        );
        result_map.insert(
            result_keys::TURN.to_string(),
            Value::Number(apxm_core::types::values::Number::Integer(
                session.turn_count() as i64,
            )),
        );
        if let Some(model) = result.model {
            result_map.insert(result_keys::MODEL.to_string(), Value::String(model));
        }
        if let Some(usage) = &result.token_usage {
            if let Some(input) = usage.input_tokens {
                result_map.insert(
                    result_keys::INPUT_TOKENS.to_string(),
                    Value::Number(apxm_core::types::values::Number::Integer(input as i64)),
                );
            }
            if let Some(output) = usage.output_tokens {
                result_map.insert(
                    result_keys::OUTPUT_TOKENS.to_string(),
                    Value::Number(apxm_core::types::values::Number::Integer(output as i64)),
                );
            }
        }

        Ok(Value::Object(result_map))
    }
}
