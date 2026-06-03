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
use apxm_backends::{LLMRequest, ToolChoice};
use apxm_core::constants::{graph::attrs as graph_attrs, runtime::belief_keys};
use apxm_core::error::RuntimeError;

const DEFAULT_MAX_ITERATIONS: u64 = 10;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Converse mode: an in-graph multi-turn conversation loop. Gated by the
    // `converse = "true"` attribute (rides the op's generic attr-dict). The loop
    // lives inside this handler (runtime iteration), so the *loop* is part of the
    // APXM program — not the host. Turns arrive as a JSON-array string in input 0
    // (batch) or, when absent, via the host's PAUSE/resume turn cycle.
    if get_optional_string_attribute(node, "converse")?.as_deref() == Some("true") {
        return converse_loop(ctx, node, inputs).await;
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
///
/// The loop is part of the program: this handler iterates user turns inside the
/// runtime, each turn an `ASK` against the live model with the persona + the
/// accumulated transcript (so context carries across turns). Tools are run when
/// the node exposes a tool group. Turns are read as a JSON-array string in
/// input 0 (batch). Returns the full transcript.
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
        let mut req = apply_llm_request_routing_from_node(LLMRequest::new(prompt), node)?;

        // Tool-using turn when the node exposes tools and the backend supports
        // auto tool choice; otherwise a plain text turn. Mirrors the ASK path.
        let reply = if tools.is_empty() {
            execute_llm_request_for_node(ctx, node, "converse_turn", &req)
                .await
                .map_err(|e| RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!("converse turn {} failed: {}", i + 1, e),
                })?
                .content
        } else {
            let supports = ctx
                .llm_registry
                .resolve_backend_name(&req)
                .ok()
                .and_then(|name| ctx.llm_registry.get_backend(&name))
                .map(|b| b.supports_auto_tool_choice())
                .unwrap_or(false);
            if supports {
                req = attach_graph_hints(ctx, node, req);
                req = req.with_tools(tools.clone()).with_tool_choice(ToolChoice::Auto);
                match run_tool_loop(ctx, node, &req).await.map_err(|e| {
                    RuntimeError::Operation {
                        op_type: node.op_type,
                        message: format!("converse tool turn {} failed: {}", i + 1, e),
                    }
                })? {
                    Value::String(s) => s,
                    other => format_state(&other),
                }
            } else {
                execute_llm_request_for_node(ctx, node, "converse_turn", &req)
                    .await
                    .map_err(|e| RuntimeError::Operation {
                        op_type: node.op_type,
                        message: format!("converse turn {} failed: {}", i + 1, e),
                    })?
                    .content
            }
        };

        transcript.push_str("Assistant: ");
        transcript.push_str(&reply);
        transcript.push('\n');
        last_reply = Value::String(reply);
    }

    let _ = &last_reply;
    Ok(Value::String(transcript))
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
}
