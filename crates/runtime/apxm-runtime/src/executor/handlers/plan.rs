//! PLAN operation - Planning with LLM and inner/outer plan execution
//!
//! Supports:
//! - LLM-based planning
//! - Inner plan (sub-DAG execution)
//! - Outer plan (continuation after inner plan)
//! - Structured plan output
//! - Goal-oriented planning

use super::{
    ExecutionContext, Node, Result, Value, apply_llm_request_routing_from_node,
    execute_llm_request_for_node, extract_json_from_markdown, get_optional_string_attribute,
    inner_plan::{InnerPlanOptions, execute_inner_plan},
    llm::attach_graph_hints,
};
use crate::aam::{Goal as AamGoal, GoalId, GoalStatus, TransitionLabel};
use apxm_backends::LLMRequest;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::{InnerPlanPayload, Plan, error::RuntimeError};
use serde::de::Error;
use std::collections::HashMap;

// Note: Plan, PlanStep, and InnerPlanPayload are now imported from apxm_core
// This ensures consistency across the entire system (chat, runtime, compiler)

/// Execute PLAN operation - LLM-based planning with inner/outer plan support
///
/// The PLAN operation supports:
/// - **Simple planning**: Returns a list of steps
/// - **Inner plan execution**: Executes a sub-DAG and continues with outer plan
/// - **Goal-based planning**: Generates plans to achieve specific goals
///
/// # Inner/Outer Plan Flow
///
/// 1. **Generate outer plan** (high-level strategy)
/// 2. **If inner_plan exists**: Execute the inner DAG
/// 3. **Return to outer plan**: Continue with remaining steps
///
/// # Structured Output Format
///
/// ```json
/// {
///   "plan": [
///     {"description": "Step 1", "priority": 90},
///     {"description": "Step 2", "priority": 80, "dependencies": ["Step 1"]}
///   ],
///   "result": "Plan summary"
/// }
/// ```
pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Get goal from attribute or first input (for dynamic goals from variables)
    let goal_attr = get_optional_string_attribute(node, graph_attrs::GOAL)?.unwrap_or_default();
    let goal = if goal_attr.is_empty() && !inputs.is_empty() {
        // Use first input as goal if attribute is empty
        inputs[0].to_string()
    } else if goal_attr.is_empty() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: "PLAN requires either a goal attribute or input".to_string(),
        });
    } else {
        goal_attr
    };
    let model = get_optional_string_attribute(node, graph_attrs::MODEL)?;
    let context_key = get_optional_string_attribute(node, graph_attrs::CONTEXT_KEY)?;
    let supports_inner_plan = node
        .attributes
        .get(graph_attrs::INNER_PLAN_SUPPORTED)
        .and_then(|v| v.as_boolean())
        .unwrap_or(false);
    let enable_inner_plan = node
        .attributes
        .get(graph_attrs::ENABLE_INNER_PLAN)
        .and_then(|v| v.as_boolean())
        .unwrap_or(supports_inner_plan);
    let bind_outputs = node
        .attributes
        .get(graph_attrs::BIND_INNER_PLAN_OUTPUTS)
        .and_then(|v| v.as_boolean())
        .unwrap_or(true);

    // Retrieve context from memory if specified
    let mut context_info = String::new();
    if let Some(key) = context_key
        && let Ok(Some(value)) = ctx
            .memory
            .read_scoped(crate::memory::MemorySpace::Ltm, ctx.scope_id(), &key)
            .await
    {
        context_info = format!("\n\nContext: {:?}", value);
    }

    // Build planning prompt using apxm-prompts
    // The plan_system template expects a "text" field containing the full user request
    let user_text = if context_info.is_empty() {
        format!("Goal: {}", goal)
    } else {
        format!("Goal: {}\n\nContext: {}", goal, context_info)
    };

    let prompt_context = serde_json::json!({
        "text": user_text
    });

    let user_prompt = apxm_backends::render_prompt("plan_system", &prompt_context)
        .unwrap_or_else(|_| {
            format!(
                "Create a detailed plan to achieve the following:\n\n{}\n\n\
                 Respond in JSON format with: plan (array of steps with description and priority) and result (summary).",
                user_text
            )
        });

    // Load system prompt from config, template, or fallback
    // Priority: 1) node attribute (agent context), 2) config instruction, 3) template, 4) hardcoded fallback
    let system_prompt = get_optional_string_attribute(node, graph_attrs::SYSTEM_PROMPT)?
        .or_else(|| ctx.instruction_config.plan.clone())
        .or_else(|| {
            apxm_backends::render_prompt("plan_outer_system", &serde_json::json!({})).ok()
        })
        .unwrap_or_else(|| {
            "You are an expert planning assistant. Generate structured, actionable plans. \
             Always respond in valid JSON format with 'plan' (array of steps) and 'result' (summary)."
                .to_string()
        });

    // Execute with retries and progressive temperature increase
    // Temperatures: 0.7 → 0.8 → 0.9 → 1.0 (increase variability on retries)
    let mut last_error = None;
    for attempt in 0..=3 {
        // Check cancellation before each planning attempt
        if ctx.cancellation_token.is_cancelled() {
            return Err(RuntimeError::SchedulerCancelled);
        }

        // Build request with progressive temperature
        let temperature = 0.7 + (attempt as f64 * 0.1);
        let request = apply_llm_request_routing_from_node(
            LLMRequest::new(user_prompt.clone())
                .with_system_prompt(system_prompt.to_string())
                .with_temperature(temperature),
            node,
        )?;
        let request = attach_graph_hints(ctx, node, request);

        if attempt > 0 {
            tracing::info!(
                execution_id = %ctx.execution_id,
                attempt = attempt,
                temperature = temperature,
                "Retrying PLAN with increased temperature"
            );

            let backoff_ms = 100 * 2_u64.pow(attempt - 1);
            tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
        }

        match execute_plan_once(
            ctx,
            node,
            &request,
            enable_inner_plan,
            bind_outputs,
            &goal,
            model.as_deref(),
        )
        .await
        {
            Ok(value) => return Ok(value),
            Err(e) => {
                last_error = Some(e);
                tracing::debug!(
                    execution_id = %ctx.execution_id,
                    attempt = attempt,
                    temperature = temperature,
                    error = %last_error.as_ref().unwrap(),
                    "PLAN attempt failed"
                );
            }
        }
    }

    Err(last_error.unwrap_or_else(|| RuntimeError::LLM {
        message: "All retry attempts exhausted for PLAN".to_string(),
        backend: None,
    }))
}

/// Execute a single PLAN attempt
async fn execute_plan_once(
    ctx: &ExecutionContext,
    node: &Node,
    request: &LLMRequest,
    enable_inner_plan: bool,
    bind_outputs: bool,
    original_goal: &str,
    model_override: Option<&str>,
) -> Result<Value> {
    let transition_label = TransitionLabel::operation(node.id, node.op_type);
    // Execute LLM request
    let response = execute_llm_request_for_node(ctx, node, "PLAN", request).await?;

    let content = response.content;
    tracing::info!(
        execution_id = %ctx.execution_id,
        raw_response = %content,
        "PLAN model response"
    );

    // Try to parse as structured plan
    if let Ok(mut plan) = parse_plan_output(&content) {
        // Emit plan-created event
        if let Some(emitter) = &ctx.event_emitter {
            emitter.emit_plan_created(&ctx.execution_id, plan.steps.len());
        }
        if enable_inner_plan && !plan.has_inner_plan() {
            match generate_inner_plan(ctx, node, &plan, original_goal, model_override).await {
                Ok(Some(air_payload)) => {
                    plan.inner_plan = Some(InnerPlanPayload {
                        air: Some(air_payload),
                        task_dag: None,
                    });
                }
                Ok(None) => {
                    tracing::warn!(
                        execution_id = %ctx.execution_id,
                        "Inner plan was requested but the model did not provide AIR payload"
                    );
                }
                Err(err) => {
                    tracing::error!(
                        execution_id = %ctx.execution_id,
                        error = %err,
                        "Inner plan generation failed"
                    );
                }
            }
        } else if enable_inner_plan && plan.has_inner_plan() {
            tracing::info!(
                execution_id = %ctx.execution_id,
                "PLAN response already contains inner plan payload"
            );
        }

        // Plans-as-graphs: validate the LLM-emitted task DAG up-front so
        // a malformed graph (cycle, dangling depends_on, dup ids) fails
        // with an actionable error tied to the PLAN node, not buried
        // inside link_task_dag → compile. validate() is also called by
        // task_dag_to_air_module, so this is defence-in-depth + better
        // UX, not a correctness change. Telemetry follows: emit the
        // PlanGraphEmitted event with the extracted parallel fan-out so
        // trace consumers can tell "LLM produced an executable graph"
        // from "LLM produced free-text steps."
        if let Some(task_dag) = plan.inner_plan.as_ref().and_then(|ip| ip.task_dag.as_ref()) {
            if let Err(err) = task_dag.validate() {
                tracing::error!(
                    execution_id = %ctx.execution_id,
                    error = %err,
                    "LLM-emitted task_dag is malformed; rejecting before link"
                );
                return Err(RuntimeError::State(format!(
                    "PLAN at node {} produced a malformed task_dag: {}. \
                     Drop the inner_plan or fix the dependency structure.",
                    node.id, err
                )));
            }
            if let Some(emitter) = &ctx.event_emitter {
                let mut deps_buckets: HashMap<Vec<u64>, usize> = HashMap::new();
                for task in &task_dag.tasks {
                    let mut deps = task.depends_on.clone();
                    deps.sort_unstable();
                    *deps_buckets.entry(deps).or_insert(0) += 1;
                }
                let parallel_fanout_max = deps_buckets.values().copied().max().unwrap_or(0);
                let task_ids: Vec<u64> = task_dag.tasks.iter().map(|t| t.id).collect();
                let generating_model = if response.model.is_empty() {
                    "<unknown>"
                } else {
                    response.model.as_str()
                };
                emitter.emit_plan_graph_emitted(
                    &ctx.execution_id,
                    generating_model,
                    task_dag.tasks.len(),
                    &task_ids,
                    parallel_fanout_max,
                );
            }
        }

        // Store plan in memory
        let plan_json = serde_json::to_value(&plan.steps)
            .map_err(|e| RuntimeError::Serialization(format!("Failed to serialize plan: {}", e)))?;
        let plan_value: Value = plan_json
            .try_into()
            .unwrap_or(Value::String("<invalid plan>".into()));

        ctx.memory
            .write_scoped(
                crate::memory::MemorySpace::Stm,
                ctx.scope_id(),
                format!("{}{}", belief_keys::PLAN_PREFIX, ctx.execution_id),
                plan_value.clone(),
            )
            .await
            .ok();

        ctx.aam.set_belief(
            format!("{}{}", belief_keys::GOAL_PREFIX, ctx.execution_id),
            Value::String(original_goal.to_string()),
            transition_label.clone(),
        );

        ctx.aam.set_belief(
            format!("{}{}", belief_keys::PLAN_PREFIX, ctx.execution_id),
            plan_value.clone(),
            transition_label.clone(),
        );

        let plan_goal = AamGoal {
            id: GoalId::new(),
            description: plan.result.clone(),
            priority: plan
                .steps
                .iter()
                .map(|step| step.priority)
                .max()
                .unwrap_or(50),
            status: GoalStatus::Active,
            parent_id: None,
        };
        ctx.aam.add_goal(plan_goal, transition_label.clone());

        // If inner plan exists and enabled, execute it
        if enable_inner_plan && let Some(inner_plan) = plan.inner_plan.clone() {
            tracing::info!(
                execution_id = %ctx.execution_id,
                "Linking inner plan DAG"
            );

            match execute_inner_plan(
                ctx,
                node,
                &inner_plan,
                InnerPlanOptions {
                    bind_outer_outputs: bind_outputs,
                },
            )
            .await
            {
                Ok(inserted_nodes) => {
                    tracing::info!(
                        execution_id = %ctx.execution_id,
                        nodes_inserted = inserted_nodes,
                        "Inner plan successfully linked and spliced into DAG"
                    );

                    ctx.memory
                        .write(
                            crate::memory::MemorySpace::Episodic,
                            format!(
                                "{}{}",
                                belief_keys::INNER_PLAN_SPLICED_PREFIX,
                                ctx.execution_id
                            ),
                            Value::String(format!(
                                "Inner plan merged into DAG with {} nodes",
                                inserted_nodes
                            )),
                        )
                        .await
                        .ok();
                }
                Err(err) => {
                    let err_msg = err.to_string();
                    if err_msg.contains("not supported") || err_msg.contains("No linker configured")
                    {
                        tracing::warn!(
                            execution_id = %ctx.execution_id,
                            "Inner plan linking not supported - continuing without inner plan execution. \
                             To enable, attach an InnerPlanLinker implementation to runtime."
                        );
                        // Continue execution without inner plan - not a fatal error
                    } else {
                        // Other errors are fatal
                        tracing::error!(
                            execution_id = %ctx.execution_id,
                            error = %err,
                            "Inner plan execution failed"
                        );
                        return Err(err);
                    }
                }
            }
        }

        // Return plan as structured value
        let mut result_obj = HashMap::new();
        result_obj.insert(
            "plan".to_string(),
            Value::Array(
                plan.steps
                    .iter()
                    .map(|step| {
                        let mut step_obj = HashMap::new();
                        step_obj.insert(
                            "description".to_string(),
                            Value::String(step.description.clone()),
                        );
                        step_obj.insert(
                            "priority".to_string(),
                            Value::Number(apxm_core::types::values::Number::Integer(
                                step.priority as i64,
                            )),
                        );
                        if !step.dependencies.is_empty() {
                            step_obj.insert(
                                "dependencies".to_string(),
                                Value::Array(
                                    step.dependencies
                                        .iter()
                                        .map(|d| Value::String(d.clone()))
                                        .collect(),
                                ),
                            );
                        }
                        Value::Object(step_obj)
                    })
                    .collect(),
            ),
        );
        result_obj.insert("result".to_string(), Value::String(plan.result));

        Ok(Value::Object(result_obj))
    } else {
        // Fall back to plain text response
        Ok(Value::String(content))
    }
}

/// Parse plan from LLM response
fn parse_plan_output(content: &str) -> std::result::Result<Plan, serde_json::Error> {
    let trimmed = content.trim();

    // Try direct parse
    if let Ok(output) = serde_json::from_str::<Plan>(trimmed) {
        return Ok(output);
    }

    // Try to extract JSON from markdown
    if let Some(json_str) = extract_json_from_markdown(trimmed)
        && let Ok(output) = serde_json::from_str::<Plan>(&json_str)
    {
        return Ok(output);
    }

    Err(serde_json::Error::custom("Failed to parse plan output"))
}

async fn generate_inner_plan(
    ctx: &ExecutionContext,
    node: &Node,
    plan: &Plan,
    goal: &str,
    model_override: Option<&str>,
) -> Result<Option<String>> {
    let prompt_context = serde_json::json!({
        "goal": goal,
        "plan": plan.steps,
        "result": plan.result,
    });

    let user_prompt = apxm_backends::render_prompt("inner_plan", &prompt_context).map_err(|e| {
        RuntimeError::LLM {
            message: format!("Failed to render inner_plan prompt: {e}"),
            backend: None,
        }
    })?;

    // Load system prompt for graph-JSON generation from template.
    // No fallback prompt is used; template load failures are explicit.
    let system_prompt = apxm_backends::render_prompt("plan_inner_system", &serde_json::json!({}))
        .map_err(|e| RuntimeError::LLM {
        message: format!("Failed to load plan_inner_system template: {e}"),
        backend: None,
    })?;

    let mut request = apply_llm_request_routing_from_node(
        LLMRequest::new(user_prompt).with_system_prompt(system_prompt),
        node,
    )?;

    if let Some(model_name) = model_override {
        request = request.with_model(model_name.to_string());
    }
    request = attach_graph_hints(ctx, node, request);

    let response = execute_llm_request_for_node(ctx, node, "INNER_PLAN", &request).await?;

    let content = response.content;
    tracing::info!(
        execution_id = %ctx.execution_id,
        raw_response = %content,
        "INNER PLAN model response"
    );

    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    Ok(Some(trimmed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plan_output() {
        let json = r#"{
            "plan": [
                {"description": "Step 1", "priority": 90},
                {"description": "Step 2", "priority": 80, "dependencies": ["Step 1"]}
            ],
            "result": "Plan created successfully"
        }"#;

        let output = parse_plan_output(json).unwrap();
        assert_eq!(output.steps.len(), 2);
        assert_eq!(output.steps[0].description, "Step 1");
        assert_eq!(output.steps[1].priority, 80);
        assert_eq!(output.result, "Plan created successfully");
    }

    #[test]
    fn test_parse_markdown_wrapped_plan() {
        let content = r#"Here is the plan:

```json
{
    "plan": [
        {"description": "Analyze", "priority": 100}
    ],
    "result": "Analysis plan"
}
```

Done."#;

        let output = parse_plan_output(content).unwrap();
        assert_eq!(output.steps.len(), 1);
        assert_eq!(output.steps[0].description, "Analyze");
    }

    #[test]
    fn test_parse_plan_output_with_inner_task_dag() {
        let json = r#"{
            "plan": [],
            "result": "ok",
            "inner_plan": {
                "task_dag": {
                    "name": "inner",
                    "tasks": [],
                    "metadata": {}
                }
            }
        }"#;

        let output = parse_plan_output(json).unwrap();
        assert!(output.has_inner_plan());
        assert!(output.inner_plan.and_then(|p| p.task_dag).is_some());
    }

    /// Parses the fan-out worked example from the PLAN system prompt
    /// (`plan_outer_system.md.jinja`) and verifies it produces the
    /// expected DAG shape: 5 tasks, fan-out=3 after task 1, join at
    /// task 5. If the prompt's example schema drifts from the Plan
    /// struct's actual deserialization, this test fails and surfaces
    /// the contract mismatch before runtime.
    #[test]
    fn test_parse_plan_output_matches_prompt_fanout_example() {
        let json = r#"{
            "plan": [
                {"description": "decompose into sub-questions", "priority": 90, "dependencies": []},
                {"description": "research each sub-question", "priority": 80, "dependencies": ["decompose into sub-questions"]},
                {"description": "synthesize findings", "priority": 70, "dependencies": ["research each sub-question"]}
            ],
            "result": "structured research brief on the requested topic",
            "inner_plan": {
                "task_dag": {
                    "name": "research",
                    "tasks": [
                        {"id": 1, "name": "decompose", "description": "Decompose the topic into 3 sub-questions", "nodes": [], "depends_on": [], "metadata": {}},
                        {"id": 2, "name": "sq1", "description": "Research sub-question 1", "nodes": [], "depends_on": [1], "metadata": {}},
                        {"id": 3, "name": "sq2", "description": "Research sub-question 2", "nodes": [], "depends_on": [1], "metadata": {}},
                        {"id": 4, "name": "sq3", "description": "Research sub-question 3", "nodes": [], "depends_on": [1], "metadata": {}},
                        {"id": 5, "name": "synthesize", "description": "Synthesize findings from all sub-questions", "nodes": [], "depends_on": [2, 3, 4], "metadata": {}}
                    ],
                    "metadata": {}
                }
            }
        }"#;

        let output = parse_plan_output(json).unwrap();
        let task_dag = output
            .inner_plan
            .as_ref()
            .and_then(|ip| ip.task_dag.as_ref())
            .expect("prompt example must produce an inner task_dag");
        assert_eq!(task_dag.tasks.len(), 5);
        assert_eq!(task_dag.tasks[0].depends_on, Vec::<u64>::new());
        assert_eq!(task_dag.tasks[1].depends_on, vec![1]);
        assert_eq!(task_dag.tasks[4].depends_on, vec![2, 3, 4]);
    }

    /// LLM-emitted task DAGs that reference missing parent ids are
    /// rejected before any compile attempt. Mirrors the up-front
    /// validate() the PLAN handler runs so a malformed dangling-deps
    /// graph surfaces with a clear error.
    #[test]
    fn test_task_dag_validate_rejects_dangling_depends_on() {
        let json = r#"{
            "plan": [],
            "result": "ok",
            "inner_plan": {
                "task_dag": {
                    "name": "broken",
                    "tasks": [
                        {"id": 1, "name": "a", "description": "first", "nodes": [], "depends_on": [], "metadata": {}},
                        {"id": 2, "name": "b", "description": "second", "nodes": [], "depends_on": [999], "metadata": {}}
                    ],
                    "metadata": {}
                }
            }
        }"#;

        let output = parse_plan_output(json).unwrap();
        let task_dag = output
            .inner_plan
            .as_ref()
            .and_then(|ip| ip.task_dag.as_ref())
            .expect("task_dag must parse");
        let err = task_dag.validate().expect_err("dangling depends_on must reject");
        let msg = format!("{err}");
        assert!(
            msg.contains("999"),
            "error must cite the offending id, got: {msg}"
        );
    }

    /// Cycle detection is the third reject path; covers the Kahn's
    /// algorithm branch in TaskDag::validate. A 2-task cycle is the
    /// minimum interesting case.
    #[test]
    fn test_task_dag_validate_rejects_cycle() {
        let json = r#"{
            "plan": [],
            "result": "ok",
            "inner_plan": {
                "task_dag": {
                    "name": "circular",
                    "tasks": [
                        {"id": 1, "name": "a", "description": "first", "nodes": [], "depends_on": [2], "metadata": {}},
                        {"id": 2, "name": "b", "description": "second", "nodes": [], "depends_on": [1], "metadata": {}}
                    ],
                    "metadata": {}
                }
            }
        }"#;

        let output = parse_plan_output(json).unwrap();
        let task_dag = output
            .inner_plan
            .as_ref()
            .and_then(|ip| ip.task_dag.as_ref())
            .expect("task_dag must parse");
        let err = task_dag.validate().expect_err("cycle must reject");
        assert!(
            format!("{err}").to_lowercase().contains("cycle"),
            "error must mention cycle, got: {err}"
        );
    }

    /// Verifies the parallel-fanout calculation that
    /// PlanGraphEmittedPayload reports. Tasks sharing the same
    /// `depends_on` set fan out in parallel after that set completes;
    /// the max bucket count is the extracted parallelism width.
    #[test]
    fn test_parallel_fanout_max_groups_by_dependency_set() {
        use std::collections::HashMap;
        let depends_on_lists: Vec<Vec<u64>> = vec![
            vec![],        // task 1 (root)
            vec![1],       // task 2 — shares deps {1} with 3, 4
            vec![1],       // task 3
            vec![1],       // task 4
            vec![2, 3, 4], // task 5 — alone in its bucket
        ];

        let mut buckets: HashMap<Vec<u64>, usize> = HashMap::new();
        for mut deps in depends_on_lists {
            deps.sort_unstable();
            *buckets.entry(deps).or_insert(0) += 1;
        }
        let parallel_fanout_max = buckets.values().copied().max().unwrap_or(0);
        assert_eq!(parallel_fanout_max, 3, "tasks 2,3,4 share depends_on=[1]");
    }
}
