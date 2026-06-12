//! JSON-schema response handling and Reason-mode structured output processing.

use super::ExecutionContext;
use crate::aam::{Goal as AamGoal, GoalId, GoalStatus, TransitionLabel};
use apxm_core::InnerPlanPayload;
use apxm_core::apxm_llm;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::error::RuntimeError;
use apxm_core::types::execution::Node;
use apxm_core::types::values::Value;
use jsonschema::JSONSchema;
use serde::de::Error;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

use super::super::Result;
use super::super::inner_plan::{InnerPlanOptions, execute_inner_plan};

/// Structured output from Reason operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredReasonOutput {
    /// Belief updates to apply to memory
    #[serde(default)]
    pub belief_updates: HashMap<String, Value>,

    /// New goals to add
    #[serde(default)]
    pub new_goals: Vec<LlmGoalOutput>,

    /// Optional inner plan emitted by the model
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inner_plan: Option<InnerPlanPayload>,

    /// The main result value
    pub result: Value,
}

/// Goal definition for LLM structured output deserialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmGoalOutput {
    pub description: String,
    #[serde(default = "default_priority")]
    pub priority: u32,
}

fn default_priority() -> u32 {
    50
}

pub(super) fn output_schema_from_node(node: &Node) -> Result<Option<JsonValue>> {
    let Some(schema_val) = node.attributes.get(graph_attrs::OUTPUT_SCHEMA) else {
        return Ok(None);
    };
    let schema = schema_val.to_json().map_err(|e| RuntimeError::LLM {
        message: format!("invalid output_schema attribute: {e}"),
        backend: None,
    })?;
    Ok(Some(schema))
}

pub(super) fn validate_against_output_schema(content: &str, schema: &JsonValue) -> Result<()> {
    let instance = parse_json_from_text(content).ok_or_else(|| RuntimeError::LLM {
        message: "response is not valid JSON; cannot validate output_schema".to_string(),
        backend: None,
    })?;
    let compiled = JSONSchema::compile(schema).map_err(|e| RuntimeError::LLM {
        message: format!("invalid configured output_schema: {e}"),
        backend: None,
    })?;
    if let Err(errors) = compiled.validate(&instance) {
        let message = errors
            .map(|err| err.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(RuntimeError::LLM {
            message: format!("schema validation failed: {message}"),
            backend: None,
        });
    }
    Ok(())
}

pub(super) fn parse_json_from_text(text: &str) -> Option<JsonValue> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<JsonValue>(trimmed) {
        return Some(value);
    }
    if let Some(fenced) = extract_fenced_json(trimmed)
        && let Ok(value) = serde_json::from_str::<JsonValue>(fenced)
    {
        return Some(value);
    }
    extract_balanced_json(trimmed).and_then(|candidate| serde_json::from_str(candidate).ok())
}

pub(super) fn extract_fenced_json(text: &str) -> Option<&str> {
    let start = text.find("```")?;
    let rest = &text[start + 3..];
    let rest = if let Some(after_lang) = rest.strip_prefix("json") {
        after_lang
    } else {
        rest
    };
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let end = rest.find("```")?;
    Some(rest[..end].trim())
}

fn extract_balanced_json(text: &str) -> Option<&str> {
    for (open, close) in [('{', '}'), ('[', ']')] {
        if let (Some(start), Some(end)) = (text.find(open), text.rfind(close))
            && end > start
        {
            return Some(text[start..=end].trim());
        }
    }
    None
}

pub(super) fn build_schema_retry_prompt(
    original_prompt: &str,
    previous_output: &str,
    schema: &JsonValue,
    validation_error: &str,
) -> String {
    format!(
        "{original_prompt}\n\n\
         Your previous response did not satisfy the required JSON schema.\n\
         Validation error: {validation_error}\n\n\
         Required JSON schema:\n{schema}\n\n\
         Previous invalid response:\n{previous_output}\n\n\
         Return ONLY valid JSON that satisfies the schema."
    )
}

pub(super) fn parse_structured_output(
    content: &str,
) -> std::result::Result<StructuredReasonOutput, serde_json::Error> {
    let json_value = parse_json_from_text(content)
        .ok_or_else(|| serde_json::Error::custom("Failed to parse structured output"))?;
    serde_json::from_value(json_value)
}

/// Process structured output from Reason mode
pub(super) async fn process_structured_output(
    ctx: &ExecutionContext,
    node: &Node,
    structured: StructuredReasonOutput,
    enable_inner_plan: bool,
    bind_outputs: bool,
) -> Result<Value> {
    let label = TransitionLabel::operation(node.id, node.op_type);

    // Apply belief updates to LTM
    for (key, value) in structured.belief_updates {
        ctx.memory
            .write_scoped(
                crate::memory::MemorySpace::Ltm,
                ctx.scope_id(),
                key.clone(),
                value.clone(),
            )
            .await
            .ok();

        ctx.aam.set_belief(key, value, label.clone());
    }

    if !structured.new_goals.is_empty() {
        let goals_value = Value::Array(
            structured
                .new_goals
                .iter()
                .map(|g| {
                    let mut obj = HashMap::new();
                    obj.insert(
                        "description".to_string(),
                        Value::String(g.description.clone()),
                    );
                    obj.insert(
                        "priority".to_string(),
                        Value::Number(apxm_core::types::values::Number::Integer(g.priority as i64)),
                    );
                    Value::Object(obj)
                })
                .collect(),
        );

        ctx.memory
            .write_scoped(
                crate::memory::MemorySpace::Stm,
                ctx.scope_id(),
                format!("{}{}", belief_keys::GOALS_PREFIX, ctx.execution_id),
                goals_value,
            )
            .await
            .ok();

        for goal in &structured.new_goals {
            let aam_goal = AamGoal {
                id: GoalId::new(),
                description: goal.description.clone(),
                priority: goal.priority,
                status: GoalStatus::Active,
                parent_id: None,
            };
            ctx.aam.add_goal(aam_goal, label.clone());
        }
    }

    if enable_inner_plan {
        if let Some(inner_plan) = structured.inner_plan.clone() {
            apxm_llm!(info,
                execution_id = %ctx.execution_id,
                "REASON provided inner plan graph payload"
            );

            let inserted_nodes = execute_inner_plan(
                ctx,
                node,
                &inner_plan,
                InnerPlanOptions {
                    bind_outer_outputs: bind_outputs,
                },
            )
            .await?;

            if inserted_nodes > 0 {
                apxm_llm!(debug,
                    execution_id = %ctx.execution_id,
                    inserted_nodes = inserted_nodes,
                    "Inner plan merged into DAG"
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
                            "REASON inner plan merged into DAG with {} nodes",
                            inserted_nodes
                        )),
                    )
                    .await
                    .ok();
            }
        } else {
            apxm_llm!(trace,
                execution_id = %ctx.execution_id,
                "REASON inner plan enabled but model omitted graph payload"
            );
        }
    }

    Ok(structured.result)
}

