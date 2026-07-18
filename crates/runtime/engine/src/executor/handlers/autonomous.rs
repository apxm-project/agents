//! AUTONOMOUS operation - Autonomous execution with goal-directed loop
//!
//! Implements autonomous agent behavior via an iterative loop:
//! 1. Plans next action based on goal + current state
//! 2. Executes the action (via LLM call)
//! 3. Evaluates progress toward goal
//! 4. Continues until goal is met or max_iterations reached

use super::{
    ExecutionContext, Node, Result, Value, apply_llm_request_routing_from_node,
    execute_llm_request_for_node_with_context, get_input, get_optional_string_attribute,
    get_optional_u64_attribute,
    llm::{
        ContextualNodeRequest, attach_graph_hints, contextualize_node_request,
        execute_contextual_node_request, resolve_node_tools, run_tool_loop,
    },
};
use crate::aam::TransitionLabel;
use apxm_backends::{LLMRequest, ToolChoice};
use apxm_core::constants::{graph::attrs as graph_attrs, runtime::belief_keys};
use apxm_core::error::RuntimeError;

const DEFAULT_MAX_ITERATIONS: u64 = 10;
pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
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

        let plan_response =
            execute_contextual_node_request(ctx, node, "autonomous_plan", &plan_req)
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

        let action_req = apply_llm_request_routing_from_node(LLMRequest::new(action_prompt), node)?;
        let ContextualNodeRequest {
            request: mut action_req,
            metrics: action_context_metrics,
            plan: action_plan,
        } = contextualize_node_request(ctx, node, action_req)?;

        // If the autonomous node exposes tools (capability_groups / tools / tools_enabled),
        // run the real model->tool->model loop so the agent can ACT on its decision
        // (actually invoke capabilities), not just describe an action. Opt-in and
        // default-safe: no tool attrs => empty tools => the original text-only path.
        //
        // Caveat: the backend capability check goes through `ctx.llm_registry`,
        // which may differ from a node-routed ModelRouter backend; this uses
        // the same check as the ASK handler (llm/mod.rs).
        let tools = resolve_node_tools(ctx, node);
        let action_content = if tools.is_empty() {
            execute_llm_request_for_node_with_context(
                ctx,
                node,
                "autonomous_action",
                &action_req,
                &action_context_metrics,
            )
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
                action_req = action_req
                    .with_tools(tools)
                    .with_tool_choice(ToolChoice::Auto);
                let value = run_tool_loop(ctx, node, &action_req, action_plan.as_ref())
                    .await
                    .map_err(|e| RuntimeError::Operation {
                        op_type: node.op_type,
                        message: format!(
                            "Failed tool-loop action (iteration {}): {}",
                            iteration, e
                        ),
                    })?;
                match value {
                    Value::String(s) => s,
                    other => format_state(&other),
                }
            } else {
                // Backend can't accept tool_choice=auto; degrade to text-only.
                execute_llm_request_for_node_with_context(
                    ctx,
                    node,
                    "autonomous_action",
                    &action_req,
                    &action_context_metrics,
                )
                .await
                .map_err(|e| RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!("Failed to execute action (iteration {}): {}", iteration, e),
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

        let eval_response =
            execute_contextual_node_request(ctx, node, "autonomous_eval", &eval_req)
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

fn format_state(state: &Value) -> String {
    match state {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}
