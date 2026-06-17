use super::{
    ExecutionContext, Node, Result, Value, get_optional_string_attribute, get_string_attribute,
};
use crate::team::TeamRegistry;
use apxm_core::apxm_op;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::response_keys;
use apxm_core::error::RuntimeError;
use std::collections::HashMap;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let team_name = get_string_attribute(node, graph_attrs::TEAM_NAME)?;
    let cwd = get_optional_string_attribute(node, graph_attrs::CWD)?;

    let registry = TeamRegistry::load_from_default_path();
    let team_def = registry.get(&team_name).ok_or_else(|| {
        let available: Vec<&str> = registry.list().iter().map(|t| t.name.as_str()).collect();
        let hint = if available.is_empty() {
            "No teams defined in ~/.apxm/teams.toml".to_string()
        } else {
            format!(
                "Team '{}' not found. Available: {}",
                team_name,
                available.join(", ")
            )
        };
        RuntimeError::Operation {
            op_type: node.op_type,
            message: hint,
        }
    })?;

    apxm_op!(info,
        team_name = %team_name,
        members = team_def.members.len(),
        "Spawning team members"
    );

    let mut spawned = HashMap::new();
    for member in &team_def.members {
        let mut spawn_attrs = HashMap::new();
        spawn_attrs.insert(
            graph_attrs::AGENT_NAME.to_string(),
            Value::String(member.role.clone()),
        );
        spawn_attrs.insert(
            graph_attrs::PROFILE.to_string(),
            Value::String(member.profile.clone()),
        );
        if let Some(ref cwd_path) = cwd {
            spawn_attrs.insert(
                graph_attrs::CWD.to_string(),
                Value::String(cwd_path.clone()),
            );
        }
        if let Some(ref prompt) = member.system_prompt {
            spawn_attrs.insert(
                graph_attrs::SYSTEM_PROMPT.to_string(),
                Value::String(prompt.clone()),
            );
        }

        let spawn_node = apxm_core::types::execution::Node {
            id: node.id,
            op_type: apxm_core::types::operations::AISOperationType::SpawnAgent,
            attributes: spawn_attrs,
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: node.metadata.clone(),
        };

        let result = super::spawn_agent::execute(ctx, &spawn_node, vec![])
            .await
            .map_err(|e| RuntimeError::Operation {
                op_type: node.op_type,
                message: format!(
                    "Failed to spawn team member '{}' (profile '{}'): {}",
                    member.role, member.profile, e
                ),
            })?;

        spawned.insert(member.role.clone(), result);
    }

    apxm_op!(info, team_name = %team_name, spawned = spawned.len(), "SPAWN_TEAM completed");

    let mut result = HashMap::new();
    result.insert(
        response_keys::TEAM_NAME.to_string(),
        Value::String(team_name),
    );
    result.insert(response_keys::MEMBERS.to_string(), Value::Object(spawned));

    Ok(Value::Object(result))
}
