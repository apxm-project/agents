//! SPAWN_AGENT operation - Create a new agent instance at runtime
//!
//! Registers a new agent in the flow registry and/or process table. The agent
//! can then receive COMMUNICATE or DELEGATE messages. Returns the agent's
//! identifier.
//!
//! ## Attributes
//! - `agent_name`    (required): name for the new agent
//! - `profile`       (optional): ACP agent profile registered by the frontend.
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
use crate::constants::env as runtime_env;
use crate::metadata_keys as metadata;
use apxm_core::apxm_op;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::context_stack as context_stack_consts;
use apxm_core::constants::runtime::{belief_keys, response_keys};
use apxm_core::error::RuntimeError;
use apxm_core::types::aam::{AamContext, CapabilityProjection, GoalProjection};
use apxm_core::types::goal::GoalStatus;
use apxm_core::types::{ProcessSpawnMetric, SpawnedProcessKind};
use std::collections::HashMap;
use std::path::PathBuf;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let agent_name = get_string_attribute(node, graph_attrs::AGENT_NAME)?;
    let profile = get_optional_string_attribute(node, graph_attrs::PROFILE)?;

    apxm_op!(info,
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
    // Persist instructions+model so HANDOFF/COMMUNICATE can dispatch against
    // inline-spawned agents without requiring a separately-registered flow.
    if let Some(sp) = get_optional_string_attribute(node, graph_attrs::SYSTEM_PROMPT)? {
        agent_info.insert(response_keys::SYSTEM_PROMPT.to_string(), Value::String(sp));
    }
    if let Some(backend) = get_optional_string_attribute(node, graph_attrs::BACKEND)? {
        agent_info.insert(response_keys::BACKEND.to_string(), Value::String(backend));
    }
    if let Some(m) = get_optional_string_attribute(node, graph_attrs::MODEL)? {
        agent_info.insert(response_keys::MODEL.to_string(), Value::String(m));
    }

    // When profile is present, spawn an ACP subprocess
    if let Some(profile_name) = &profile {
        let spawn_start = std::time::Instant::now();
        let spawner = match ctx.process_table.agent_spawner().await {
            Some(spawner) => spawner,
            None => {
                let message = "No AgentSpawner configured. Cannot spawn ACP agent.".to_string();
                ctx.graph_metrics.record_spawn(ProcessSpawnMetric {
                    node_id: node.id,
                    agent_name: agent_name.clone(),
                    process_id: None,
                    parent_process_id: parent_process_id.clone(),
                    profile: Some(profile_name.clone()),
                    process_kind: SpawnedProcessKind::External,
                    duration_ms: spawn_start.elapsed().as_millis() as u64,
                    success: false,
                    error: Some(message.clone()),
                });
                return Err(RuntimeError::Operation {
                    op_type: node.op_type,
                    message,
                });
            }
        };

        let mode = get_optional_string_attribute(node, graph_attrs::MODE)?;
        let model = get_optional_string_attribute(node, graph_attrs::MODEL)?;
        // Determine node workspace folder for APXM context files. The spawned
        // agent adapter may read this generic APXM-owned path while cwd stays
        // at the project root for normal build/test workflows.
        let node_workspace = if let Some(session_dir) = ctx.metadata.get(metadata::SESSION_DIR) {
            let nodes_dir =
                PathBuf::from(session_dir).join(apxm_core::constants::session::files::NODES_DIR);
            let prefix = format!("{:02}_", node.id);
            std::fs::read_dir(&nodes_dir).ok().and_then(|mut entries| {
                entries.find_map(|e| {
                    let entry = e.ok()?;
                    let name = entry.file_name();
                    let name_str = name.to_str()?;
                    if name_str.starts_with(&prefix) {
                        Some(nodes_dir.join(name_str))
                    } else {
                        None
                    }
                })
            })
        } else {
            None
        };

        let cwd = if let Some(explicit_cwd) = get_optional_string_attribute(node, graph_attrs::CWD)?
        {
            PathBuf::from(explicit_cwd)
        } else {
            // Default: project root so agent can build/test/commit.
            // Context files are in node_workspace (passed via env).
            std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir())
        };

        // Project current AAM state into AamContext for the spawned agent
        let aam_context = project_aam_context(ctx, node.id, profile_name);

        // Build generic APXM-owned env for the agent subprocess. Adapter-specific
        // environment belongs in the registered ACP profile, not in runtime.
        let mut extra_env = std::collections::HashMap::new();
        if let Some(ref ws) = node_workspace {
            let ws_str = ws.to_string_lossy().into_owned();
            extra_env.insert(runtime_env::APXM_NODE_WORKSPACE.to_string(), ws_str);
        }

        let session = spawner
            .spawn_external(
                &agent_name,
                profile_name,
                &cwd,
                mode.as_deref(),
                model.as_deref(),
                &aam_context,
                &extra_env,
            )
            .await?;

        // Register the session — if this fails, shut down the subprocess to avoid leaking it
        let process_id = ctx
            .process_table
            .register_external(
                agent_name.clone(),
                parent_process_id.clone(),
                session,
                profile_name.clone(),
            )
            .map_err(|e| {
                let message = format!("Failed to register ACP process '{}': {}", agent_name, e);
                ctx.graph_metrics.record_spawn(ProcessSpawnMetric {
                    node_id: node.id,
                    agent_name: agent_name.clone(),
                    process_id: None,
                    parent_process_id: parent_process_id.clone(),
                    profile: Some(profile_name.clone()),
                    process_kind: SpawnedProcessKind::External,
                    duration_ms: spawn_start.elapsed().as_millis() as u64,
                    success: false,
                    error: Some(message.clone()),
                });
                RuntimeError::Operation {
                    op_type: node.op_type,
                    message,
                }
            })?;

        ctx.graph_metrics.record_spawn(ProcessSpawnMetric {
            node_id: node.id,
            agent_name: agent_name.clone(),
            process_id: Some(process_id.clone()),
            parent_process_id: parent_process_id.clone(),
            profile: Some(profile_name.clone()),
            process_kind: SpawnedProcessKind::External,
            duration_ms: spawn_start.elapsed().as_millis() as u64,
            success: true,
            error: None,
        });

        agent_info.insert(
            response_keys::PROFILE.to_string(),
            Value::String(profile_name.clone()),
        );
        agent_info.insert(
            response_keys::PROCESS_ID.to_string(),
            Value::String(process_id),
        );

        apxm_op!(info,
            execution_id = %ctx.execution_id,
            agent_name = %agent_name,
            profile = %profile_name,
            "SPAWN_AGENT: ACP subprocess spawned and registered in ProcessTable"
        );
    } else {
        // No profile — register as a local process for tracking
        let spawn_start = std::time::Instant::now();
        match ctx
            .process_table
            .spawn_local(agent_name.clone(), parent_process_id.clone())
        {
            Ok(process_id) => {
                ctx.graph_metrics.record_spawn(ProcessSpawnMetric {
                    node_id: node.id,
                    agent_name: agent_name.clone(),
                    process_id: Some(process_id.clone()),
                    parent_process_id: parent_process_id.clone(),
                    profile: None,
                    process_kind: SpawnedProcessKind::Local,
                    duration_ms: spawn_start.elapsed().as_millis() as u64,
                    success: true,
                    error: None,
                });
                agent_info.insert(
                    response_keys::PROCESS_ID.to_string(),
                    Value::String(process_id),
                );
            }
            Err(e) => {
                ctx.graph_metrics.record_spawn(ProcessSpawnMetric {
                    node_id: node.id,
                    agent_name: agent_name.clone(),
                    process_id: None,
                    parent_process_id: parent_process_id.clone(),
                    profile: None,
                    process_kind: SpawnedProcessKind::Local,
                    duration_ms: spawn_start.elapsed().as_millis() as u64,
                    success: false,
                    error: Some(e.to_string()),
                });
                // Log but don't fail — local agents work via FlowRegistry without a process entry
                apxm_op!(warn,
                    agent_name = %agent_name,
                    error = %e,
                    "SPAWN_AGENT: local process registration failed (agent still usable via FlowRegistry)"
                );
            }
        }
    }

    // Write agent_info to the parent (flow-root) scope so HANDOFF/COMMUNICATE
    // running in sibling worker scopes can see it. The scheduler creates a
    // fresh child scope per node (`base_ctx.child()` in scheduler/worker.rs),
    // so writing to ctx.scope_id() here would isolate agent_info from peers.
    let stm_key = format!("{}{}", belief_keys::AGENT_INFO_PREFIX, agent_name);
    let target_scope = ctx
        .metadata
        .get(metadata::PARENT_SCOPE_ID)
        .cloned()
        .unwrap_or_else(|| ctx.scope_id().to_string());
    if let Err(e) = ctx
        .memory
        .write_scoped(
            crate::memory::MemorySpace::Stm,
            &target_scope,
            stm_key,
            Value::Object(agent_info.clone()),
        )
        .await
    {
        // STM write failure is non-fatal but warn — downstream COMMUNICATE lookups may fail
        apxm_op!(warn,
            agent_name = %agent_name,
            error = %e,
            "SPAWN_AGENT: failed to write agent info to STM (downstream lookups may fail)"
        );
    }

    apxm_op!(info,
        execution_id = %ctx.execution_id,
        agent_name = %agent_name,
        "SPAWN_AGENT completed successfully"
    );

    Ok(Value::Object(agent_info))
}

/// Project the current AAM state into an `AamContext` for transmission to a spawned agent.
///
/// Filters out internal beliefs (prefixed with `_`) and only includes active goals.
/// Values are converted from the runtime `Value` type to `serde_json::Value`.
fn project_aam_context(ctx: &ExecutionContext, node_id: u64, profile: &str) -> AamContext {
    let beliefs: HashMap<String, serde_json::Value> = ctx
        .aam
        .beliefs()
        .into_iter()
        .filter(|(k, _)| !k.starts_with(belief_keys::INTERNAL_PREFIX))
        .filter_map(|(k, v)| v.to_json().ok().map(|jv| (k, jv)))
        .collect();

    let goals: Vec<GoalProjection> = ctx
        .aam
        .goals()
        .into_iter()
        .filter(|g| g.status == GoalStatus::Active)
        .map(|g| GoalProjection {
            description: g.description,
            priority: g.priority,
        })
        .collect();

    let capabilities: Vec<CapabilityProjection> = ctx
        .aam
        .capabilities()
        .into_iter()
        .map(|(_, rec)| CapabilityProjection {
            name: rec.name,
            description: rec.description,
        })
        .collect();

    let system_prompt = ctx.context_stack.as_ref().and_then(|stack| {
        let assembly = stack.assemble(
            node_id,
            profile,
            context_stack_consts::DEFAULT_PROMPT_BUDGET_TOKENS,
        );

        if assembly.frames.is_empty() {
            None
        } else {
            Some(assembly.to_string())
        }
    });

    AamContext {
        beliefs,
        goals,
        capabilities,
        system_prompt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::capability::flow_registry::FlowRegistry;
    use crate::context_stack::{ContextStack, NodeMetadata as ContextNodeMetadata};
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::constants::graph::attrs as graph_attrs;
    use apxm_core::constants::runtime::{belief_keys, response_keys};
    use apxm_core::constants::session;
    use apxm_core::paths::session_node_dir_name;
    use apxm_core::types::execution::NodeMetadata;
    use apxm_core::types::operations::AISOperationType;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::tempdir;

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

    #[tokio::test]
    async fn test_project_aam_context_includes_context_stack_prompt() {
        let dir = tempdir().expect("tempdir");
        let session_dir = dir.path().join("session");
        let upstream_dir = session_dir
            .join(session::files::NODES_DIR)
            .join(session_node_dir_name(1, "seed"));
        std::fs::create_dir_all(&upstream_dir).expect("upstream dir");
        std::fs::write(
            upstream_dir.join(session::node::OUTPUT_JSON),
            r#"{"result":"design context"}"#,
        )
        .expect("output");

        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let mut node_metadata = HashMap::new();
        node_metadata.insert(
            1,
            ContextNodeMetadata {
                name: "seed".to_string(),
                op_type: AISOperationType::ConstStr,
            },
        );
        node_metadata.insert(
            2,
            ContextNodeMetadata {
                name: "spawn".to_string(),
                op_type: AISOperationType::SpawnAgent,
            },
        );

        let context_stack = Arc::new(ContextStack::new(
            session_dir,
            Arc::new(node_metadata),
            Arc::new(vec![(1, 2)]),
        ));

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
        .with_context_stack(context_stack);

        let projected = project_aam_context(&ctx, 2, context_stack_consts::DEFAULT_PROFILE);

        let system_prompt = projected.system_prompt.expect("system_prompt");
        assert!(system_prompt.contains("## Upstream: seed (#1)"));
        assert!(system_prompt.contains("design context"));
    }
}
