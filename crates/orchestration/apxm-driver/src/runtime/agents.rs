//! Agent registry configuration for the runtime.
//!
//! Follows the same pattern as `configure_llm_registry` and
//! `configure_capability_registry`: called during `RuntimeExecutor::new()`
//! to set up the agent subsystem from configuration.

use std::sync::Arc;

use apxm_acp::registry::AgentRegistry;
use apxm_acp::reverse::CapabilityReverseHandler;
use apxm_acp::session::AcpSession;
use apxm_core::apxm_acp;
use apxm_core::error::RuntimeError;
use apxm_core::types::aam::AamContext;
use apxm_runtime::process::AgentProcess;
use apxm_runtime::process_table::{AgentPromptResponse, AgentPrompter, AgentSpawner};
use apxm_runtime::sandbox::{IsolationLevel, SandboxBackend, SandboxRegistry};
use apxm_runtime::{CapabilitySystem, ProcessTable};

use apxm_acp::registry::AcpAgentProfile;

use crate::error::DriverError;

/// Configure the agent process table with ACP spawner and prompter.
///
/// This wires the `AgentSpawner` and `AgentPrompter` trait implementations
/// into the `ProcessTable`, enabling `SPAWN_AGENT` to create real ACP
/// subprocesses and `COMMUNICATE(protocol="acp")` to send prompts to them.
pub async fn configure_agent_registry(
    process_table: &ProcessTable,
    capability_system: Arc<CapabilitySystem>,
    sandbox_registry: Arc<SandboxRegistry>,
) -> Result<(), DriverError> {
    let spawner = Arc::new(AcpAgentSpawner::new(Arc::clone(&sandbox_registry)));
    let prompter = Arc::new(AcpAgentPrompter::new(capability_system, sandbox_registry));

    process_table.set_agent_spawner(spawner).await;
    process_table.set_agent_prompter(prompter).await;

    Ok(())
}

/// Resolve the sandbox backend to confine an agent under, honoring its profile.
///
/// Returns `None` when the profile does not opt into sandboxing. When it does
/// but no backend can confine a long-running child, this fails closed rather
/// than spawning the agent unconfined.
///
/// Selection is by isolation level + availability (`registry.select`), not by a
/// per-request `validate()` — the long-running spawn has no one-shot
/// `ExecRequest`. A backend whose confinement is request-shape dependent would
/// need its own check here; today's bubblewrap backend confines uniformly.
fn select_agent_sandbox(
    sandbox_registry: &SandboxRegistry,
    profile: &AcpAgentProfile,
    profile_name: &str,
) -> Result<Option<Arc<dyn SandboxBackend>>, RuntimeError> {
    if !profile.sandbox {
        return Ok(None);
    }
    sandbox_registry
        .select(IsolationLevel::OsLevel)
        .map(Some)
        .map_err(|e| RuntimeError::Operation {
            op_type: apxm_core::types::operations::AISOperationType::SpawnAgent,
            message: format!(
                "agent profile '{profile_name}' requests sandbox isolation but no capable \
                 backend is available: {e}"
            ),
        })
}

fn is_unsupported_session_control(error: &apxm_acp::AcpError) -> bool {
    matches!(
        error,
        apxm_acp::AcpError::AgentError { code, .. }
            if *code == apxm_acp::constants::json_rpc_errors::METHOD_NOT_FOUND
    )
}

// ─── AgentSpawner implementation ─────────────────────────────────────────────

/// Spawns external ACP agent subprocesses.
struct AcpAgentSpawner {
    registry: AgentRegistry,
    sandbox_registry: Arc<SandboxRegistry>,
}

impl AcpAgentSpawner {
    fn new(sandbox_registry: Arc<SandboxRegistry>) -> Self {
        Self {
            registry: AgentRegistry::load(),
            sandbox_registry,
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
        extra_env: &std::collections::HashMap<String, String>,
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

        // Inject extra_env (e.g. APXM_NODE_WORKSPACE) into the profile env first
        // so subsequent borrows use the final profile state.
        let mut profile = profile;
        for (k, v) in extra_env {
            profile.env.insert(k.clone(), v.clone());
        }

        // Profile defaults fill mode/model when the graph does not specify them.
        let effective_mode = mode.or(profile.default_mode.as_deref());
        let effective_model = model.or(profile.default_model.as_deref());

        apxm_acp!(info,
            agent_name = agent_name,
            profile = profile_name,
            cwd = %cwd.display(),
            "Spawning ACP agent subprocess"
        );

        let sandbox = select_agent_sandbox(&self.sandbox_registry, &profile, profile_name)?;

        let mut session = AcpSession::spawn(agent_name, &profile, cwd, aam_context, sandbox)
            .await
            .map_err(|e| RuntimeError::Operation {
                op_type: apxm_core::types::operations::AISOperationType::SpawnAgent,
                message: format!("ACP spawn failed for '{}': {}", agent_name, e),
            })?;

        // Apply session controls if specified (or from profile defaults)
        if let Some(mode_id) = effective_mode {
            if let Err(e) =
                apxm_acp::controls::SessionControls::set_mode(&mut session, mode_id).await
            {
                if is_unsupported_session_control(&e) {
                    apxm_acp!(
                        warn,
                        agent_name = agent_name,
                        profile = profile_name,
                        mode = mode_id,
                        "ACP agent does not support session mode control"
                    );
                } else {
                    return Err(RuntimeError::Operation {
                        op_type: apxm_core::types::operations::AISOperationType::SpawnAgent,
                        message: format!("set_mode('{}') failed: {}", mode_id, e),
                    });
                }
            }
        }
        if let Some(model_id) = effective_model {
            if let Err(e) =
                apxm_acp::controls::SessionControls::set_model(&mut session, model_id).await
            {
                if is_unsupported_session_control(&e) {
                    apxm_acp!(
                        warn,
                        agent_name = agent_name,
                        profile = profile_name,
                        model = model_id,
                        "ACP agent does not support session model control"
                    );
                } else {
                    return Err(RuntimeError::Operation {
                        op_type: apxm_core::types::operations::AISOperationType::SpawnAgent,
                        message: format!("set_model('{}') failed: {}", model_id, e),
                    });
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_session_control_is_json_rpc_method_not_found() {
        let unsupported = apxm_acp::AcpError::AgentError {
            code: apxm_acp::constants::json_rpc_errors::METHOD_NOT_FOUND,
            message: "Method not found".to_string(),
        };
        let invalid_request = apxm_acp::AcpError::AgentError {
            code: -32600,
            message: "Invalid request".to_string(),
        };

        assert!(is_unsupported_session_control(&unsupported));
        assert!(!is_unsupported_session_control(&invalid_request));
    }
}

// ─── AgentPrompter implementation ────────────────────────────────────────────

/// Sends prompts to live ACP agent sessions via the ProcessTable.
struct AcpAgentPrompter {
    capability_system: Arc<CapabilitySystem>,
    registry: AgentRegistry,
    sandbox_registry: Arc<SandboxRegistry>,
}

impl AcpAgentPrompter {
    fn new(
        capability_system: Arc<CapabilitySystem>,
        sandbox_registry: Arc<SandboxRegistry>,
    ) -> Self {
        Self {
            capability_system,
            registry: AgentRegistry::load(),
            sandbox_registry,
        }
    }
}

#[async_trait::async_trait]
impl AgentPrompter for AcpAgentPrompter {
    async fn prompt(
        &self,
        process: &AgentProcess,
        message: &str,
    ) -> Result<AgentPromptResponse, RuntimeError> {
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

        // Resolve permission mode and sandbox opt-in from the cached registry.
        let profile = self.registry.get(&profile_name);
        let permission_mode = profile
            .as_ref()
            .map(|p| p.permission_mode.clone())
            .unwrap_or_default();
        let sandbox = match &profile {
            Some(p) => select_agent_sandbox(&self.sandbox_registry, p, &profile_name)?,
            None => None,
        };

        let handler = CapabilityReverseHandler::with_sandbox(
            Arc::clone(&self.capability_system),
            permission_mode,
            sandbox,
        );

        // Lock the session and send the prompt
        let mut guard = session_arc.lock().await;
        let session =
            guard
                .downcast_mut::<AcpSession>()
                .ok_or_else(|| RuntimeError::Operation {
                    op_type: apxm_core::types::operations::AISOperationType::Communicate,
                    message: format!(
                        "Agent '{}' session has unexpected type (expected AcpSession)",
                        process.name
                    ),
                })?;

        let result =
            session
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

        let token_usage = result.token_usage.as_ref();
        let input_tokens = token_usage
            .and_then(|usage| usage.input_tokens)
            .and_then(|tokens| usize::try_from(tokens).ok());
        let output_tokens = token_usage
            .and_then(|usage| usage.output_tokens)
            .and_then(|tokens| usize::try_from(tokens).ok());

        let mut response = AgentPromptResponse::text(result.text)
            .with_session_id(session.session_id().to_string())
            .with_turn(u64::from(session.turn_count()))
            .with_stop_reason(result.stop_reason)
            .with_token_usage(input_tokens, output_tokens);
        if let Some(agent_session_id) = session.agent_session_id() {
            response = response.with_agent_session_id(agent_session_id.to_string());
        }
        if let Some(model) = result.model {
            response = response.with_model(model);
        }

        Ok(response)
    }
}
