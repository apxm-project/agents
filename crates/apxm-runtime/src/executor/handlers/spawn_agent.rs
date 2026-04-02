//! SPAWN_AGENT operation - Create a new agent instance at runtime
//!
//! Registers a new agent in the flow registry and/or process table. The agent
//! can then receive COMMUNICATE or DELEGATE messages. Returns the agent's
//! identifier.
//!
//! ## Attributes
//! - `agent_name`    (required): name for the new agent
//! - `profile`       (optional): ACP agent profile (e.g. "claude", "codex").
//!   When present, spawns a real ACP subprocess via the ProcessTable's
//!   `AgentSpawner`.
//! - `mode`          (optional): agent mode to set after spawn (e.g. "architect")
//! - `model`         (optional): model override (e.g. "claude-sonnet-4")
//! - `cwd`           (optional): working directory for the agent subprocess
//! - `capabilities`  (optional): list of capabilities
//! - `goals`         (optional): initial goals

use super::{
    ExecutionContext, Node, Result, Value, get_optional_string_attribute, get_string_attribute,
};
use crate::aam::TransitionLabel;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::{belief_keys, response_keys};
use apxm_core::error::RuntimeError;
use std::collections::HashMap;
use std::path::PathBuf;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let agent_name = get_string_attribute(node, graph_attrs::AGENT_NAME)?;
    let profile = get_optional_string_attribute(node, graph_attrs::PROFILE)?;

    tracing::info!(
        execution_id = %ctx.execution_id,
        agent_name = %agent_name,
        profile = ?profile,
        "Executing SPAWN_AGENT operation"
    );

    // Check if agent already exists in either the flow registry or process table
    let existing_flows = ctx.flow_registry.flows_for_agent(&agent_name);
    if !existing_flows.is_empty() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Agent '{}' already exists in the flow registry", agent_name),
        });
    }
    if ctx.process_table.get_by_name(&agent_name).is_some() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Agent '{}' already exists in the process table", agent_name),
        });
    }

    // Resolve the parent process ID from the current agent context
    let parent_process_id = ctx
        .current_agent
        .as_ref()
        .and_then(|agent| ctx.process_table.get_by_name(&agent.name))
        .map(|entry| entry.id.clone());

    // Record agent spawn in AAM
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::SPAWNED_AGENT_PREFIX, agent_name),
        Value::String(agent_name.clone()),
        TransitionLabel::Custom(format!("spawn_agent:{}", agent_name)),
    );

    // Store the new agent's metadata in STM for later reference
    let mut agent_info = HashMap::new();
    agent_info.insert(
        response_keys::NAME.to_string(),
        Value::String(agent_name.clone()),
    );
    agent_info.insert(
        response_keys::SPAWNED_BY.to_string(),
        Value::String(ctx.execution_id.clone()),
    );

    if let Some(capabilities) = node.attributes.get(response_keys::CAPABILITIES) {
        agent_info.insert(
            response_keys::CAPABILITIES.to_string(),
            capabilities.clone(),
        );
    }
    if let Some(goals) = node.attributes.get(response_keys::GOALS) {
        agent_info.insert(response_keys::GOALS.to_string(), goals.clone());
    }

    // When profile is present, spawn an ACP subprocess
    if let Some(profile_name) = &profile {
        let spawner = ctx
            .process_table
            .agent_spawner()
            .await
            .ok_or_else(|| RuntimeError::Operation {
                op_type: node.op_type,
                message: "No AgentSpawner configured. Cannot spawn ACP agent.".to_string(),
            })?;

        let mode = get_optional_string_attribute(node, graph_attrs::MODE)?;
        let model = get_optional_string_attribute(node, graph_attrs::MODEL)?;
        let cwd = get_optional_string_attribute(node, graph_attrs::CWD)?
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let session = spawner
            .spawn_external(
                &agent_name,
                profile_name,
                &cwd,
                mode.as_deref(),
                model.as_deref(),
            )
            .await?;

        let process_id = ctx
            .process_table
            .register_external(
                agent_name.clone(),
                parent_process_id.clone(),
                session,
                profile_name.clone(),
            )
            .map_err(|e| RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("Failed to register process: {}", e),
            })?;

        agent_info.insert(
            response_keys::PROFILE.to_string(),
            Value::String(profile_name.clone()),
        );
        agent_info.insert(
            response_keys::PROCESS_ID.to_string(),
            Value::String(process_id),
        );

        tracing::info!(
            execution_id = %ctx.execution_id,
            agent_name = %agent_name,
            profile = %profile_name,
            "SPAWN_AGENT: ACP subprocess spawned and registered in ProcessTable"
        );
    } else {
        // No profile — register as a local process for tracking
        let process_id = ctx
            .process_table
            .spawn_local(agent_name.clone(), parent_process_id)
            .unwrap_or_default(); // Non-fatal: local agents can still work via FlowRegistry

        if !process_id.is_empty() {
            agent_info.insert(
                response_keys::PROCESS_ID.to_string(),
                Value::String(process_id),
            );
        }
    }

    let _ = ctx
        .memory
        .write_scoped(
            crate::memory::MemorySpace::Stm,
            ctx.scope_id(),
            format!("{}{}", belief_keys::AGENT_INFO_PREFIX, agent_name),
            Value::Object(agent_info.clone()),
        )
        .await;

    tracing::info!(
        execution_id = %ctx.execution_id,
        agent_name = %agent_name,
        "SPAWN_AGENT completed successfully"
    );

    Ok(Value::Object(agent_info))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::capability::flow_registry::FlowRegistry;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::constants::graph::attrs as graph_attrs;
    use apxm_core::constants::runtime::{belief_keys, response_keys};
    use apxm_core::types::execution::NodeMetadata;
    use apxm_core::types::operations::AISOperationType;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_spawn_node(agent_name: &str) -> apxm_core::types::execution::Node {
        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::SpawnAgent,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            Value::String(agent_name.to_string()),
        );
        node
    }

    #[tokio::test]
    async fn test_spawn_agent_registers_in_aam() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_spawn_node("research_agent");
        let result = execute(&ctx, &node, vec![]).await.unwrap();

        // Check returned object has the agent name
        match &result {
            Value::Object(obj) => {
                assert_eq!(
                    obj.get(response_keys::NAME),
                    Some(&Value::String("research_agent".to_string()))
                );
                // Should contain spawned_by with execution_id
                assert!(obj.contains_key(response_keys::SPAWNED_BY));
            }
            _ => panic!("Expected Value::Object, got {:?}", result),
        }

        // Check AAM has the spawned agent belief
        let beliefs = ctx.aam.beliefs();
        let key = format!("{}research_agent", belief_keys::SPAWNED_AGENT_PREFIX);
        assert_eq!(
            beliefs.get(&key),
            Some(&Value::String("research_agent".to_string())),
            "AAM should record the spawned agent"
        );
    }

    #[tokio::test]
    async fn test_spawn_agent_stores_in_memory() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(
            memory.clone(),
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        let node = make_spawn_node("worker_agent");
        let _ = execute(&ctx, &node, vec![]).await.unwrap();

        // Check STM for agent info
        let key = format!("{}worker_agent", belief_keys::AGENT_INFO_PREFIX);
        let stored = memory
            .read_scoped(crate::memory::MemorySpace::Stm, ctx.scope_id(), &key)
            .await
            .unwrap();
        assert!(stored.is_some(), "Agent info should be stored in STM");
    }

    #[tokio::test]
    async fn test_spawn_agent_duplicate_rejected() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        // Pre-register a flow for "existing_agent" so it already "exists"
        let dag = apxm_core::types::execution::ExecutionDag {
            nodes: vec![],
            edges: vec![],
            entry_nodes: vec![],
            exit_nodes: vec![],
            metadata: Default::default(),
        };
        flow_registry.register_flow("existing_agent", "main", dag);

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

        let node = make_spawn_node("existing_agent");
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[tokio::test]
    async fn test_spawn_agent_missing_name() {
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
            op_type: AISOperationType::SpawnAgent,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("agent_name"));
    }

    #[tokio::test]
    async fn test_spawn_agent_with_capabilities_and_goals() {
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

        let mut node = make_spawn_node("skilled_agent");
        node.attributes.insert(
            response_keys::CAPABILITIES.to_string(),
            Value::Array(vec![
                Value::String("web_search".to_string()),
                Value::String("code_gen".to_string()),
            ]),
        );
        node.attributes.insert(
            response_keys::GOALS.to_string(),
            Value::Array(vec![Value::String("find information".to_string())]),
        );

        let result = execute(&ctx, &node, vec![]).await.unwrap();

        match &result {
            Value::Object(obj) => {
                assert!(obj.contains_key(response_keys::CAPABILITIES));
                assert!(obj.contains_key(response_keys::GOALS));
            }
            _ => panic!("Expected Value::Object"),
        }
    }
}
