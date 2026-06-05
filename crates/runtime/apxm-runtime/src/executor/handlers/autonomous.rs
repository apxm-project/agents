//! AUTONOMOUS operation - Autonomous execution with goal-directed loop
//!
//! Implements autonomous agent behavior via an iterative loop:
//! 1. Plans next action based on goal + current state
//! 2. Executes the action (via LLM call)
//! 3. Evaluates progress toward goal
//! 4. Continues until goal is met or max_iterations reached

use super::{
    ExecutionContext, Node, Result, Value, apply_llm_request_routing_from_node,
    execute_llm_request_for_node, get_input, get_optional_string_attribute,
    get_optional_u64_attribute,
    llm::{attach_graph_hints, resolve_node_tools, run_tool_loop},
};
use crate::aam::TransitionLabel;
use apxm_backends::{LLMRequest, ToolChoice, ToolDefinition};
use apxm_core::constants::{graph::attrs as graph_attrs, runtime::belief_keys};
use apxm_core::error::RuntimeError;

const DEFAULT_MAX_ITERATIONS: u64 = 10;
/// Idle poll cadence for RECV mode when the node sets no `poll_interval_ms`.
const DEFAULT_POLL_INTERVAL_MS: u64 = 2_000;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Converse mode: multi-turn loop inside the handler, gated by `converse = "true"`.
    // Turns arrive as a JSON-array string in input 0.
    if get_optional_string_attribute(node, "converse")?.as_deref() == Some("true") {
        return converse_loop(ctx, node, inputs).await;
    }
    // Recv mode: an artifact-resident wait. The node PARKS until an external
    // event arrives at `recv_url`, runs one agent turn per event, and (in
    // once-mode) returns after the first — so "wait for an event, then act" is a
    // node in the graph rather than a host loop. Attr-gated like `converse`, so
    // it adds no new op (the 44-op invariant holds).
    if get_optional_string_attribute(node, "mode")?.as_deref() == Some("recv") {
        return recv_loop(ctx, node, inputs).await;
    }
    let goal = if let Some(goal) = get_optional_string_attribute(node, graph_attrs::PROMPT)? {
        goal
    } else if let Some(goal) = get_optional_string_attribute(node, graph_attrs::TEMPLATE_STR)? {
        goal
    } else if let Some(goal) = get_optional_string_attribute(node, graph_attrs::REGION)? {
        goal
    } else {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Missing required attribute: {}", graph_attrs::PROMPT),
        });
    };

    // Get initial state from inputs
    let initial_state = if !inputs.is_empty() {
        get_input(node, &inputs, 0)?
    } else {
        Value::String("No initial state provided".to_string())
    };

    // Get max iterations
    let max_iterations = get_optional_u64_attribute(node, graph_attrs::MAX_ITERATIONS)?
        .unwrap_or(DEFAULT_MAX_ITERATIONS);

    tracing::info!(
        execution_id = %ctx.execution_id,
        node_id = node.id,
        goal = %goal,
        max_iterations = max_iterations,
        "Starting AUTONOMOUS execution loop"
    );

    let mut current_state = initial_state;
    let mut iteration = 0u64;
    let mut final_result = Value::Null;

    while iteration < max_iterations {
        iteration += 1;

        tracing::debug!(
            execution_id = %ctx.execution_id,
            node_id = node.id,
            iteration = iteration,
            "Autonomous loop iteration"
        );

        // Step 1: Plan next action
        let plan_prompt = format!(
            "You are an autonomous agent working toward this goal:\n\n{}\n\n\
            Current state:\n{}\n\n\
            Based on the current state, what is the next action you should take to progress toward the goal? \
            Respond with a brief action plan.",
            goal,
            format_state(&current_state)
        );

        let plan_req =
            apply_llm_request_routing_from_node(LLMRequest::new(plan_prompt.clone()), node)?;

        let plan_response = execute_llm_request_for_node(ctx, node, "autonomous_plan", &plan_req)
            .await
            .map_err(|e| RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("Failed to plan action (iteration {}): {}", iteration, e),
            })?;

        tracing::debug!(
            execution_id = %ctx.execution_id,
            node_id = node.id,
            iteration = iteration,
            "Planned action"
        );

        // Step 2: Execute the action
        let action_prompt = format!(
            "Goal: {}\n\n\
            Current state:\n{}\n\n\
            Planned action:\n{}\n\n\
            Execute this action and report the result.",
            goal,
            format_state(&current_state),
            plan_response.content
        );

        let mut action_req =
            apply_llm_request_routing_from_node(LLMRequest::new(action_prompt), node)?;

        // If the autonomous node exposes tools (tool_groups / tools / tools_enabled),
        // run the real model->tool->model loop so the agent can ACT on its decision
        // (actually invoke capabilities), not just describe an action. Opt-in and
        // default-safe: no tool attrs => empty tools => the original text-only path.
        //
        // Caveat: the backend capability check goes through `ctx.llm_registry`,
        // which may differ from a node-routed ModelRouter backend; this mirrors
        // the ASK handler's check exactly (llm/mod.rs).
        let tools = resolve_node_tools(ctx, node);
        let action_content = if tools.is_empty() {
            execute_llm_request_for_node(ctx, node, "autonomous_action", &action_req)
                .await
                .map_err(|e| RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!("Failed to execute action (iteration {}): {}", iteration, e),
                })?
                .content
        } else {
            let backend_name = ctx.llm_registry.resolve_backend_name(&action_req).ok();
            let supports = backend_name
                .as_deref()
                .and_then(|name| ctx.llm_registry.get_backend(name))
                .map(|b| b.supports_auto_tool_choice())
                .unwrap_or(false);
            if supports {
                action_req = attach_graph_hints(ctx, node, action_req);
                action_req = action_req.with_tools(tools).with_tool_choice(ToolChoice::Auto);
                let value = run_tool_loop(ctx, node, &action_req).await.map_err(|e| {
                    RuntimeError::Operation {
                        op_type: node.op_type,
                        message: format!("Failed tool-loop action (iteration {}): {}", iteration, e),
                    }
                })?;
                match value {
                    Value::String(s) => s,
                    other => format_state(&other),
                }
            } else {
                // Backend can't accept tool_choice=auto; degrade to text-only.
                execute_llm_request_for_node(ctx, node, "autonomous_action", &action_req)
                    .await
                    .map_err(|e| RuntimeError::Operation {
                        op_type: node.op_type,
                        message: format!(
                            "Failed to execute action (iteration {}): {}",
                            iteration, e
                        ),
                    })?
                    .content
            }
        };

        current_state = Value::String(action_content.clone());

        // Step 3: Evaluate progress
        let eval_prompt = format!(
            "Goal: {}\n\n\
            Current state after action:\n{}\n\n\
            Has the goal been achieved? Respond with ONLY 'YES' if the goal is fully achieved, or 'NO' if more work is needed.",
            goal, action_content
        );

        let eval_req = apply_llm_request_routing_from_node(LLMRequest::new(eval_prompt), node)?;

        let eval_response = execute_llm_request_for_node(ctx, node, "autonomous_eval", &eval_req)
            .await
            .map_err(|e| RuntimeError::Operation {
                op_type: node.op_type,
                message: format!(
                    "Failed to evaluate progress (iteration {}): {}",
                    iteration, e
                ),
            })?;

        // Check if goal is achieved
        if eval_response
            .content
            .trim()
            .to_uppercase()
            .starts_with("YES")
        {
            tracing::info!(
                execution_id = %ctx.execution_id,
                node_id = node.id,
                iteration = iteration,
                "Goal achieved"
            );
            final_result = current_state;
            break;
        }

        if iteration >= max_iterations {
            tracing::warn!(
                execution_id = %ctx.execution_id,
                node_id = node.id,
                max_iterations = max_iterations,
                "Reached max iterations without achieving goal"
            );
            final_result = Value::String(format!(
                "Max iterations ({}) reached. Final state:\n{}",
                max_iterations,
                format_state(&current_state)
            ));
        }
    }

    // Record autonomous transition in AAM
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::AUTONOMOUS_NODE_PREFIX, node.id),
        final_result.clone(),
        TransitionLabel::operation(node.id, node.op_type),
    );

    tracing::info!(
        execution_id = %ctx.execution_id,
        node_id = node.id,
        iterations = iteration,
        "AUTONOMOUS operation completed"
    );

    Ok(final_result)
}

/// In-graph multi-turn conversation loop (the `converse` mode of AUTONOMOUS).
/// Each turn is an ASK with persona + accumulated transcript; tools run when
/// the node exposes a tool group. Returns the full transcript.
async fn converse_loop(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let persona = get_optional_string_attribute(node, graph_attrs::SYSTEM_PROMPT)?
        .or(get_optional_string_attribute(node, graph_attrs::PROMPT)?)
        .unwrap_or_else(|| "You are a helpful assistant.".to_string());

    // Turns: JSON array of user messages in input 0.
    let turns: Vec<String> = inputs
        .first()
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();

    let max_turns = get_optional_u64_attribute(node, graph_attrs::MAX_ITERATIONS)?.unwrap_or(100);
    let tools = resolve_node_tools(ctx, node);

    tracing::info!(
        execution_id = %ctx.execution_id,
        node_id = node.id,
        turns = turns.len(),
        "Starting CONVERSE in-graph conversation loop"
    );

    let mut transcript = String::new();
    let mut last_reply = Value::String(String::new());

    for (i, turn) in turns.iter().enumerate() {
        if i as u64 >= max_turns {
            break;
        }
        transcript.push_str("User: ");
        transcript.push_str(turn);
        transcript.push('\n');

        let prompt = format!("{persona}\n\n{transcript}Assistant:");
        let reply = run_agent_turn(ctx, node, &tools, prompt, "converse_turn").await?;

        transcript.push_str("Assistant: ");
        transcript.push_str(&reply);
        transcript.push('\n');
        last_reply = Value::String(reply);
    }

    let _ = &last_reply;
    Ok(Value::String(transcript))
}

/// Run one agent turn for `prompt`: a tool-using turn when the node exposes tools
/// and the backend supports auto tool choice, otherwise a plain text turn.
/// Shared by `converse_loop` and `recv_loop` (mirrors the ASK path).
async fn run_agent_turn(
    ctx: &ExecutionContext,
    node: &Node,
    tools: &[ToolDefinition],
    prompt: String,
    label: &str,
) -> Result<String> {
    let mut req = apply_llm_request_routing_from_node(LLMRequest::new(prompt), node)?;
    if tools.is_empty() {
        return Ok(execute_llm_request_for_node(ctx, node, label, &req)
            .await
            .map_err(|e| RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("{label} failed: {e}"),
            })?
            .content);
    }
    let supports = ctx
        .llm_registry
        .resolve_backend_name(&req)
        .ok()
        .and_then(|name| ctx.llm_registry.get_backend(&name))
        .is_some_and(|b| b.supports_auto_tool_choice());
    if supports {
        req = attach_graph_hints(ctx, node, req);
        req = req.with_tools(tools.to_vec()).with_tool_choice(ToolChoice::Auto);
        let out = run_tool_loop(ctx, node, &req)
            .await
            .map_err(|e| RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("{label} tool turn failed: {e}"),
            })?;
        Ok(match out {
            Value::String(s) => s,
            other => format_state(&other),
        })
    } else {
        Ok(execute_llm_request_for_node(ctx, node, label, &req)
            .await
            .map_err(|e| RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("{label} failed: {e}"),
            })?
            .content)
    }
}

/// RECV mode: an artifact-resident event wait. The node parks polling `recv_url`
/// for the next event; for each event it runs one agent turn (persona + event)
/// and accumulates a transcript. `once` (default) returns after the first event;
/// otherwise it re-arms up to `max_iterations` events. This is the in-graph
/// counterpart to the host-loop monitor: the wait lives in the `.air` artifact.
///
/// Attributes:
/// - `recv_url`        (required): HTTP endpoint polled for the next event. A 2xx
///   with a non-empty body is an event; any other response / empty body = nothing
///   pending (poll again after `poll_interval_ms`).
/// - `recv_once`       ("false" to re-arm; default once)
/// - `max_iterations`  (re-arm event budget; default 100)
/// - `poll_interval_ms`(idle poll cadence; default 2000)
/// - `recv_max_polls`  (optional bound on consecutive idle polls; default = wait
///   indefinitely — true parking)
/// - input 0           (optional): a seed event handled before the first poll.
async fn recv_loop(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let persona = get_optional_string_attribute(node, graph_attrs::SYSTEM_PROMPT)?
        .or(get_optional_string_attribute(node, graph_attrs::PROMPT)?)
        .unwrap_or_else(|| "You are an agent reacting to external events.".to_string());

    let recv_url =
        get_optional_string_attribute(node, "recv_url")?.ok_or_else(|| RuntimeError::Operation {
            op_type: node.op_type,
            message: "recv mode requires a `recv_url` attribute (event source endpoint)"
                .to_string(),
        })?;

    let once = get_optional_string_attribute(node, "recv_once")?.as_deref() != Some("false");
    let max_events = if once {
        1
    } else {
        get_optional_u64_attribute(node, graph_attrs::MAX_ITERATIONS)?.unwrap_or(100)
    };
    let poll_interval_ms = get_optional_u64_attribute(node, graph_attrs::POLL_INTERVAL_MS)?
        .unwrap_or(DEFAULT_POLL_INTERVAL_MS);
    let max_empty_polls = get_optional_u64_attribute(node, "recv_max_polls")?;

    // Optional seed event handed in by the graph (no network round-trip).
    let seed = inputs
        .first()
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty());

    let tools = resolve_node_tools(ctx, node);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("recv: HTTP client build failed: {e}"),
        })?;

    tracing::info!(
        execution_id = %ctx.execution_id,
        node_id = node.id,
        recv_url = %recv_url,
        once,
        "Starting RECV in-graph event wait"
    );

    let mut transcript = String::new();
    let mut events: u64 = 0;
    let mut empty: u64 = 0;

    while events < max_events {
        let event_json = if events == 0 && seed.is_some() {
            seed.clone()
        } else {
            poll_once(&client, &recv_url).await
        };

        match event_json {
            Some(ev) => {
                empty = 0;
                events += 1;
                transcript.push_str("Event: ");
                transcript.push_str(&ev);
                transcript.push('\n');
                let prompt = format!("{persona}\n\nReact to this event:\n{ev}\n\nAssistant:");
                let reply = run_agent_turn(ctx, node, &tools, prompt, "recv_turn").await?;
                transcript.push_str("Assistant: ");
                transcript.push_str(&reply);
                transcript.push('\n');
            }
            None => {
                empty += 1;
                if max_empty_polls.is_some_and(|mp| empty >= mp) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(poll_interval_ms)).await;
            }
        }
    }

    Ok(Value::String(transcript))
}

/// Poll the event endpoint once. A 2xx with a non-empty body is an event;
/// anything else (non-2xx, empty body, transient error) means "nothing pending".
async fn poll_once(client: &reqwest::Client, url: &str) -> Option<String> {
    match client.get(url).send().await {
        Ok(r) if r.status().is_success() => {
            let body = r.text().await.unwrap_or_default();
            if body.trim().is_empty() {
                None
            } else {
                Some(body)
            }
        }
        _ => None,
    }
}

fn format_state(state: &Value) -> String {
    match state {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::constants::graph::attrs as graph_attrs;
    use apxm_core::types::execution::NodeMetadata;
    use apxm_core::types::operations::AISOperationType;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_autonomous_node(
        id: u64,
        goal: &str,
        max_iterations: Option<u64>,
    ) -> apxm_core::types::execution::Node {
        let mut attributes = HashMap::new();
        attributes.insert(
            graph_attrs::PROMPT.to_string(),
            Value::String(goal.to_string()),
        );
        if let Some(max_iter) = max_iterations {
            attributes.insert(
                graph_attrs::MAX_ITERATIONS.to_string(),
                Value::Number(apxm_core::types::values::Number::Integer(max_iter as i64)),
            );
        }

        apxm_core::types::execution::Node {
            id,
            op_type: AISOperationType::Autonomous,
            attributes,
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        }
    }

    #[tokio::test]
    async fn test_autonomous_missing_goal() {
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

        // Node without goal attribute
        let node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Autonomous,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("prompt"));
    }

    #[tokio::test]
    async fn test_converse_mode_empty_turns_returns_empty_transcript() {
        let ctx = ExecutionContext::new(
            Arc::new(MemorySystem::new(MemoryConfig::in_memory_ltm()).await.unwrap()),
            Arc::new(LLMRegistry::new()),
            Arc::new(CapabilitySystem::new()),
            crate::aam::Aam::new(),
        );
        // converse=true gates the in-graph turn loop; no PROMPT goal required.
        let mut attributes = HashMap::new();
        attributes.insert("converse".to_string(), Value::String("true".to_string()));
        attributes.insert(
            graph_attrs::SYSTEM_PROMPT.to_string(),
            Value::String("You are terse.".to_string()),
        );
        let node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Autonomous,
            attributes,
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        // Empty turn list => zero iterations, no LLM call, empty transcript.
        let result = execute(&ctx, &node, vec![Value::String("[]".to_string())])
            .await
            .expect("converse with empty turns should succeed without an LLM");
        assert_eq!(result, Value::String(String::new()));
    }

    #[tokio::test]
    async fn test_autonomous_default_max_iterations() {
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

        // Node without max_iterations should use default
        let node = make_autonomous_node(1, "Test goal", None);

        // Will fail due to no LLM but validates the attribute parsing works
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_autonomous_custom_max_iterations() {
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

        let node = make_autonomous_node(1, "Test goal", Some(5));

        // Will fail due to no LLM but validates custom max_iterations parsing
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_autonomous_format_state() {
        let string_state = Value::String("test state".to_string());
        assert_eq!(format_state(&string_state), "test state");

        let number_state = Value::Number(apxm_core::types::values::Number::Integer(42));
        assert_eq!(format_state(&number_state), "42");
    }

    fn num(n: i64) -> Value {
        Value::Number(apxm_core::types::values::Number::Integer(n))
    }

    fn recv_node(attrs: &[(&str, Value)]) -> apxm_core::types::execution::Node {
        let mut attributes = HashMap::new();
        attributes.insert("mode".to_string(), Value::String("recv".to_string()));
        for (k, v) in attrs {
            attributes.insert((*k).to_string(), v.clone());
        }
        apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Autonomous,
            attributes,
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        }
    }

    async fn test_ctx() -> ExecutionContext {
        ExecutionContext::new(
            Arc::new(MemorySystem::new(MemoryConfig::in_memory_ltm()).await.unwrap()),
            Arc::new(LLMRegistry::new()),
            Arc::new(CapabilitySystem::new()),
            crate::aam::Aam::new(),
        )
    }

    #[tokio::test]
    async fn recv_mode_requires_recv_url() {
        // mode=recv routes to recv_loop; without recv_url it errors clearly.
        let ctx = test_ctx().await;
        let node = recv_node(&[]);
        let err = execute(&ctx, &node, vec![]).await.unwrap_err().to_string();
        assert!(err.contains("recv_url"), "expected recv_url error, got: {err}");
    }

    #[tokio::test]
    async fn recv_mode_idle_polls_then_returns_empty_when_bounded() {
        // Re-arm mode against an unreachable endpoint: every poll is "nothing
        // pending", so after `recv_max_polls` idle polls it returns an empty
        // transcript without ever calling an LLM. Proves the park/poll loop and
        // its safety bound without a backend or event server.
        let ctx = test_ctx().await;
        let node = recv_node(&[
            ("recv_url", Value::String("http://127.0.0.1:1/events".to_string())),
            ("recv_once", Value::String("false".to_string())),
            ("recv_max_polls", num(2)),
            ("poll_interval_ms", num(10)),
        ]);
        let result = execute(&ctx, &node, vec![]).await.expect("recv returns Ok on idle");
        assert_eq!(result, Value::String(String::new()));
    }
}
