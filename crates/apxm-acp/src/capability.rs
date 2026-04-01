use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use apxm_core::error::RuntimeError;
use apxm_core::types::values::Value;
use apxm_runtime::CapabilitySystem;
use apxm_runtime::capability::executor::{CapabilityExecutor, CapabilityResult};
use apxm_runtime::capability::metadata::CapabilityMetadata;

use crate::constants::{args as cap_args, capability as cap_consts, result_keys};
use crate::controls::SessionControls;
use crate::registry::AgentRegistry;
use crate::reverse::CapabilityReverseHandler;
use crate::session_pool::SessionPool;

/// ACP capability for INV(acp) nodes.
///
/// Spawns coding agents (Claude, Codex, Gemini, etc.) via the ACP protocol
/// and returns their responses as `Value::String`.
pub struct AcpCapability {
    metadata: CapabilityMetadata,
    registry: AgentRegistry,
    capability_system: Arc<CapabilitySystem>,
    session_pool: Arc<SessionPool>,
}

impl AcpCapability {
    pub fn new(capability_system: Arc<CapabilitySystem>, session_pool: Arc<SessionPool>) -> Self {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                (cap_args::AGENT): {
                    "type": "string",
                    "description": "Agent profile name (claude, codex, gemini, ...)"
                },
                (cap_args::PROMPT): {
                    "type": "string",
                    "description": "Prompt text to send to the agent"
                },
                (cap_args::CWD): {
                    "type": "string",
                    "description": "Working directory for the agent session"
                },
                (cap_args::SESSION_HANDLE): {
                    "type": "string",
                    "description": "Named session handle for multi-turn (default: 'main')"
                },
                (cap_args::MODE): {
                    "type": "string",
                    "description": "Agent mode to set before prompting (e.g. 'architect', 'code')"
                },
                (cap_args::MODEL): {
                    "type": "string",
                    "description": "Model to use (e.g. 'claude-sonnet-4')"
                }
            },
            "required": [cap_args::AGENT, cap_args::PROMPT]
        });

        Self {
            metadata: CapabilityMetadata::new(
                cap_consts::ACP_CAPABILITY_NAME,
                "Invoke an ACP-compatible coding agent (Claude, Codex, Gemini, etc.)",
                schema,
            )
            .with_returns("object")
            .with_latency(cap_consts::DEFAULT_LATENCY_MS)
            .with_tags(vec![
                "agent".to_string(),
                cap_consts::ACP_CAPABILITY_NAME.to_string(),
            ]),
            registry: AgentRegistry::load(),
            capability_system,
            session_pool,
        }
    }

    fn cap_err(&self, message: String) -> RuntimeError {
        RuntimeError::Capability {
            capability: cap_consts::ACP_CAPABILITY_NAME.to_string(),
            message,
        }
    }
}

#[async_trait]
impl CapabilityExecutor for AcpCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        // 1. Extract args
        let agent_name = args
            .get(cap_args::AGENT)
            .or_else(|| args.get(cap_args::ARG_AGENT))
            .or_else(|| args.get(cap_args::ARG0))
            .and_then(|v| v.as_string())
            .ok_or_else(|| self.cap_err("Missing required 'agent' argument".to_string()))?
            .to_string();

        let prompt_text = args
            .get(cap_args::PROMPT)
            .or_else(|| args.get(cap_args::ARG_PROMPT))
            .or_else(|| args.get(cap_args::ARG1))
            .and_then(|v| v.as_string())
            .ok_or_else(|| self.cap_err("Missing required 'prompt' argument".to_string()))?
            .to_string();

        let cwd = args
            .get(cap_args::CWD)
            .and_then(|v| v.as_string())
            .map(|s| PathBuf::from(s.as_str()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let session_handle = args
            .get(cap_args::SESSION_HANDLE)
            .and_then(|v| v.as_string())
            .map(|s| s.to_string())
            .unwrap_or_else(|| cap_consts::DEFAULT_SESSION_HANDLE.to_string());

        let mode = args
            .get(cap_args::MODE)
            .and_then(|v| v.as_string())
            .map(|s| s.to_string());

        let model = args
            .get(cap_args::MODEL)
            .and_then(|v| v.as_string())
            .map(|s| s.to_string());

        // 2. Look up profile
        let profile = self
            .registry
            .get(&agent_name)
            .ok_or_else(|| {
                let available: Vec<String> = self
                    .registry
                    .list()
                    .into_iter()
                    .map(|(name, _, _)| name)
                    .collect();
                self.cap_err(format!(
                    "Unknown agent: '{agent_name}'. Available: {}",
                    available.join(", ")
                ))
            })?
            .clone();

        // 3. Get or create session from pool (multi-turn support)
        let session_arc = self
            .session_pool
            .get_or_create(&agent_name, &cwd, &session_handle, &profile)
            .await
            .map_err(|e| self.cap_err(format!("Session spawn failed: {e}")))?;

        let mut session = session_arc.lock().await;

        // 4. Apply session controls (mode/model) if requested
        if let Some(ref mode_id) = mode {
            SessionControls::set_mode(&mut session, mode_id)
                .await
                .map_err(|e| self.cap_err(format!("set_mode failed: {e}")))?;
        }
        if let Some(ref model_id) = model {
            SessionControls::set_model(&mut session, model_id)
                .await
                .map_err(|e| self.cap_err(format!("set_model failed: {e}")))?;
        }

        // 5. Create reverse handler for this prompt
        let handler = CapabilityReverseHandler::new(
            Arc::clone(&self.capability_system),
            profile.permission_mode.clone(),
        );

        // 6. Send prompt, collect response
        let result = session
            .prompt(&prompt_text, &handler)
            .await
            .map_err(|e| self.cap_err(format!("Prompt failed: {e}")))?;

        tracing::info!(
            agent = %agent_name,
            session = %session.session_id(),
            turn = session.turn_count(),
            stop_reason = %result.stop_reason,
            response_len = result.text.len(),
            "ACP prompt completed"
        );

        // 7. Build structured result
        let mut result_map = HashMap::new();
        result_map.insert(
            result_keys::TEXT.to_string(),
            Value::String(result.text.clone()),
        );
        result_map.insert(result_keys::AGENT.to_string(), Value::String(agent_name));
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

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_agent_arg_returns_error() {
        let cap = AcpCapability::new(
            Arc::new(CapabilitySystem::new()),
            Arc::new(SessionPool::new()),
        );
        let args = HashMap::new();
        let result = cap.execute(args).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("agent"), "error should mention 'agent': {err}");
    }

    #[tokio::test]
    async fn missing_prompt_arg_returns_error() {
        let cap = AcpCapability::new(
            Arc::new(CapabilitySystem::new()),
            Arc::new(SessionPool::new()),
        );
        let mut args = HashMap::new();
        args.insert("agent".to_string(), Value::String("claude".to_string()));
        let result = cap.execute(args).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("prompt"),
            "error should mention 'prompt': {err}"
        );
    }

    #[tokio::test]
    async fn unknown_agent_returns_error_with_available_list() {
        let cap = AcpCapability::new(
            Arc::new(CapabilitySystem::new()),
            Arc::new(SessionPool::new()),
        );
        let mut args = HashMap::new();
        args.insert(
            "agent".to_string(),
            Value::String("nonexistent-agent".to_string()),
        );
        args.insert("prompt".to_string(), Value::String("hello".to_string()));
        let result = cap.execute(args).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("nonexistent-agent"),
            "error should mention the agent name: {err}"
        );
        assert!(
            err.contains("claude"),
            "error should list available agents: {err}"
        );
    }

    #[test]
    fn capability_metadata() {
        let cap = AcpCapability::new(
            Arc::new(CapabilitySystem::new()),
            Arc::new(SessionPool::new()),
        );
        assert_eq!(cap.metadata().name, "acp");
        assert!(cap.metadata().tags.contains(&"agent".to_string()));
    }
}
