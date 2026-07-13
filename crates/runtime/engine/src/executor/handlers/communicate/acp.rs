//! ACP COMMUNICATE: dispatch to a subprocess agent via the ProcessTable.

use super::super::{ExecutionContext, Node, Result, Value};
use crate::aam::TransitionLabel;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::{belief_keys, context_stack as context_stack_consts};
use apxm_core::error::RuntimeError;
use apxm_core::events::payload::GenerationIdentity;
use apxm_core::types::{CommunicateProtocol, ProcessPromptMetric};

/// Dispatch COMMUNICATE to an ACP agent subprocess via the ProcessTable.
///
/// The recipient must have been previously spawned via `SPAWN_AGENT` with a
/// `profile` attribute. The message is sent as a `session/prompt` request
/// via the live ACP connection. The response is returned as plain text;
/// the full structured response is recorded in beliefs for observability.
pub(super) async fn execute_acp(
    ctx: &ExecutionContext,
    node: &Node,
    recipient: &str,
    message: Value,
) -> Result<Value> {
    if recipient.is_empty() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: "COMMUNICATE(acp) requires a 'recipient' attribute".to_string(),
        });
    }

    tracing::info!(
        execution_id = %ctx.execution_id,
        recipient = %recipient,
        "COMMUNICATE ACP dispatch"
    );

    let process = {
        let guard = ctx.process_table.get_by_name(recipient).ok_or_else(|| {
            let names = ctx.process_table.list_process_names();
            let hint = if names.is_empty() {
                "No agent processes are registered. Did you SPAWN_AGENT first?".to_string()
            } else {
                format!(
                    "Agent '{}' not found in ProcessTable. Active agents: {}",
                    recipient,
                    names.join(", ")
                )
            };
            RuntimeError::Operation {
                op_type: node.op_type,
                message: hint,
            }
        })?;
        guard.clone()
    };

    let prompter =
        ctx.process_table
            .agent_prompter()
            .await
            .ok_or_else(|| RuntimeError::Operation {
                op_type: node.op_type,
                message: "No AgentPrompter configured. Cannot send ACP prompts.".to_string(),
            })?;

    let prompt_text = match &message {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other
            .to_json()
            .map(|j| serde_json::to_string(&j).unwrap_or_default())
            .unwrap_or_default(),
    };

    if prompt_text.trim().is_empty() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!(
                "COMMUNICATE(acp) to '{}' requires a non-empty message or string input",
                recipient
            ),
        });
    }

    let label = TransitionLabel::Custom(format!("communicate_acp:{}", recipient));
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::PENDING_COMMUNICATE_PREFIX, recipient),
        Value::Object(
            vec![
                (
                    graph_attrs::RECIPIENT.to_string(),
                    Value::String(recipient.to_string()),
                ),
                (
                    graph_attrs::PROTOCOL.to_string(),
                    Value::String(CommunicateProtocol::Acp.as_str().to_string()),
                ),
                (graph_attrs::MESSAGE.to_string(), message.clone()),
            ]
            .into_iter()
            .collect(),
        ),
        label,
    );

    let enriched_prompt = if let Some(ref stack) = ctx.context_stack {
        let profile = node
            .attributes
            .get(graph_attrs::PROFILE)
            .and_then(|value| value.as_str())
            .unwrap_or(context_stack_consts::DEFAULT_PROFILE);
        let assembly = stack.assemble(
            node.id,
            profile,
            context_stack_consts::DEFAULT_PROMPT_BUDGET_TOKENS,
        );

        if assembly.frames.is_empty() {
            prompt_text.clone()
        } else {
            format!("{}\n\n---\n\n{}", assembly, prompt_text)
        }
    } else {
        prompt_text.clone()
    };

    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_llm_prompt_with_name(node.id, node.metadata.name.as_deref(), &enriched_prompt);
    }

    let prompt_start = std::time::Instant::now();
    let prompt_response = match prompter.prompt(&process, &enriched_prompt).await {
        Ok(response) => response,
        Err(error) => {
            ctx.graph_metrics.record_turn(ProcessPromptMetric {
                node_id: node.id,
                agent_name: process.name.clone(),
                process_id: process.id.clone(),
                protocol: CommunicateProtocol::Acp.as_str().to_string(),
                session_id: None,
                turn: None,
                model: None,
                stop_reason: None,
                duration_ms: prompt_start.elapsed().as_millis() as u64,
                input_tokens: None,
                output_tokens: None,
                response_bytes: 0,
                success: false,
                error: Some(error.to_string()),
            });
            return Err(error);
        }
    };

    let prompt_duration_ms = prompt_start.elapsed().as_millis() as u64;
    let response_bytes = prompt_response.text.len();
    ctx.graph_metrics.record_turn(ProcessPromptMetric {
        node_id: node.id,
        agent_name: process.name.clone(),
        process_id: process.id.clone(),
        protocol: CommunicateProtocol::Acp.as_str().to_string(),
        session_id: prompt_response.session_id.clone(),
        turn: prompt_response.turn,
        model: prompt_response.model.clone(),
        stop_reason: prompt_response.stop_reason.clone(),
        duration_ms: prompt_duration_ms,
        input_tokens: prompt_response.token_usage.input_tokens,
        output_tokens: prompt_response.token_usage.output_tokens,
        response_bytes,
        success: true,
        error: None,
    });

    let generation = prompt_generation_identity(&prompt_response);
    if let (Some(input_tokens), Some(output_tokens)) = (
        prompt_response.token_usage.input_tokens,
        prompt_response.token_usage.output_tokens,
    ) {
        ctx.token_accountant.record(
            node.id,
            input_tokens,
            output_tokens,
            None,
            Some(&process.name),
        );
        if let Some(emitter) = &ctx.event_emitter {
            emitter.emit_token_usage_with_generation(
                node.id,
                input_tokens,
                output_tokens,
                generation.as_ref(),
            );
        }
    }

    let text_output = Value::String(prompt_response.text.clone());
    let response = prompt_response.to_value(&process.name);

    ctx.aam.set_belief(
        format!("{}{}", belief_keys::PENDING_COMMUNICATE_PREFIX, recipient),
        Value::Null,
        TransitionLabel::Custom(format!("communicate_acp_completed:{}", recipient)),
    );

    ctx.aam.set_belief(
        format!(
            "{}{}:last",
            belief_keys::PENDING_COMMUNICATE_PREFIX,
            recipient
        ),
        response,
        TransitionLabel::Custom(format!("communicate_acp_completed:{}", recipient)),
    );

    tracing::info!(
        execution_id = %ctx.execution_id,
        recipient = %recipient,
        response_len = response_bytes,
        "COMMUNICATE ACP completed"
    );

    Ok(text_output)
}

fn prompt_generation_identity(
    prompt_response: &crate::process_table::AgentPromptResponse,
) -> Option<GenerationIdentity> {
    let session_id = prompt_response
        .session_id
        .as_deref()
        .filter(|session_id| !session_id.is_empty())
        .or_else(|| {
            prompt_response
                .agent_session_id
                .as_deref()
                .filter(|session_id| !session_id.is_empty())
        })?;
    let turn = prompt_response.turn.filter(|turn| *turn > 0)?;

    // ACP reports one aggregated `session/prompt` completion per turn. Treat
    // that observed turn as one canonical generation so retry/step semantics
    // stay aligned with the normal LLM path: one attempt, one physical step.
    Some(GenerationIdentity::new(
        format!("{session_id}/turn/{turn}"),
        1,
        1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::executor::{EmitterAdapter, ExecutionEventEmitter};
    use crate::memory::{MemoryConfig, MemorySystem};
    use crate::testing::{
        MOCK_AGENT_NAME, MOCK_AGENT_PROFILE, MOCK_SESSION_ID, MockUsageAgentPrompter,
    };
    use apxm_backends::LLMRegistry;
    use apxm_core::events::payload::TokenUsagePayload;
    use apxm_core::events::{ApxmEvent, EventEmitter, EventSource};
    use apxm_core::types::operations::AISOperationType;
    use parking_lot::Mutex as PlMutex;
    use std::sync::Arc;

    #[derive(Default)]
    struct CapturingEmitter {
        events: PlMutex<Vec<ApxmEvent>>,
    }

    impl EventEmitter for CapturingEmitter {
        fn emit(&self, event: ApxmEvent) {
            self.events.lock().push(event);
        }
    }

    fn adapter_with_capture() -> (Arc<dyn ExecutionEventEmitter>, Arc<CapturingEmitter>) {
        let capture = Arc::new(CapturingEmitter::default());
        let adapter: Arc<dyn ExecutionEventEmitter> = Arc::new(EmitterAdapter::new(
            capture.clone() as Arc<dyn EventEmitter>,
            EventSource::Runtime,
            "trace-acp-communicate",
        ));
        (adapter, capture)
    }

    async fn test_context(emitter: Option<Arc<dyn ExecutionEventEmitter>>) -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        ExecutionContext::new(memory, Arc::new(LLMRegistry::new()), capability_system, aam)
            .with_event_emitter(emitter)
    }

    fn node_with(recipient: &str) -> Node {
        let mut node = Node::new(2, AISOperationType::Communicate);
        node.set_attribute(
            graph_attrs::RECIPIENT.to_string(),
            Value::String(recipient.to_string()),
        );
        node
    }

    fn external_session() -> Arc<tokio::sync::Mutex<dyn std::any::Any + Send + Sync>> {
        Arc::new(tokio::sync::Mutex::new(()))
    }

    #[tokio::test]
    async fn communicate_acp_token_usage_carries_generation_identity() {
        let (emitter, capture) = adapter_with_capture();
        let ctx = test_context(Some(emitter)).await;

        ctx.process_table
            .set_agent_prompter(Arc::new(
                MockUsageAgentPrompter::new("reply:")
                    .with_session_id(MOCK_SESSION_ID)
                    .with_turn(3)
                    .with_token_usage(Some(11), Some(13)),
            ))
            .await;
        ctx.process_table
            .register_external(
                MOCK_AGENT_NAME.to_string(),
                None,
                external_session(),
                MOCK_AGENT_PROFILE.to_string(),
            )
            .expect("register external ACP process");

        let node = node_with(MOCK_AGENT_NAME);
        let result = execute_acp(
            &ctx,
            &node,
            MOCK_AGENT_NAME,
            Value::String("hello ACP".to_string()),
        )
        .await
        .expect("ACP communicate should succeed");

        assert_eq!(result, Value::String("reply:hello ACP".to_string()));

        let events = capture.events.lock();
        let token_usage = events
            .iter()
            .find_map(|event| event.payload.downcast_ref::<TokenUsagePayload>())
            .expect("token_usage event");
        let generation = token_usage.generation.as_ref().expect("generation");

        assert_eq!(token_usage.node_id, 2);
        assert_eq!(token_usage.input_tokens, 11);
        assert_eq!(token_usage.output_tokens, 13);
        assert_eq!(generation.call_id, format!("{MOCK_SESSION_ID}/turn/3"));
        assert_eq!(generation.attempt, 1);
        assert_eq!(generation.step_number, 1);
    }
}
