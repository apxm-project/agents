//! SPAWN_TEAM operation - Spawn all members of a team
//!
//! Expands a team definition from ~/.apxm/teams.toml into N SPAWN_AGENT operations.
//! Each team member is spawned with its configured role, profile, and optional system_prompt.
//!
//! ## Attributes
//! - `team_name` (required): name of the team to spawn (from ~/.apxm/teams.toml)
//! - `cwd` (optional): working directory for all team member subprocesses

use super::{ExecutionContext, Node, Result, Value, get_optional_string_attribute, get_string_attribute};
use apxm_core::apxm_op;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::response_keys;
use apxm_core::error::RuntimeError;
use crate::team::TeamRegistry;
use std::collections::HashMap;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let team_name = get_string_attribute(node, graph_attrs::TEAM_NAME)?;
    let cwd = get_optional_string_attribute(node, graph_attrs::CWD)?;

    apxm_op!(info,
        execution_id = %ctx.execution_id,
        team_name = %team_name,
        "Executing SPAWN_TEAM operation"
    );

    // Load team registry
    let registry = TeamRegistry::load_from_default_path();
    let team_def = registry.get(&team_name).ok_or_else(|| {
        let available_teams: Vec<String> = registry.list().iter().map(|t| t.name.clone()).collect();
        let hint = if available_teams.is_empty() {
            "No teams defined in ~/.apxm/teams.toml. Use 'apxm team add' to create a team.".to_string()
        } else {
            format!(
                "Team '{}' not found. Available teams: {}. Check ~/.apxm/teams.toml or use 'apxm team list'.",
                team_name,
                available_teams.join(", ")
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
        "Team definition loaded, spawning members"
    );

    // Spawn each team member as a SPAWN_AGENT operation
    let mut spawned = HashMap::new();
    for member in &team_def.members {
        apxm_op!(debug,
            team_name = %team_name,
            role = %member.role,
            profile = %member.profile,
            "Spawning team member"
        );

        // Build attributes for SPAWN_AGENT
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

        // Create a synthetic SPAWN_AGENT node
        let spawn_node = apxm_core::types::execution::Node {
            id: node.id,
            op_type: apxm_core::types::operations::AISOperationType::SpawnAgent,
            attributes: spawn_attrs,
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: node.metadata.clone(),
        };

        // Execute SPAWN_AGENT
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

        apxm_op!(info,
            team_name = %team_name,
            role = %member.role,
            "Team member spawned successfully"
        );
    }

    apxm_op!(info,
        execution_id = %ctx.execution_id,
        team_name = %team_name,
        spawned_count = spawned.len(),
        "SPAWN_TEAM completed successfully"
    );

    // Return object containing all spawned agent metadata
    let mut result = HashMap::new();
    result.insert(
        response_keys::TEAM_NAME.to_string(),
        Value::String(team_name.clone()),
    );
    result.insert(
        "members".to_string(),
        Value::Object(spawned),
    );

    Ok(Value::Object(result))
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

    #[tokio::test]
    async fn test_spawn_team_missing_team_name() {
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

        let node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::SpawnTeam,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: NodeMetadata::default(),
        };

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("team_name"));
    }

    #[tokio::test]
    async fn test_spawn_team_not_found() {
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
            op_type: AISOperationType::SpawnTeam,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::TEAM_NAME.to_string(),
            Value::String("nonexistent_team".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not found") || err_msg.contains("No teams"));
    }
}
