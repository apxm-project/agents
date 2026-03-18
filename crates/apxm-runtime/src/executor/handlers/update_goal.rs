//! UPDATE_GOAL operation — modify AAM goals at runtime.
//!
//! Allows agents to upsert, remove, or clear goals from the Agent Abstract
//! Machine during execution. This closes a gap in the ISA: PLAN creates a
//! goal list, but there was previously no op to modify individual goals.
//!
//! ## Attributes
//! - `goal_id`   (required): string key identifying the goal
//! - `action`    (optional): "set" (default) | "remove" | "clear"
//! - `priority`  (optional): u32 priority for new goals (default: 1)
//!
//! ## AIS usage
//! ```ais
//! // Set a new goal
//! update_goal(goal_id: "research_topic", action: "set", priority: 2) <- "Investigate GPU architecture"
//!
//! // Remove a goal
//! update_goal(goal_id: "old_task", action: "remove")
//!
//! // Clear all goals
//! update_goal(goal_id: "", action: "clear")
//! ```

use super::{
    ExecutionContext, Node, Result, Value, get_optional_string_attribute,
    get_optional_u64_attribute, get_string_attribute,
};
use crate::aam::{Goal, GoalId, GoalStatus, TransitionLabel};
use apxm_core::constants::graph::attrs as graph_attrs;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let action = get_optional_string_attribute(node, graph_attrs::ACTION)?
        .unwrap_or_else(|| "set".to_string());

    tracing::info!(
        execution_id = %ctx.execution_id,
        action = %action,
        "Executing UPDATE_GOAL operation"
    );

    match action.to_lowercase().as_str() {
        "clear" => {
            // Remove all goals by restoring checkpoint with empty goals
            // We do this by iterating over current goals and removing each one
            let current_goals = ctx.aam.goals();
            for goal in current_goals {
                ctx.aam.remove_goal(
                    goal.id,
                    TransitionLabel::Custom("update_goal:clear".to_string()),
                );
            }
            tracing::info!(execution_id = %ctx.execution_id, "Cleared all goals");
            Ok(Value::String("cleared".to_string()))
        }

        "remove" => {
            // Remove a specific goal by matching description or goal_id attribute
            let goal_id_str = get_string_attribute(node, graph_attrs::GOAL_ID)?;
            let goals = ctx.aam.goals();
            let to_remove = goals.iter().find(|g| g.description == goal_id_str);
            match to_remove {
                Some(goal) => {
                    let id = goal.id;
                    ctx.aam.remove_goal(
                        id,
                        TransitionLabel::Custom(format!("update_goal:remove:{}", goal_id_str)),
                    );
                    tracing::info!(
                        execution_id = %ctx.execution_id,
                        goal_id = %goal_id_str,
                        "Removed goal"
                    );
                    Ok(Value::String("removed".to_string()))
                }
                None => {
                    tracing::warn!(
                        execution_id = %ctx.execution_id,
                        goal_id = %goal_id_str,
                        "Goal not found for removal (no-op)"
                    );
                    Ok(Value::String("not_found".to_string()))
                }
            }
        }

        _ => {
            let goal_id_str = get_string_attribute(node, graph_attrs::GOAL_ID)?;
            let priority =
                get_optional_u64_attribute(node, graph_attrs::PRIORITY)?.unwrap_or(1) as u32;

            // Get description from first input, or from goal_id attribute
            let description = inputs
                .first()
                .and_then(|v| v.as_string().map(|s| s.to_string()))
                .unwrap_or_else(|| goal_id_str.clone());

            // Remove existing goal with same description (upsert)
            let existing_goals = ctx.aam.goals();
            if let Some(existing) = existing_goals.iter().find(|g| g.description == goal_id_str) {
                let id = existing.id;
                ctx.aam.remove_goal(
                    id,
                    TransitionLabel::Custom(format!("update_goal:upsert_remove:{}", goal_id_str)),
                );
            }

            let goal = Goal {
                id: GoalId::new(),
                description,
                priority,
                status: GoalStatus::Active,
                parent_id: None,
            };

            ctx.aam.add_goal(
                goal,
                TransitionLabel::Custom(format!("update_goal:set:{}", goal_id_str)),
            );

            tracing::info!(
                execution_id = %ctx.execution_id,
                goal_id = %goal_id_str,
                priority = %priority,
                "Set goal"
            );

            Ok(Value::String("set".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::MemoryConfig;
    use apxm_core::types::operations::AISOperationType;
    use std::sync::Arc;

    async fn make_ctx_and_aam() -> (ExecutionContext, crate::aam::Aam) {
        let memory = Arc::new(
            crate::memory::MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());
        (ctx, aam)
    }

    fn make_set_node(goal_id: &str, priority: Option<u64>) -> Node {
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::UpdateGoal,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::ACTION.to_string(),
            Value::String("set".to_string()),
        );
        node.attributes.insert(
            graph_attrs::GOAL_ID.to_string(),
            Value::String(goal_id.to_string()),
        );
        if let Some(p) = priority {
            node.attributes.insert(
                graph_attrs::PRIORITY.to_string(),
                Value::Number(apxm_core::types::values::Number::Integer(p as i64)),
            );
        }
        node
    }

    #[tokio::test]
    async fn test_set_goal() {
        let (ctx, aam) = make_ctx_and_aam().await;

        let node = make_set_node("research_topic", Some(2));
        let result = execute(&ctx, &node, vec![Value::String("Investigate GPU".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::String("set".to_string()));

        let goals = aam.goals();
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].description, "Investigate GPU");
        assert_eq!(goals[0].priority, 2);
    }

    #[tokio::test]
    async fn test_remove_goal() {
        let (ctx, aam) = make_ctx_and_aam().await;

        // First add a goal
        let set_node = make_set_node("old_task", None);
        execute(&ctx, &set_node, vec![Value::String("old_task".to_string())])
            .await
            .unwrap();
        assert_eq!(aam.goals().len(), 1);

        // Now remove it
        let mut remove_node = Node {
            id: 2,
            op_type: AISOperationType::UpdateGoal,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        remove_node.attributes.insert(
            graph_attrs::ACTION.to_string(),
            Value::String("remove".to_string()),
        );
        remove_node.attributes.insert(
            graph_attrs::GOAL_ID.to_string(),
            Value::String("old_task".to_string()),
        );

        let result = execute(&ctx, &remove_node, vec![]).await.unwrap();
        assert_eq!(result, Value::String("removed".to_string()));
        assert_eq!(aam.goals().len(), 0);
    }

    #[tokio::test]
    async fn test_remove_goal_not_found() {
        let (ctx, _aam) = make_ctx_and_aam().await;

        let mut node = Node {
            id: 1,
            op_type: AISOperationType::UpdateGoal,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::ACTION.to_string(),
            Value::String("remove".to_string()),
        );
        node.attributes.insert(
            graph_attrs::GOAL_ID.to_string(),
            Value::String("nonexistent".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::String("not_found".to_string()));
    }

    #[tokio::test]
    async fn test_clear_goals() {
        let (ctx, aam) = make_ctx_and_aam().await;

        // Add two goals
        let node1 = make_set_node("goal_a", None);
        execute(&ctx, &node1, vec![Value::String("goal_a".to_string())])
            .await
            .unwrap();
        let mut node2 = make_set_node("goal_b", None);
        node2.id = 2;
        execute(&ctx, &node2, vec![Value::String("goal_b".to_string())])
            .await
            .unwrap();
        assert_eq!(aam.goals().len(), 2);

        // Clear all goals
        let mut clear_node = Node {
            id: 3,
            op_type: AISOperationType::UpdateGoal,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        clear_node.attributes.insert(
            graph_attrs::ACTION.to_string(),
            Value::String("clear".to_string()),
        );

        let result = execute(&ctx, &clear_node, vec![]).await.unwrap();
        assert_eq!(result, Value::String("cleared".to_string()));
        assert_eq!(aam.goals().len(), 0);
    }
}
