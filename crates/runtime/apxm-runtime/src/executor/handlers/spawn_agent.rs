//! SPAWN_AGENT operation - Create a new agent instance at runtime
//!
//! Registers a new agent in the flow registry and/or process table. The agent
//! can then receive COMMUNICATE or DELEGATE messages. Returns the agent's
//! identifier.
//!
//! ## Attributes
//! - `agent_name`    (required): name for the new agent
//! - `profile`       (optional): explicit APXM ACP agent profile.
//! - `agent_route`   (optional): set to "auto" to let APXM select a profile.
//! - `required_capabilities` / `preferred_profiles` (optional): route hints.
//! - `mode`          (optional): agent mode to set after spawn (e.g. "architect")
//! - `model`         (optional): model override (e.g. "claude-sonnet-4")
//! - `cwd`           (optional): working directory for the agent subprocess
//! - `capabilities`  (optional): list of capabilities
//! - `goals`         (optional): initial goals

use super::{
    ExecutionContext, Node, Result, Value, get_optional_string_attribute, get_string_attribute,
};
use crate::aam::TransitionLabel;
use crate::agent_router::{
    AGENT_ROUTE_CAPABILITIES, AGENT_ROUTE_SELECTOR_DETERMINISTIC, AgentRouteDecision,
    AgentRouteRequest, AgentRouter,
};
use crate::constants::env as runtime_env;
use crate::metadata_keys as metadata;
use apxm_core::apxm_op;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::context_stack as context_stack_consts;
use apxm_core::constants::runtime::{belief_keys, response_keys};
use apxm_core::error::RuntimeError;
use apxm_core::types::aam::{AamContext, CapabilityProjection, GoalProjection};
use apxm_core::types::goal::GoalStatus;
use apxm_core::types::{Number, ProcessSpawnMetric, SpawnedProcessKind};
use std::collections::HashMap;
use std::path::PathBuf;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let agent_name = get_string_attribute(node, graph_attrs::AGENT_NAME)?;
    let initial_profile = get_optional_string_attribute(node, graph_attrs::PROFILE)?;
    let route_mode = get_optional_string_attribute(node, graph_attrs::AGENT_ROUTE)?;
    let required_capabilities =
        get_optional_string_list_attribute(node, graph_attrs::REQUIRED_CAPABILITIES)?;
    let preferred_profiles =
        get_optional_string_list_attribute(node, graph_attrs::PREFERRED_PROFILES)?;
    let wants_route =
        route_mode.is_some() || !required_capabilities.is_empty() || !preferred_profiles.is_empty();

    apxm_op!(info,
        execution_id = %ctx.execution_id,
        agent_name = %agent_name,
        profile = ?initial_profile,
        agent_route = ?route_mode,
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

    let route = if wants_route {
        Some(
            resolve_spawn_agent_route(
                ctx,
                node,
                &agent_name,
                initial_profile.clone(),
                route_mode.as_deref(),
                required_capabilities.clone(),
                preferred_profiles.clone(),
            )
            .await?,
        )
    } else {
        None
    };
    let profile = route
        .as_ref()
        .and_then(|decision| decision.profile.clone())
        .or(initial_profile);

    // When profile is present or APXM selected one, spawn an ACP subprocess.
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

        let mode = route
            .as_ref()
            .and_then(|decision| decision.mode.clone())
            .or(get_optional_string_attribute(node, graph_attrs::MODE)?);
        let model = route
            .as_ref()
            .and_then(|decision| decision.model.clone())
            .or(get_optional_string_attribute(node, graph_attrs::MODEL)?);
        if let Some(mode) = &mode {
            agent_info.insert(graph_attrs::MODE.to_string(), Value::String(mode.clone()));
        }
        if let Some(model) = &model {
            agent_info.insert(
                response_keys::MODEL.to_string(),
                Value::String(model.clone()),
            );
        }
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
        // environment belongs in the APXM ACP profile, not in runtime.
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
        if let Some(decision) = &route {
            agent_info.insert(
                response_keys::ROUTE_SOURCE.to_string(),
                Value::String(decision.source.as_str().to_string()),
            );
            agent_info.insert(
                response_keys::ROUTE_SELECTOR.to_string(),
                Value::String(AGENT_ROUTE_SELECTOR_DETERMINISTIC.to_string()),
            );
            agent_info.insert(
                response_keys::ROUTE_REASON.to_string(),
                Value::String(decision.reason.clone()),
            );
            agent_info.insert(
                response_keys::ROUTE_CANDIDATE_SNAPSHOT.to_string(),
                Value::String(decision.candidate_snapshot_hash.clone()),
            );
            agent_info.insert(
                response_keys::ELIGIBLE_PROFILES.to_string(),
                Value::Array(
                    decision
                        .eligible_profiles
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
            agent_info.insert(
                response_keys::REJECTED_PROFILES.to_string(),
                Value::Array(
                    decision
                        .rejected_candidates
                        .iter()
                        .map(|rejection| Value::String(rejection.profile.clone()))
                        .collect(),
                ),
            );
            agent_info.insert(
                response_keys::ROUTE_SCORES.to_string(),
                route_scores_value(decision),
            );
            agent_info.insert(
                graph_attrs::REQUIRED_CAPABILITIES.to_string(),
                Value::Array(
                    decision
                        .required_capabilities
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
            agent_info.insert(
                graph_attrs::PREFERRED_PROFILES.to_string(),
                Value::Array(
                    decision
                        .preferred_profiles
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
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

    // Emit a typed AGENT_SPAWNED event so observers can
    // attach an agent label to this node without scraping STM.
    if let Some(emitter) = &ctx.event_emitter {
        let process_id = agent_info
            .get(response_keys::PROCESS_ID)
            .and_then(|value| value.as_string())
            .map(|s| s.to_string());
        emitter.emit_agent_spawned(
            node.id,
            &agent_name,
            &ctx.execution_id,
            profile.as_deref(),
            process_id.as_deref(),
            None,
        );
    }

    // Layer 2 — push an agent scope and bracket it with
    // `subagent_spawn_begin` / `subagent_spawn_end`. Subsequent ASK /
    // INV_TOOL handlers see a non-empty stack and emit paired Layer 2
    // events tagged with this `agent_code`. The scope's pop site lives
    // in the spawned subgraph's terminal handler — see the engine
    // bracket below; for now we leave the scope on the stack so the
    // remainder of the run benefits.
    let parent_span_id = ctx.agent_scope_stack.peek().map(|s| s.span_id);
    let span_id = format!("agent-{}-{}", agent_name, node.id);
    ctx.agent_scope_stack
        .push(crate::executor::agent_scope::AgentScope::new(
            agent_name.clone(),
            span_id,
            parent_span_id.clone(),
            None,
        ));
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_subagent_spawn_begin(
            &agent_name,
            Some(&agent_name),
            None,
            None,
            None,
            parent_span_id.as_deref(),
        );
        emitter.emit_subagent_spawn_end(&agent_name);
    }

    apxm_op!(info,
        execution_id = %ctx.execution_id,
        agent_name = %agent_name,
        "SPAWN_AGENT completed successfully"
    );

    Ok(Value::Object(agent_info))
}

async fn resolve_spawn_agent_route(
    ctx: &ExecutionContext,
    node: &Node,
    agent_name: &str,
    profile: Option<String>,
    route_mode: Option<&str>,
    required_capabilities: Vec<String>,
    preferred_profiles: Vec<String>,
) -> Result<AgentRouteDecision> {
    if let Some(mode) = route_mode {
        if mode != "auto" {
            return Err(RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("SPAWN_AGENT agent_route must be 'auto', got '{mode}'"),
            });
        }
    }
    validate_route_capabilities(node, &required_capabilities)?;

    let Some(spawner) = ctx.process_table.agent_spawner().await else {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: "No AgentSpawner configured. Cannot route ACP agent.".to_string(),
        });
    };
    let candidates = spawner.route_candidates();
    if candidates.is_empty() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: "SPAWN_AGENT routing found no APXM agent route candidates".to_string(),
        });
    }
    let request = AgentRouteRequest::spawn_agent(
        agent_name.to_string(),
        profile,
        get_optional_string_attribute(node, graph_attrs::MODE)?,
        get_optional_string_attribute(node, graph_attrs::MODEL)?,
        required_capabilities,
        preferred_profiles,
        true,
    );
    let profile_counts = ctx.process_table.external_profile_counts();
    AgentRouter::new(candidates)
        .route_requests_with_counts(&[request], &profile_counts)
        .map_err(|error| RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("SPAWN_AGENT routing failed: {error}"),
        })?
        .into_iter()
        .next()
        .ok_or_else(|| RuntimeError::Operation {
            op_type: node.op_type,
            message: "SPAWN_AGENT routing returned no decision".to_string(),
        })
}

fn validate_route_capabilities(node: &Node, capabilities: &[String]) -> Result<()> {
    for capability in capabilities {
        let capability = capability.trim().to_ascii_lowercase();
        if capability.is_empty() {
            continue;
        }
        if !AGENT_ROUTE_CAPABILITIES.contains(&capability.as_str()) {
            return Err(RuntimeError::Operation {
                op_type: node.op_type,
                message: format!(
                    "SPAWN_AGENT required_capabilities contains unsupported route capability '{capability}'. Use one of: {}",
                    AGENT_ROUTE_CAPABILITIES.join(", ")
                ),
            });
        }
    }
    Ok(())
}

fn route_scores_value(decision: &AgentRouteDecision) -> Value {
    Value::Array(
        decision
            .candidate_scores
            .iter()
            .map(|score| {
                let mut fields = HashMap::new();
                fields.insert("profile".to_string(), Value::String(score.profile.clone()));
                fields.insert("eligible".to_string(), Value::Bool(score.eligible));
                fields.insert(
                    "matched_capabilities".to_string(),
                    string_array_value(&score.matched_capabilities),
                );
                fields.insert(
                    "missing_capabilities".to_string(),
                    string_array_value(&score.missing_capabilities),
                );
                fields.insert(
                    "selected_count".to_string(),
                    Value::Number(Number::Integer(score.selected_count as i64)),
                );
                fields.insert(
                    "capability_fit_score".to_string(),
                    Value::Number(Number::Integer(score.capability_fit_score as i64)),
                );
                fields.insert(
                    "preference_rank".to_string(),
                    score
                        .preference_rank
                        .map(|rank| Value::Number(Number::Integer(rank as i64)))
                        .unwrap_or(Value::Null),
                );
                fields.insert(
                    "registry_index".to_string(),
                    Value::Number(Number::Integer(score.registry_index as i64)),
                );
                fields.insert("reason".to_string(), Value::String(score.reason.clone()));
                Value::Object(fields)
            })
            .collect(),
    )
}

fn string_array_value(values: &[String]) -> Value {
    Value::Array(values.iter().cloned().map(Value::String).collect())
}

fn get_optional_string_list_attribute(node: &Node, key: &str) -> Result<Vec<String>> {
    let Some(value) = node.attributes.get(key) else {
        return Ok(Vec::new());
    };
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_string()
                    .cloned()
                    .ok_or_else(|| RuntimeError::Operation {
                        op_type: node.op_type,
                        message: format!("Attribute {key} must be an array of strings"),
                    })
            })
            .collect(),
        Value::String(value) => Ok(vec![value.clone()]),
        _ => Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Attribute {key} must be an array of strings"),
        }),
    }
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
    use crate::process_table::AgentSpawner;
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

    struct RoutingTestSpawner {
        candidates: Vec<crate::agent_router::AgentRouteCandidate>,
    }

    #[async_trait::async_trait]
    impl AgentSpawner for RoutingTestSpawner {
        fn route_candidates(&self) -> Vec<crate::agent_router::AgentRouteCandidate> {
            self.candidates.clone()
        }

        async fn spawn_external(
            &self,
            _agent_name: &str,
            _profile_name: &str,
            _cwd: &std::path::Path,
            _mode: Option<&str>,
            _model: Option<&str>,
            _aam_context: &AamContext,
            _extra_env: &std::collections::HashMap<String, String>,
        ) -> std::result::Result<
            Arc<tokio::sync::Mutex<dyn std::any::Any + Send + Sync>>,
            RuntimeError,
        > {
            Ok(Arc::new(tokio::sync::Mutex::new(())))
        }
    }

    #[tokio::test]
    async fn spawn_agent_pushes_agent_scope_and_emits_layer2_bracket() {
        use crate::executor::events::ExecutionEventEmitter;
        use std::sync::Mutex;

        // Minimal recorder emitter that only captures the Layer 2
        // bracket events we care about for this test.
        #[derive(Default)]
        struct Recorder {
            spawn_begins: Mutex<Vec<String>>,
            spawn_ends: Mutex<Vec<String>>,
        }
        impl ExecutionEventEmitter for Recorder {
            fn emit_llm_token(&self, _content: &str) {}
            fn emit_tool_start(
                &self,
                _name: &str,
                _args: &std::collections::HashMap<String, Value>,
            ) {
            }
            fn emit_tool_end(&self, _name: &str, _result: &Value) {}
            fn emit_subagent_spawn_begin(
                &self,
                agent_code: &str,
                _agent_name: Option<&str>,
                _agent_type: Option<&str>,
                _module_key: Option<&str>,
                _autonomy_policy: Option<&str>,
                _parent_span_id: Option<&str>,
            ) {
                self.spawn_begins
                    .lock()
                    .unwrap()
                    .push(agent_code.to_string());
            }
            fn emit_subagent_spawn_end(&self, agent_code: &str) {
                self.spawn_ends.lock().unwrap().push(agent_code.to_string());
            }
        }

        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let recorder: Arc<Recorder> = Arc::new(Recorder::default());
        let emitter: Arc<dyn ExecutionEventEmitter> = recorder.clone();

        let mut ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );
        ctx.event_emitter = Some(emitter);

        assert!(ctx.agent_scope_stack.is_empty());

        let node = make_spawn_node("crm");
        let _ = execute(&ctx, &node, vec![]).await.unwrap();

        assert_eq!(ctx.agent_scope_stack.depth(), 1);
        let top = ctx.agent_scope_stack.peek().unwrap();
        assert_eq!(top.agent_code, "crm");
        assert!(top.parent_span_id.is_none());

        let begins = recorder.spawn_begins.lock().unwrap();
        let ends = recorder.spawn_ends.lock().unwrap();
        assert_eq!(*begins, vec!["crm".to_string()]);
        assert_eq!(*ends, vec!["crm".to_string()]);
    }

    #[tokio::test]
    async fn nested_spawn_agent_tracks_parent_span_id() {
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

        let outer = make_spawn_node("cleo");
        let _ = execute(&ctx, &outer, vec![]).await.unwrap();
        let outer_span = ctx.agent_scope_stack.peek().unwrap().span_id;

        let mut inner = make_spawn_node("crm");
        inner.id = 2;
        let _ = execute(&ctx, &inner, vec![]).await.unwrap();

        assert_eq!(ctx.agent_scope_stack.depth(), 2);
        let top = ctx.agent_scope_stack.peek().unwrap();
        assert_eq!(top.agent_code, "crm");
        assert_eq!(top.parent_span_id.as_deref(), Some(outer_span.as_str()));
    }

    #[tokio::test]
    async fn spawn_agent_can_route_to_acp_profile_without_goal() {
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
        ctx.process_table
            .set_agent_spawner(Arc::new(RoutingTestSpawner {
                candidates: vec![
                    crate::agent_router::AgentRouteCandidate {
                        profile: "reader".to_string(),
                        description: None,
                        source: Some("test".to_string()),
                        executable: "reader".to_string(),
                        capabilities: vec!["read".to_string()],
                        default_mode: None,
                        default_model: None,
                    },
                    crate::agent_router::AgentRouteCandidate {
                        profile: "executor".to_string(),
                        description: None,
                        source: Some("test".to_string()),
                        executable: "executor".to_string(),
                        capabilities: vec!["read".to_string(), "execute".to_string()],
                        default_mode: Some("code".to_string()),
                        default_model: Some("test-model".to_string()),
                    },
                ],
            }))
            .await;

        let mut node = make_spawn_node("worker");
        node.attributes.insert(
            graph_attrs::AGENT_ROUTE.to_string(),
            Value::String("auto".to_string()),
        );
        node.attributes.insert(
            graph_attrs::REQUIRED_CAPABILITIES.to_string(),
            Value::Array(vec![Value::String("execute".to_string())]),
        );

        let result = execute(&ctx, &node, vec![]).await.unwrap();

        let Value::Object(obj) = result else {
            panic!("expected object");
        };
        assert_eq!(
            obj.get(response_keys::PROFILE),
            Some(&Value::String("executor".to_string()))
        );
        assert_eq!(
            obj.get(response_keys::ROUTE_SOURCE),
            Some(&Value::String("selected".to_string()))
        );
        assert_eq!(
            obj.get(response_keys::ROUTE_SELECTOR),
            Some(&Value::String("deterministic".to_string()))
        );
        assert!(matches!(
            obj.get(response_keys::ROUTE_CANDIDATE_SNAPSHOT),
            Some(Value::String(value)) if value.starts_with("fnv1a64:")
        ));
        assert_eq!(
            obj.get(response_keys::ELIGIBLE_PROFILES),
            Some(&Value::Array(vec![Value::String("executor".to_string())]))
        );
        assert_eq!(
            obj.get(response_keys::REJECTED_PROFILES),
            Some(&Value::Array(vec![Value::String("reader".to_string())]))
        );
        let Some(Value::Array(route_scores)) = obj.get(response_keys::ROUTE_SCORES) else {
            panic!("expected route_scores array");
        };
        assert_eq!(route_scores.len(), 2);
        assert!(
            route_scores.iter().any(|score| {
                let Value::Object(fields) = score else {
                    return false;
                };
                fields.get("profile") == Some(&Value::String("executor".to_string()))
                    && fields.get("eligible") == Some(&Value::Bool(true))
            }),
            "expected eligible executor score: {route_scores:?}"
        );
        assert_eq!(
            obj.get(response_keys::MODEL),
            Some(&Value::String("test-model".to_string()))
        );
    }

    #[tokio::test]
    async fn spawn_agent_rejects_invalid_route_mode() {
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
        let mut node = make_spawn_node("worker");
        node.attributes.insert(
            graph_attrs::AGENT_ROUTE.to_string(),
            Value::String("manual".to_string()),
        );

        let error = execute(&ctx, &node, vec![])
            .await
            .expect_err("invalid route mode should fail");

        match error {
            RuntimeError::Operation { message, .. } => {
                assert!(message.contains("agent_route must be 'auto'"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn spawn_agent_rejects_empty_route_candidate_inventory() {
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
        ctx.process_table
            .set_agent_spawner(Arc::new(RoutingTestSpawner {
                candidates: Vec::new(),
            }))
            .await;
        let mut node = make_spawn_node("worker");
        node.attributes.insert(
            graph_attrs::AGENT_ROUTE.to_string(),
            Value::String("auto".to_string()),
        );

        let error = execute(&ctx, &node, vec![])
            .await
            .expect_err("empty route inventory should fail");

        match error {
            RuntimeError::Operation { message, .. } => {
                assert!(message.contains("no APXM agent route candidates"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn spawn_agent_explicit_profile_does_not_require_route_inventory() {
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
        ctx.process_table
            .set_agent_spawner(Arc::new(RoutingTestSpawner {
                candidates: Vec::new(),
            }))
            .await;
        let mut node = make_spawn_node("worker");
        node.attributes.insert(
            graph_attrs::PROFILE.to_string(),
            Value::String("fixture-profile".to_string()),
        );

        let value = execute(&ctx, &node, vec![])
            .await
            .expect("explicit profile should spawn without route inventory");
        let Value::Object(obj) = value else {
            panic!("expected spawn object");
        };
        assert_eq!(
            obj.get(response_keys::PROFILE),
            Some(&Value::String("fixture-profile".to_string()))
        );
        assert!(obj.get(response_keys::ROUTE_SOURCE).is_none());
    }

    #[tokio::test]
    async fn spawn_agent_validates_explicit_profile_capabilities() {
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
        ctx.process_table
            .set_agent_spawner(Arc::new(RoutingTestSpawner {
                candidates: vec![crate::agent_router::AgentRouteCandidate {
                    profile: "reader".to_string(),
                    description: None,
                    source: Some("test".to_string()),
                    executable: "reader".to_string(),
                    capabilities: vec!["read".to_string()],
                    default_mode: None,
                    default_model: None,
                }],
            }))
            .await;
        let mut node = make_spawn_node("worker");
        node.attributes.insert(
            graph_attrs::PROFILE.to_string(),
            Value::String("reader".to_string()),
        );
        node.attributes.insert(
            graph_attrs::REQUIRED_CAPABILITIES.to_string(),
            Value::Array(vec![Value::String("execute".to_string())]),
        );

        let error = execute(&ctx, &node, vec![])
            .await
            .expect_err("capability mismatch should fail");

        match error {
            RuntimeError::Operation { message, .. } => {
                assert!(message.contains("without required capabilities"));
                assert!(message.contains("execute"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn spawn_agent_seeds_routing_with_active_profile_counts() {
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
        ctx.process_table
            .set_agent_spawner(Arc::new(RoutingTestSpawner {
                candidates: vec![
                    crate::agent_router::AgentRouteCandidate {
                        profile: "first".to_string(),
                        description: None,
                        source: Some("test".to_string()),
                        executable: "first".to_string(),
                        capabilities: vec!["read".to_string()],
                        default_mode: None,
                        default_model: None,
                    },
                    crate::agent_router::AgentRouteCandidate {
                        profile: "second".to_string(),
                        description: None,
                        source: Some("test".to_string()),
                        executable: "second".to_string(),
                        capabilities: vec!["read".to_string()],
                        default_mode: None,
                        default_model: None,
                    },
                ],
            }))
            .await;

        let mut first = make_spawn_node("worker_one");
        first.attributes.insert(
            graph_attrs::AGENT_ROUTE.to_string(),
            Value::String("auto".to_string()),
        );
        let mut second = make_spawn_node("worker_two");
        second.id = 2;
        second.attributes.insert(
            graph_attrs::AGENT_ROUTE.to_string(),
            Value::String("auto".to_string()),
        );

        let first_result = execute(&ctx, &first, vec![]).await.unwrap();
        let second_result = execute(&ctx, &second, vec![]).await.unwrap();

        let Value::Object(first_obj) = first_result else {
            panic!("expected object");
        };
        let Value::Object(second_obj) = second_result else {
            panic!("expected object");
        };
        assert_eq!(
            first_obj.get(response_keys::PROFILE),
            Some(&Value::String("first".to_string()))
        );
        assert_eq!(
            second_obj.get(response_keys::PROFILE),
            Some(&Value::String("second".to_string()))
        );
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
