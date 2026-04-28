//! COMMUNICATE operation - Inter-agent communication
//!
//! Supports four dispatch modes:
//!   - `local` (default): in-process sub-flow execution via FlowRegistry
//!   - `http`: POST to an external APXM agent's `/v1/receive` endpoint
//!   - `acp`: send a prompt to an ACP agent subprocess via the ProcessTable
//!   - `broadcast`: fan-out to ALL registered agents in FlowRegistry in parallel;
//!     returns an Array of all responses (non-fatal errors included as strings)
//!
//! The protocol is selected via the `protocol` node attribute.
//! For HTTP, `recipient` may be a full URL (`http://...`) or an agent name
//! looked up via `APXM_SERVER_URL/v1/agents/{name}`.
//!
//! For ACP, `recipient` must match an agent name previously spawned via
//! `SPAWN_AGENT` with a `profile` attribute. The agent must be registered
//! in the ProcessTable.
//!
//! For BROADCAST, `recipient` is ignored. The message is sent to every agent
//! currently registered in the FlowRegistry; results are collected in parallel.

use super::{
    ExecutionContext, Node, Result, Value, execute_llm_request, get_string_attribute,
    template::{input_names_from_node, render_named},
};
use crate::aam::{ScopeSpec, TransitionLabel};
use crate::executor::ExecutorEngine;
use apxm_backends::LLMRequest;
use apxm_core::constants::communicate_protocols as comm_proto;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::context_stack as context_stack_consts;
use apxm_core::constants::runtime::{belief_keys, metadata, response_keys};
use apxm_core::error::RuntimeError;
use apxm_core::types::{AISOperationType, ProcessPromptMetric};

/// Well-known flow names tried in order when looking up a recipient agent.
const COMMUNICATE_FLOW_NAMES: &[&str] = &["communicate", "main"];

fn message_from_attributes(node: &Node) -> Option<Value> {
    get_string_attribute(node, graph_attrs::MESSAGE)
        .ok()
        .map(Value::String)
        .or_else(|| {
            get_string_attribute(node, graph_attrs::PROMPT)
                .ok()
                .map(Value::String)
        })
        .or_else(|| {
            get_string_attribute(node, graph_attrs::TEMPLATE_STR)
                .ok()
                .map(Value::String)
        })
}

fn resolve_message(node: &Node, protocol: &str, inputs: &[Value]) -> Result<Value> {
    let input_names = input_names_from_node(node);
    let attr_message = message_from_attributes(node);
    let message_input_count = input_names.len();
    let message_inputs = if message_input_count == 0 {
        inputs
    } else {
        &inputs[..message_input_count.min(inputs.len())]
    };

    if let Some(Value::String(template)) = attr_message.as_ref() {
        if !input_names.is_empty() {
            return Ok(Value::String(render_named(
                template,
                message_inputs,
                &input_names,
            )?));
        }
        return Ok(Value::String(template.clone()));
    }

    let fallback = if protocol == comm_proto::ACP {
        message_inputs
            .iter()
            .find(|v| matches!(v, Value::String(_)))
            .cloned()
            .or_else(|| message_inputs.first().cloned())
    } else {
        message_inputs.first().cloned()
    };

    Ok(fallback.or(attr_message).unwrap_or(Value::Null))
}

fn communication_llm_operation(node: &Node) -> Result<AISOperationType> {
    let Some(raw_value) = node.attributes.get(graph_attrs::LLM_OPERATION) else {
        return Ok(AISOperationType::Ask);
    };
    let value = raw_value
        .as_string()
        .ok_or_else(|| RuntimeError::Operation {
            op_type: node.op_type,
            message: format!(
                "COMMUNICATE '{}' must be a string",
                graph_attrs::LLM_OPERATION
            ),
        })?;
    let operation = value
        .parse::<AISOperationType>()
        .map_err(|_| RuntimeError::Operation {
            op_type: node.op_type,
            message: format!(
                "COMMUNICATE has invalid '{}' value '{}'",
                graph_attrs::LLM_OPERATION,
                value
            ),
        })?;
    if !matches!(
        operation,
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
    ) {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!(
                "COMMUNICATE '{}' must be {}, {}, or {}",
                graph_attrs::LLM_OPERATION,
                AISOperationType::Ask,
                AISOperationType::Think,
                AISOperationType::Reason
            ),
        });
    }
    Ok(operation)
}

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Accept both the historical "target" attribute and the current "recipient"
    // emitted by the compiler.
    let recipient = get_string_attribute(node, graph_attrs::RECIPIENT)
        .or_else(|_| get_string_attribute(node, graph_attrs::TARGET))
        .unwrap_or_default();
    let protocol = get_string_attribute(node, graph_attrs::PROTOCOL)
        .unwrap_or_else(|_| comm_proto::LOCAL.to_string());
    let message = resolve_message(node, &protocol, &inputs)?;

    match protocol.as_str() {
        comm_proto::HTTP | comm_proto::HTTPS => {
            return execute_http(ctx, node, &recipient, message).await;
        }
        comm_proto::BROADCAST => return execute_broadcast(ctx, node, message).await,
        comm_proto::ACP => return execute_acp(ctx, node, &recipient, message).await,
        _ => {
            // local — require recipient
            if recipient.is_empty() {
                return Err(RuntimeError::Operation {
                    op_type: node.op_type,
                    message: "COMMUNICATE requires 'recipient' attribute for local protocol"
                        .to_string(),
                });
            }
        }
    }

    tracing::info!(
        execution_id = %ctx.execution_id,
        recipient = %recipient,
        "Executing COMMUNICATE operation"
    );

    // Record the outgoing message in AAM beliefs for observability
    let label = TransitionLabel::Custom(format!("communicate:{}", recipient));
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::PENDING_COMMUNICATE_PREFIX, recipient),
        Value::Object(
            vec![
                (
                    graph_attrs::RECIPIENT.to_string(),
                    Value::String(recipient.clone()),
                ),
                (graph_attrs::MESSAGE.to_string(), message.clone()),
            ]
            .into_iter()
            .collect(),
        ),
        label,
    );

    // Look up the target agent's flow in the FlowRegistry.
    // Try well-known flow names in order: "communicate", then "main".
    let sub_dag = {
        let mut found = None;
        for flow_name in COMMUNICATE_FLOW_NAMES {
            if let Some(dag) = ctx.flow_registry.get_flow(&recipient, flow_name) {
                found = Some(dag);
                break;
            }
        }
        match found {
            Some(dag) => dag,
            None => {
                // Fallback: try inline-spawned agent (SPAWN_AGENT stamps
                // instructions+model into STM) and dispatch via LLM.
                if let Some(response) =
                    communicate_inline_agent(ctx, node, &recipient, &message).await?
                {
                    return Ok(response);
                }

                let available = ctx.flow_registry.flows_for_agent(&recipient);
                let hint = if available.is_empty() {
                    let all_flows = ctx.flow_registry.list_flows();
                    if all_flows.is_empty() {
                        format!(
                            "COMMUNICATE target '{}' not found. No agents registered and no \
                             inline SPAWN_AGENT info in STM.",
                            recipient
                        )
                    } else {
                        format!(
                            "Agent '{}' not found. Registered agents: {}",
                            recipient,
                            all_flows
                                .iter()
                                .map(|(a, f)| format!("{}.{}", a, f))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                } else {
                    format!(
                        "Agent '{}' has no 'communicate' or 'main' flow. Available flows: {}",
                        recipient,
                        available.join(", ")
                    )
                };

                return Err(RuntimeError::Operation {
                    op_type: node.op_type,
                    message: hint,
                });
            }
        }
    };

    tracing::debug!(
        recipient = %recipient,
        nodes = sub_dag.nodes.len(),
        "Found flow for recipient agent, executing sub-DAG"
    );

    // Create a child context for the sub-flow execution
    let child_ctx = ctx
        .child_with_scope(ScopeSpec::snapshot_all())
        .with_metadata(
            metadata::PARENT_EXECUTION_ID.to_string(),
            ctx.execution_id.clone(),
        )
        .with_metadata(
            metadata::COMMUNICATE_SENDER.to_string(),
            ctx.execution_id.clone(),
        )
        .with_metadata(
            metadata::COMMUNICATE_RECIPIENT.to_string(),
            recipient.clone(),
        );

    // Inject the message into STM so the sub-flow can access it
    let _ = child_ctx
        .memory
        .write_scoped(
            crate::memory::MemorySpace::Stm,
            child_ctx.scope_id(),
            belief_keys::COMMUNICATE_MESSAGE.to_string(),
            message.clone(),
        )
        .await;

    // Execute the sub-flow DAG
    let engine = ExecutorEngine::new(child_ctx);
    let dag_to_execute = (*sub_dag).clone();

    let result = engine.execute_dag(dag_to_execute).await.map_err(|e| {
        tracing::error!(
            recipient = %recipient,
            error = %e,
            "Inter-agent communication failed"
        );
        RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Communication with agent '{}' failed: {}", recipient, e),
        }
    })?;

    // Clear the pending belief
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::PENDING_COMMUNICATE_PREFIX, recipient),
        Value::Null,
        TransitionLabel::Custom(format!("communicate_completed:{}", recipient)),
    );

    // Extract the response from the sub-flow's exit nodes
    let response = result
        .results
        .values()
        .find(|v| !matches!(v, Value::Null))
        .cloned()
        .unwrap_or(Value::Null);

    tracing::info!(
        recipient = %recipient,
        duration_ms = result.stats.duration_ms,
        executed_nodes = result.stats.executed_nodes,
        "COMMUNICATE completed successfully"
    );

    Ok(response)
}

// ─── Inline-agent LLM fallback ────────────────────────────────────────────

/// Fallback dispatch when the recipient agent has no registered flow.
///
/// Looks up `agent_info:<recipient>` in STM (written by SPAWN_AGENT) and, if
/// it has a `system_prompt` and/or `model`, dispatches the message as a
/// one-shot LLM call against that agent's config. Returns `Ok(None)` if no
/// inline agent info is present so the caller can emit the original error.
async fn communicate_inline_agent(
    ctx: &ExecutionContext,
    node: &Node,
    recipient: &str,
    message: &Value,
) -> Result<Option<Value>> {
    let key = format!("{}{}", belief_keys::AGENT_INFO_PREFIX, recipient);
    let agent_info = match super::read_stm_with_scope_fallback(ctx, &key).await {
        Some(Value::Object(obj)) => obj,
        _ => return Ok(None),
    };

    let system_prompt = agent_info
        .get(response_keys::SYSTEM_PROMPT)
        .and_then(|v| v.as_string())
        .cloned();
    let backend = agent_info
        .get(response_keys::BACKEND)
        .and_then(|v| v.as_string())
        .cloned();
    let model = agent_info
        .get(response_keys::MODEL)
        .and_then(|v| v.as_string())
        .cloned();

    // Require at least one of system_prompt/backend/model — pure metadata-only entries
    // (e.g. ACP subprocess agents) shouldn't be auto-dispatched as LLMs.
    if system_prompt.is_none() && backend.is_none() && model.is_none() {
        return Ok(None);
    }

    let prompt = message.as_string().cloned().unwrap_or_default();

    let mut request =
        LLMRequest::new(prompt).with_operation_type(communication_llm_operation(node)?);
    if let Some(sp) = system_prompt {
        request = request.with_system_prompt(sp);
    }
    if let Some(b) = backend {
        request = request.with_backend(b);
    }
    if let Some(m) = model {
        request = request.with_model(m);
    }

    tracing::info!(
        execution_id = %ctx.execution_id,
        recipient = %recipient,
        "COMMUNICATE dispatching to inline-spawned agent via LLM"
    );

    let response = execute_llm_request(ctx, node.id, "COMMUNICATE", &request).await?;
    Ok(Some(Value::String(response.content)))
}

// ─── Broadcast dispatch ────────────────────────────────────────────────────

/// Execute a broadcast sub-flow for a single agent.
///
/// Extracted into a standalone async fn so the compiler can prove `Send + 'static`
/// for the returned future — inline async blocks with captured `ExecutionContext`
/// trigger DashMap `Map` trait invariance errors.
async fn broadcast_one_agent(
    child_ctx: ExecutionContext,
    agent: String,
    dag: apxm_core::types::execution::ExecutionDag,
    msg: Value,
) -> (String, Result<Value>) {
    let _ = child_ctx
        .memory
        .write_scoped(
            crate::memory::MemorySpace::Stm,
            child_ctx.scope_id(),
            belief_keys::COMMUNICATE_MESSAGE.to_string(),
            msg,
        )
        .await;

    let engine = ExecutorEngine::new(child_ctx);
    match engine.execute_dag(dag).await {
        Ok(result) => {
            let response = result
                .results
                .values()
                .find(|v| !matches!(v, Value::Null))
                .cloned()
                .unwrap_or(Value::Null);
            (agent, Ok(response))
        }
        Err(e) => (agent, Err(e)),
    }
}

/// Fan-out COMMUNICATE to ALL agents registered in the FlowRegistry in parallel.
///
/// Dispatches to every unique agent name that has a known "communicate" or "main"
/// flow.  Each agent is called concurrently via `tokio::spawn`.  Non-fatal errors
/// are captured as `Value::String("<agent>: <error>")` so the caller receives a
/// complete picture rather than a partial failure.
///
/// Returns `Value::Array` of all responses (one per agent, in arbitrary order).
async fn execute_broadcast(ctx: &ExecutionContext, _node: &Node, message: Value) -> Result<Value> {
    // Collect unique agent names that have a usable flow
    let all_flows = ctx.flow_registry.list_flows();
    let agent_names: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        let mut agents = Vec::new();
        for (agent_name, flow_name) in &all_flows {
            if COMMUNICATE_FLOW_NAMES.contains(&flow_name.as_str())
                && seen.insert(agent_name.clone())
            {
                agents.push(agent_name.clone());
            }
        }
        agents
    };

    if agent_names.is_empty() {
        tracing::warn!(
            execution_id = %ctx.execution_id,
            "BROADCAST: no agents with communicate/main flows registered"
        );
        return Ok(Value::Array(vec![]));
    }

    tracing::info!(
        execution_id = %ctx.execution_id,
        agents = agent_names.len(),
        "COMMUNICATE BROADCAST fan-out starting"
    );

    let mut handles = Vec::with_capacity(agent_names.len());
    for agent_name in &agent_names {
        // Find the sub-DAG
        let sub_dag = {
            let mut found = None;
            for flow_name in COMMUNICATE_FLOW_NAMES {
                if let Some(dag) = ctx.flow_registry.get_flow(agent_name, flow_name) {
                    found = Some(dag);
                    break;
                }
            }
            match found {
                Some(dag) => dag,
                None => continue,
            }
        };

        // Build a child context per recipient
        let child_ctx = ctx
            .child_with_scope(ScopeSpec::snapshot_all())
            .with_metadata(
                metadata::PARENT_EXECUTION_ID.to_string(),
                ctx.execution_id.clone(),
            )
            .with_metadata(
                metadata::COMMUNICATE_SENDER.to_string(),
                ctx.execution_id.clone(),
            )
            .with_metadata(
                metadata::COMMUNICATE_RECIPIENT.to_string(),
                agent_name.clone(),
            )
            .with_metadata(
                metadata::COMMUNICATE_MODE.to_string(),
                comm_proto::BROADCAST.to_string(),
            );

        let msg = message.clone();
        let agent = agent_name.clone();
        let dag_clone = (*sub_dag).clone();

        handles.push(tokio::spawn(broadcast_one_agent(
            child_ctx, agent, dag_clone, msg,
        )));
    }

    // Collect results — non-fatal errors become string values
    let mut responses = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok((agent, Ok(val))) => {
                tracing::debug!(agent = %agent, "BROADCAST response received");
                responses.push(val);
            }
            Ok((agent, Err(e))) => {
                tracing::warn!(agent = %agent, error = %e, "BROADCAST agent returned error");
                responses.push(Value::String(format!("{}: {}", agent, e)));
            }
            Err(join_err) => {
                tracing::error!(error = %join_err, "BROADCAST task panicked");
                responses.push(Value::String(format!("panic: {}", join_err)));
            }
        }
    }

    tracing::info!(
        execution_id = %ctx.execution_id,
        responses = responses.len(),
        "COMMUNICATE BROADCAST completed"
    );

    Ok(Value::Array(responses))
}

// ─── ACP dispatch ──────────────────────────────────────────────────────────

/// Dispatch COMMUNICATE to an ACP agent subprocess via the ProcessTable.
///
/// The recipient must have been previously spawned via `SPAWN_AGENT` with a
/// `profile` attribute. The message is sent as a `session/prompt` request
/// via the live ACP connection. The response is returned as a structured
/// `Value::Object`.
async fn execute_acp(
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

    // Look up the agent process — clone out of the DashMap guard immediately
    // to avoid holding the shard lock across the async prompt boundary.
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

    // Get the prompter
    let prompter =
        ctx.process_table
            .agent_prompter()
            .await
            .ok_or_else(|| RuntimeError::Operation {
                op_type: node.op_type,
                message: "No AgentPrompter configured. Cannot send ACP prompts.".to_string(),
            })?;

    // Convert message to prompt text
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

    // Record the outgoing message in AAM beliefs for observability
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
                    Value::String(comm_proto::ACP.to_string()),
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
        emitter.emit_llm_prompt(node.id, &enriched_prompt);
    }

    // Send prompt via the AgentPrompter trait and record the node-owned
    // spawned-agent turn before returning the node output.
    let prompt_start = std::time::Instant::now();
    let prompt_response = match prompter.prompt(&process, &enriched_prompt).await {
        Ok(response) => response,
        Err(error) => {
            ctx.graph_metrics.record_turn(ProcessPromptMetric {
                node_id: node.id,
                agent_name: process.name.clone(),
                process_id: process.id.clone(),
                protocol: comm_proto::ACP.to_string(),
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
        protocol: comm_proto::ACP.to_string(),
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
            emitter.emit_token_usage(node.id, input_tokens, output_tokens);
        }
    }

    // Return plain text to downstream nodes; store the full typed response in
    // beliefs for observability.
    let text_output = Value::String(prompt_response.text.clone());
    let response = prompt_response.to_value(&process.name);

    // Clear the pending belief
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::PENDING_COMMUNICATE_PREFIX, recipient),
        Value::Null,
        TransitionLabel::Custom(format!("communicate_acp_completed:{}", recipient)),
    );

    // Store full response object in beliefs for observability
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

// ─── HTTP dispatch ─────────────────────────────────────────────────────────

/// Dispatch COMMUNICATE over HTTP to an external APXM agent.
///
/// `recipient` may be:
///   - A full URL: `http://host:port` — used directly as base URL.
///   - An agent name: looked up via `APXM_SERVER_URL/v1/agents/{name}`.
async fn execute_http(
    ctx: &ExecutionContext,
    node: &Node,
    recipient: &str,
    message: Value,
) -> Result<Value> {
    let op_err = |msg: String| RuntimeError::Operation {
        op_type: node.op_type,
        message: msg,
    };

    // Resolve base URL
    let base_url = if recipient.starts_with("http://") || recipient.starts_with("https://") {
        recipient.to_string()
    } else {
        // Name-based lookup via APXM server agent registry
        let server_url = std::env::var("APXM_SERVER_URL")
            .unwrap_or_else(|_| "http://localhost:18800".to_string());
        let lookup_url = format!("{}/v1/agents/{}", server_url, recipient);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| op_err(format!("Failed to build HTTP client: {e}")))?;
        let resp = client
            .get(&lookup_url)
            .send()
            .await
            .map_err(|e| op_err(format!("Agent registry lookup failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(op_err(format!(
                "Agent '{}' not found in registry ({})",
                recipient,
                resp.status()
            )));
        }
        let agent_info: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| op_err(format!("Failed to parse agent info: {e}")))?;
        agent_info["url"]
            .as_str()
            .ok_or_else(|| op_err(format!("Agent '{}' has no 'url' field", recipient)))?
            .to_string()
    };

    // Serialize message
    let msg_json = message
        .to_json()
        .map_err(|e| op_err(format!("Failed to serialize message: {e}")))?;

    // POST to /v1/receive
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| op_err(format!("Failed to build HTTP client: {e}")))?;

    let receive_url = format!("{}/v1/receive", base_url.trim_end_matches('/'));
    tracing::info!(
        execution_id = %ctx.execution_id,
        recipient = %recipient,
        url = %receive_url,
        "COMMUNICATE HTTP dispatch"
    );

    let resp = client
        .post(&receive_url)
        .json(&serde_json::json!({
            "from": ctx.execution_id,
            "message": msg_json
        }))
        .send()
        .await
        .map_err(|e| op_err(format!("HTTP COMMUNICATE to '{}' failed: {e}", recipient)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(op_err(format!(
            "HTTP COMMUNICATE '{}' returned {}: {}",
            recipient, status, text
        )));
    }

    let result: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| op_err(format!("Failed to parse COMMUNICATE response: {e}")))?;

    Value::try_from(result).map_err(|e| op_err(format!("Failed to convert response: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::capability::flow_registry::FlowRegistry;
    use crate::context_stack::{ContextStack, NodeMetadata as ContextNodeMetadata};
    use crate::executor::events::ExecutionEventEmitter;
    use crate::memory::{MemoryConfig, MemorySystem};
    use crate::testing::{
        MOCK_AGENT_PROFILE, MOCK_MODEL_NAME, MOCK_SESSION_ID, MOCK_STOP_REASON,
        MockUsageAgentPrompter, RecordingAgentPrompter,
    };
    use apxm_backends::LLMRegistry;
    use apxm_core::paths::session_node_dir_name;
    use apxm_core::types::{
        execution::{ExecutionDag, NodeMetadata},
        operations::AISOperationType,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn communicate_node_with_llm_operation(value: Value) -> Node {
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes
            .insert(graph_attrs::LLM_OPERATION.to_string(), value);
        node
    }

    #[derive(Default)]
    struct RecordingEmitter {
        prompts: Mutex<Vec<(u64, String)>>,
    }

    impl RecordingEmitter {
        fn prompts(&self) -> Vec<(u64, String)> {
            self.prompts.lock().expect("prompt lock").clone()
        }
    }

    impl ExecutionEventEmitter for RecordingEmitter {
        fn emit_llm_token(&self, _content: &str) {}

        fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}

        fn emit_tool_end(&self, _name: &str, _result: &Value) {}

        fn emit_llm_prompt(&self, node_id: u64, prompt: &str) {
            self.prompts
                .lock()
                .expect("prompt lock")
                .push((node_id, prompt.to_string()));
        }
    }

    #[test]
    fn communication_llm_operation_accepts_llm_turn_operations() {
        let node =
            communicate_node_with_llm_operation(Value::String(AISOperationType::Think.to_string()));

        assert_eq!(
            communication_llm_operation(&node).expect("valid operation"),
            AISOperationType::Think
        );
    }

    #[test]
    fn communication_llm_operation_rejects_non_llm_turn_operations() {
        let node =
            communicate_node_with_llm_operation(Value::String(AISOperationType::Plan.to_string()));

        assert!(communication_llm_operation(&node).is_err());
    }

    fn create_echo_dag() -> ExecutionDag {
        let mut const_node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::ConstStr,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        const_node.attributes.insert(
            graph_attrs::VALUE.to_string(),
            Value::String("ack from agent".to_string()),
        );
        ExecutionDag {
            nodes: vec![const_node],
            edges: vec![],
            entry_nodes: vec![1],
            exit_nodes: vec![1],
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    async fn test_communicate_with_registered_agent() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        // Register a "communicate" flow for "PeerAgent"
        flow_registry.register_flow("PeerAgent", "communicate", create_echo_dag());

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );
        let ctx = ExecutionContext {
            flow_registry,
            ..ctx
        };

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("PeerAgent".to_string()),
        );

        let result = execute(&ctx, &node, vec![Value::String("hello".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::String("ack from agent".to_string()));
    }

    #[tokio::test]
    async fn test_communicate_acp_prepends_context_stack_frames() {
        let dir = tempdir().expect("tempdir");
        let session_dir = dir.path().join("session");
        let upstream_dir = session_dir
            .join(apxm_core::constants::session::files::NODES_DIR)
            .join(session_node_dir_name(1, "seed"));
        std::fs::create_dir_all(&upstream_dir).expect("upstream dir");
        std::fs::write(
            upstream_dir.join(apxm_core::constants::session::node::OUTPUT_JSON),
            "{\"result\":\"upstream data\"}",
        )
        .expect("output");

        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let process_table = Arc::new(crate::process_table::ProcessTable::new());
        process_table
            .spawn_local("PeerAgent".to_string(), None)
            .expect("spawn local process");

        let prompts = Arc::new(Mutex::new(Vec::new()));
        process_table
            .set_agent_prompter(Arc::new(RecordingAgentPrompter::new(Arc::clone(&prompts))))
            .await;

        let context_stack = Arc::new(ContextStack::new(
            session_dir.clone(),
            Arc::new(HashMap::from([
                (
                    1,
                    ContextNodeMetadata {
                        name: "seed".to_string(),
                        op_type: AISOperationType::ConstStr,
                    },
                ),
                (
                    2,
                    ContextNodeMetadata {
                        name: "communicate_peer".to_string(),
                        op_type: AISOperationType::Communicate,
                    },
                ),
            ])),
            Arc::new(vec![(1, 2)]),
        ));

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
        .with_context_stack(context_stack)
        .with_process_table(process_table);

        let mut node = apxm_core::types::execution::Node {
            id: 2,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("PeerAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            Value::String(comm_proto::ACP.to_string()),
        );
        node.attributes.insert(
            graph_attrs::PROFILE.to_string(),
            Value::String(MOCK_AGENT_PROFILE.to_string()),
        );

        let result = execute(
            &ctx,
            &node,
            vec![Value::String("Respond to the user".to_string())],
        )
        .await
        .unwrap();

        let recorded = prompts.lock().expect("prompt lock");
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].contains("## Session"));
        assert!(recorded[0].contains("## Upstream: seed (#1)"));
        assert!(recorded[0].contains("upstream data"));
        assert!(recorded[0].contains("Respond to the user"));
        assert_eq!(result, Value::String(recorded[0].clone()));
    }

    #[tokio::test]
    async fn test_communicate_acp_uses_message_attribute_when_inputs_missing() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let process_table = Arc::new(crate::process_table::ProcessTable::new());
        process_table
            .spawn_local("PeerAgent".to_string(), None)
            .expect("spawn local process");

        let prompts = Arc::new(Mutex::new(Vec::new()));
        process_table
            .set_agent_prompter(Arc::new(RecordingAgentPrompter::new(Arc::clone(&prompts))))
            .await;

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
        .with_process_table(process_table);

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("PeerAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            Value::String(comm_proto::ACP.to_string()),
        );
        node.attributes.insert(
            graph_attrs::MESSAGE.to_string(),
            Value::String("Respond from attribute".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::String("Respond from attribute".to_string()));
        let recorded = prompts.lock().expect("prompt lock");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0], "Respond from attribute");
    }

    #[tokio::test]
    async fn test_communicate_acp_emits_actual_prompt() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let process_table = Arc::new(crate::process_table::ProcessTable::new());
        process_table
            .spawn_local("PeerAgent".to_string(), None)
            .expect("spawn local process");
        process_table
            .set_agent_prompter(Arc::new(RecordingAgentPrompter::new(Arc::new(Mutex::new(
                Vec::new(),
            )))))
            .await;

        let emitter = Arc::new(RecordingEmitter::default());
        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
        .with_process_table(process_table)
        .with_event_emitter(Some(emitter.clone() as Arc<dyn ExecutionEventEmitter>));

        let mut node = apxm_core::types::execution::Node {
            id: 42,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![7],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("PeerAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            Value::String(comm_proto::ACP.to_string()),
        );
        node.attributes.insert(
            graph_attrs::MESSAGE.to_string(),
            Value::String("Review {task}".to_string()),
        );
        node.attributes.insert(
            graph_attrs::INPUT_NAMES.to_string(),
            Value::Array(vec![Value::String("task".to_string())]),
        );

        let result = execute(&ctx, &node, vec![Value::String("runtime task".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::String("Review runtime task".to_string()));

        assert_eq!(
            emitter.prompts(),
            vec![(42, "Review runtime task".to_string())]
        );
    }

    #[tokio::test]
    async fn test_communicate_acp_records_metrics_with_mock_prompter() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let process_table = Arc::new(crate::process_table::ProcessTable::new());
        let process_id = process_table
            .spawn_local("PeerAgent".to_string(), None)
            .expect("spawn local process");
        process_table
            .set_agent_prompter(Arc::new(
                MockUsageAgentPrompter::new("mock response: ")
                    .with_session_id(MOCK_SESSION_ID)
                    .with_turn(1)
                    .with_model(MOCK_MODEL_NAME)
                    .with_stop_reason(MOCK_STOP_REASON)
                    .with_token_usage(Some(10), Some(5)),
            ))
            .await;

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
        .with_process_table(process_table);

        let mut node = apxm_core::types::execution::Node {
            id: 7,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("PeerAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            Value::String(comm_proto::ACP.to_string()),
        );

        let result = execute(&ctx, &node, vec![Value::String("hello".to_string())])
            .await
            .unwrap();

        assert_eq!(
            result,
            Value::String("mock response: hello".to_string()),
            "handler should return the mock prompter text"
        );

        let snapshot = ctx.graph_metrics.snapshot();
        let node_metrics = snapshot.nodes.get(&node.id).expect("node metrics");
        assert_eq!(node_metrics.processes.totals.prompt_turns, 1);
        assert_eq!(node_metrics.processes.totals.input_tokens, 10);
        assert_eq!(node_metrics.processes.totals.output_tokens, 5);
        assert_eq!(snapshot.graph.processes.total_tokens, 15);
        assert_eq!(snapshot.aggregates.by_agent["PeerAgent"].prompt_turns, 1);

        let turn = &node_metrics.processes.prompt_turns[0];
        assert_eq!(turn.process_id, process_id);
        assert_eq!(turn.protocol, comm_proto::ACP);
        assert_eq!(turn.session_id.as_deref(), Some(MOCK_SESSION_ID));
        assert_eq!(turn.model.as_deref(), Some(MOCK_MODEL_NAME));
        assert_eq!(turn.stop_reason.as_deref(), Some(MOCK_STOP_REASON));

        let tokens = ctx.token_accountant.get_node(node.id).expect("tokens");
        assert_eq!(tokens.input_tokens, 10);
        assert_eq!(tokens.output_tokens, 5);
    }

    #[tokio::test]
    async fn test_communicate_acp_rejects_empty_message() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let process_table = Arc::new(crate::process_table::ProcessTable::new());
        process_table
            .spawn_local("PeerAgent".to_string(), None)
            .expect("spawn local process");

        let prompts = Arc::new(Mutex::new(Vec::new()));
        process_table
            .set_agent_prompter(Arc::new(RecordingAgentPrompter::new(Arc::clone(&prompts))))
            .await;

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
        .with_process_table(process_table);

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("PeerAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            Value::String(comm_proto::ACP.to_string()),
        );

        let error = execute(&ctx, &node, vec![])
            .await
            .expect_err("empty prompt should fail");
        assert!(
            error
                .to_string()
                .contains("requires a non-empty message or string input")
        );
        assert!(prompts.lock().expect("prompt lock").is_empty());
    }

    #[tokio::test]
    async fn test_communicate_agent_not_found() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("NonExistent".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No agents"));
    }
}
