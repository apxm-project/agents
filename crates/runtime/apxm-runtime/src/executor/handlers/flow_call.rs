//! FLOW_CALL operation - Call a flow on another agent

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use crate::aam::{ScopeSpec, TransitionLabel};
use crate::executor::ExecutorEngine;
use crate::executor::handlers::template::input_names_from_node;
use crate::metadata_keys as metadata;
use crate::scheduler::{DataflowScheduler, SchedulerConfig};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::error::RuntimeError;
use apxm_core::types::execution::ExecutionDag;
use apxm_core::types::values::Number;

/// Maximum recursion depth for flow calls to prevent stack overflow
const MAX_FLOW_CALL_DEPTH: usize = 100;

/// Execute a flow call operation
///
/// Invokes a flow on another agent, passing arguments and receiving a result.
/// This enables cross-agent communication and coordination.
///
/// The flow call:
/// 1. Looks up the target flow in the FlowRegistry
/// 2. Creates a child execution context for the sub-flow
/// 3. Executes the sub-flow DAG
/// 4. Returns the result from the sub-flow's exit node
///
/// If the target flow is not registered, returns an error.
///
/// Note: This function returns a boxed future to break the async recursion chain
/// that occurs when a flow calls another flow (flow_call -> execute_dag -> dispatch -> flow_call).
pub fn execute<'a>(
    ctx: &'a ExecutionContext,
    node: &'a Node,
    inputs: Vec<Value>,
) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
    Box::pin(execute_impl(ctx, node, inputs))
}

async fn execute_impl(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Get agent name
    let agent_name = get_string_attribute(node, graph_attrs::AGENT_NAME)?;

    // Get flow name
    let flow_name = get_string_attribute(node, graph_attrs::FLOW_NAME)?;

    tracing::info!(
        agent = %agent_name,
        flow = %flow_name,
        num_args = inputs.len(),
        "Executing flow call"
    );

    // Check recursion depth via metadata
    let current_depth: usize = ctx
        .metadata
        .get(metadata::FLOW_CALL_DEPTH)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if current_depth >= MAX_FLOW_CALL_DEPTH {
        return Err(RuntimeError::Capability {
            capability: format!("flow_call:{}:{}", agent_name, flow_name),
            message: format!(
                "Maximum flow call depth ({}) exceeded. Possible infinite recursion.",
                MAX_FLOW_CALL_DEPTH
            ),
        });
    }

    // Look up the target flow in the FlowRegistry
    let sub_dag = match ctx.flow_registry.get_flow(&agent_name, &flow_name) {
        Some(dag) => dag,
        None => {
            // Flow not registered - return error with available flows
            let available = ctx.flow_registry.list_flows();
            let available_str = if available.is_empty() {
                "No flows registered".to_string()
            } else {
                available
                    .iter()
                    .map(|(a, f)| format!("{}.{}", a, f))
                    .collect::<Vec<_>>()
                    .join(", ")
            };

            return Err(RuntimeError::Capability {
                capability: format!("flow_call:{}:{}", agent_name, flow_name),
                message: format!(
                    "Flow '{}.{}' not found in registry. Available flows: {}",
                    agent_name, flow_name, available_str
                ),
            });
        }
    };

    let resolved_args = resolve_flow_call_args(node, &inputs, &sub_dag)?;

    // Record the flow call in AAM beliefs for tracking.
    let call_request = Value::Object(
        vec![
            ("type".to_string(), Value::String("flow_call".to_string())),
            ("agent".to_string(), Value::String(agent_name.clone())),
            ("flow".to_string(), Value::String(flow_name.clone())),
            ("inputs".to_string(), Value::Array(inputs.clone())),
            (
                "args_by_name".to_string(),
                Value::Object(resolved_args.named_args.clone()),
            ),
            (
                "execution_id".to_string(),
                Value::String(ctx.execution_id.clone()),
            ),
        ]
        .into_iter()
        .collect(),
    );

    let label = TransitionLabel::Custom(format!("flow_call:{}:{}", agent_name, flow_name));
    ctx.aam.set_belief(
        format!(
            "{}{}:{}",
            belief_keys::PENDING_FLOW_CALL_PREFIX,
            agent_name,
            flow_name
        ),
        call_request.clone(),
        label,
    );

    // Record in episodic memory for tracing.
    ctx.memory
        .record_episode(
            format!("flow_call:{}:{}", agent_name, flow_name),
            call_request.clone(),
            ctx.execution_id.clone(),
            Some(node.id),
            ctx.metadata
                .get(metadata::SESSION_DIR)
                .map(std::path::PathBuf::from),
        )
        .await
        .ok();

    tracing::debug!(
        agent = %agent_name,
        flow = %flow_name,
        resolved_args = resolved_args.ordered_inputs.len(),
        nodes = sub_dag.nodes.len(),
        "Found flow in registry, executing sub-DAG"
    );

    let target_agent = ctx.flow_registry.get_agent(&agent_name);

    // Create a child context for the sub-flow execution
    let mut child_ctx = ctx
        .child_with_scope(ScopeSpec::snapshot_all())
        .with_metadata(
            metadata::PARENT_EXECUTION_ID.to_string(),
            ctx.execution_id.clone(),
        )
        .with_metadata(
            metadata::FLOW_CALL_DEPTH.to_string(),
            (current_depth + 1).to_string(),
        )
        .with_metadata(metadata::TARGET_AGENT.to_string(), agent_name.clone())
        .with_metadata(metadata::TARGET_FLOW.to_string(), flow_name.clone());
    if let Some(agent) = target_agent {
        child_ctx = child_ctx.with_agent(agent);
    }

    // Inject resolved named arguments into STM under their declared parameter names.
    for (name, value) in &resolved_args.named_args {
        let _ = child_ctx
            .memory
            .write_scoped(
                crate::memory::MemorySpace::Stm,
                child_ctx.scope_id(),
                name.clone(),
                value.clone(),
            )
            .await;
    }

    // Propagate the child scope_id to the event emitter so emitted events
    // carry the sub-flow's scope for session isolation.
    let child_event_emitter = child_ctx.event_emitter.as_ref().map(Arc::clone);
    let previous_scope_id = if let Some(emitter) = &child_event_emitter {
        let previous_scope_id = emitter.current_scope_id();
        emitter.set_current_scope_id(child_ctx.current_scope_id.clone());
        previous_scope_id
    } else {
        None
    };

    // Execute the sub-flow DAG with real scheduler inputs so compile
    // parameters work for nested flows.
    let engine = Arc::new(ExecutorEngine::new(child_ctx.clone()));
    let scheduler =
        DataflowScheduler::new(SchedulerConfig::default().with_collect_all_outputs(true));
    let dag_to_execute = (*sub_dag).clone();
    let token_accountant = Arc::clone(&child_ctx.token_accountant);
    let child_scope_id = child_ctx.scope_id().to_string();

    let scheduler_result = scheduler
        .execute(
            dag_to_execute,
            engine,
            child_ctx,
            resolved_args.ordered_inputs.clone(),
        )
        .await;

    if let Some(emitter) = &child_event_emitter {
        emitter.set_current_scope_id(previous_scope_id);
    }

    let (results, stats, _scheduler_metrics, all_outputs, node_output_map) = scheduler_result
        .map_err(|e| {
            tracing::error!(
                agent = %agent_name,
                flow = %flow_name,
                error = %e,
                "Sub-flow execution failed"
            );
            RuntimeError::Capability {
                capability: format!("flow_call:{}:{}", agent_name, flow_name),
                message: format!("Sub-flow execution failed: {}", e),
            }
        })?;

    record_child_flow_outputs(
        ctx,
        node.id,
        &agent_name,
        &flow_name,
        &child_scope_id,
        &results,
        all_outputs.as_ref(),
        node_output_map.as_ref(),
    );

    // Clear the pending flow call belief
    ctx.aam.set_belief(
        format!(
            "{}{}:{}",
            belief_keys::PENDING_FLOW_CALL_PREFIX,
            agent_name,
            flow_name
        ),
        Value::Null,
        TransitionLabel::Custom(format!("flow_call_completed:{}:{}", agent_name, flow_name)),
    );

    // Extract the result from the sub-flow's exit nodes
    // If there are multiple exit nodes, we return the first non-null value
    let return_value = results
        .values()
        .find(|v| !matches!(v, Value::Null))
        .cloned()
        .unwrap_or(Value::Null);

    tracing::info!(
        agent = %agent_name,
        flow = %flow_name,
        duration_ms = stats.duration_ms,
        executed_nodes = stats.executed_nodes,
        tokens = token_accountant.snapshot().total.total_tokens,
        "Flow call completed successfully"
    );

    Ok(return_value)
}

fn record_child_flow_outputs(
    ctx: &ExecutionContext,
    parent_node_id: u64,
    agent_name: &str,
    flow_name: &str,
    child_scope_id: &str,
    results: &HashMap<u64, Value>,
    all_outputs: Option<&HashMap<u64, Value>>,
    node_output_map: Option<&HashMap<u64, Vec<u64>>>,
) {
    let evidence = Value::Object(
        vec![
            (
                "type".to_string(),
                Value::String("flow_call_child_outputs".to_string()),
            ),
            (
                "parent_node_id".to_string(),
                Value::Number(Number::Integer(parent_node_id as i64)),
            ),
            ("agent".to_string(), Value::String(agent_name.to_string())),
            ("flow".to_string(), Value::String(flow_name.to_string())),
            (
                "child_scope_id".to_string(),
                Value::String(child_scope_id.to_string()),
            ),
            ("results".to_string(), output_map_to_value(results)),
            (
                "all_outputs".to_string(),
                all_outputs
                    .map(output_map_to_value)
                    .unwrap_or_else(|| Value::Object(HashMap::new())),
            ),
            (
                "node_output_map".to_string(),
                node_output_map
                    .map(node_output_map_to_value)
                    .unwrap_or_else(|| Value::Object(HashMap::new())),
            ),
        ]
        .into_iter()
        .collect(),
    );

    ctx.aam.set_belief(
        format!(
            "{}{}:{}:{}",
            belief_keys::FLOW_CALL_OUTPUT_PREFIX,
            agent_name,
            flow_name,
            parent_node_id
        ),
        evidence,
        TransitionLabel::Custom(format!("flow_call_outputs:{}:{}", agent_name, flow_name)),
    );
}

fn output_map_to_value(outputs: &HashMap<u64, Value>) -> Value {
    Value::Object(
        outputs
            .iter()
            .map(|(node_id, value)| (node_id.to_string(), value.clone()))
            .collect(),
    )
}

fn node_output_map_to_value(output_map: &HashMap<u64, Vec<u64>>) -> Value {
    Value::Object(
        output_map
            .iter()
            .map(|(node_id, output_ids)| {
                (
                    node_id.to_string(),
                    Value::Array(
                        output_ids
                            .iter()
                            .map(|output_id| Value::Number(Number::Integer(*output_id as i64)))
                            .collect(),
                    ),
                )
            })
            .collect(),
    )
}

#[derive(Debug, Clone)]
struct ResolvedFlowCallArgs {
    named_args: HashMap<String, Value>,
    ordered_inputs: Vec<Value>,
}

fn resolve_flow_call_args(
    node: &Node,
    inputs: &[Value],
    sub_dag: &ExecutionDag,
) -> Result<ResolvedFlowCallArgs> {
    let input_names = input_names_from_node(node);
    if !input_names.is_empty() && input_names.len() != inputs.len() {
        return Err(RuntimeError::Capability {
            capability: format!(
                "flow_call:{}:{}",
                get_string_attribute(node, graph_attrs::AGENT_NAME)?,
                get_string_attribute(node, graph_attrs::FLOW_NAME)?
            ),
            message: format!(
                "FLOW_CALL input_names length ({}) does not match inputs length ({})",
                input_names.len(),
                inputs.len()
            ),
        });
    }

    let input_bindings: HashMap<String, Value> = input_names
        .iter()
        .cloned()
        .zip(inputs.iter().cloned())
        .collect();

    let mut named_args = HashMap::new();
    if let Some(raw_args) = node.attributes.get(graph_attrs::ARGS) {
        let raw_args = raw_args
            .as_object()
            .ok_or_else(|| RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("Attribute {} must be an object", graph_attrs::ARGS),
            })?;
        for (name, raw_value) in raw_args {
            named_args.insert(
                name.clone(),
                resolve_argument_value(raw_value, &input_bindings)?,
            );
        }
    } else if !input_bindings.is_empty() {
        named_args.extend(input_bindings.clone());
    }

    let declared_params = &sub_dag.metadata.parameters;
    if !declared_params.is_empty() {
        for param in declared_params {
            if !named_args.contains_key(&param.name)
                && let Some(value) = input_bindings.get(&param.name)
            {
                named_args.insert(param.name.clone(), value.clone());
            }
        }

        if inputs.len() > named_args.len() {
            return Err(RuntimeError::Capability {
                capability: format!(
                    "flow_call:{}:{}",
                    get_string_attribute(node, graph_attrs::AGENT_NAME)?,
                    get_string_attribute(node, graph_attrs::FLOW_NAME)?
                ),
                message:
                    "FLOW_CALL received input values that were not bound by input_names or args"
                        .to_string(),
            });
        }

        let unknown_args: Vec<String> = named_args
            .keys()
            .filter(|name| {
                !declared_params
                    .iter()
                    .any(|param| param.name == name.as_str())
            })
            .cloned()
            .collect();
        if !unknown_args.is_empty() {
            return Err(RuntimeError::Capability {
                capability: format!(
                    "flow_call:{}:{}",
                    get_string_attribute(node, graph_attrs::AGENT_NAME)?,
                    get_string_attribute(node, graph_attrs::FLOW_NAME)?
                ),
                message: format!(
                    "Unknown FLOW_CALL argument(s): {}. Callee expects: {}",
                    unknown_args.join(", "),
                    declared_params
                        .iter()
                        .map(|param| param.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }

        let missing_params: Vec<String> = declared_params
            .iter()
            .filter(|param| !named_args.contains_key(&param.name))
            .map(|param| param.name.clone())
            .collect();
        if !missing_params.is_empty() {
            return Err(RuntimeError::Capability {
                capability: format!(
                    "flow_call:{}:{}",
                    get_string_attribute(node, graph_attrs::AGENT_NAME)?,
                    get_string_attribute(node, graph_attrs::FLOW_NAME)?
                ),
                message: format!(
                    "Missing FLOW_CALL argument(s): {}",
                    missing_params.join(", ")
                ),
            });
        }

        let ordered_inputs = declared_params
            .iter()
            .map(|param| named_args.get(&param.name).cloned().unwrap_or(Value::Null))
            .collect();

        return Ok(ResolvedFlowCallArgs {
            named_args,
            ordered_inputs,
        });
    }

    if inputs.len() > named_args.len() {
        return Err(RuntimeError::Capability {
            capability: format!(
                "flow_call:{}:{}",
                get_string_attribute(node, graph_attrs::AGENT_NAME)?,
                get_string_attribute(node, graph_attrs::FLOW_NAME)?
            ),
            message: "FLOW_CALL received input values that were not bound by input_names or args"
                .to_string(),
        });
    }

    Ok(ResolvedFlowCallArgs {
        named_args,
        ordered_inputs: inputs.to_vec(),
    })
}

fn resolve_argument_value(raw: &Value, input_bindings: &HashMap<String, Value>) -> Result<Value> {
    match raw {
        Value::String(text) => {
            if let Some(binding) = parse_exact_placeholder(text) {
                return input_bindings.get(binding).cloned().ok_or_else(|| {
                    RuntimeError::Executor(format!(
                        "FLOW_CALL argument placeholder '{{{binding}}}' is not present in input_names"
                    ))
                });
            }
            Ok(raw.clone())
        }
        Value::Array(items) => Ok(Value::Array(
            items
                .iter()
                .map(|item| resolve_argument_value(item, input_bindings))
                .collect::<Result<Vec<_>>>()?,
        )),
        Value::Object(map) => Ok(Value::Object(
            map.iter()
                .map(|(key, value)| {
                    resolve_argument_value(value, input_bindings)
                        .map(|resolved| (key.clone(), resolved))
                })
                .collect::<Result<HashMap<_, _>>>()?,
        )),
        _ => Ok(raw.clone()),
    }
}

fn parse_exact_placeholder(value: &str) -> Option<&str> {
    let inner = if let Some(stripped) = value.strip_prefix("{{").and_then(|v| v.strip_suffix("}}"))
    {
        stripped
    } else {
        value.strip_prefix('{')?.strip_suffix('}')?
    };
    if inner.is_empty()
        || !inner
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }
    Some(inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::capability::flow_registry::FlowRegistry;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::constants::runtime::belief_keys;
    use apxm_core::types::{
        execution::{ExecutionDag, FlowParameter, NodeMetadata},
        operations::AISOperationType,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

    fn create_simple_flow_dag() -> ExecutionDag {
        // Create a simple DAG that returns a constant string
        let mut const_node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::ConstStr,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        const_node.attributes.insert(
            graph_attrs::VALUE.to_string(),
            Value::String("hello from sub-flow".to_string()),
        );

        ExecutionDag {
            nodes: vec![const_node],
            edges: vec![],
            entry_nodes: vec![1],
            exit_nodes: vec![1],
            metadata: Default::default(),
        }
    }

    fn create_err_flow_dag() -> ExecutionDag {
        let mut err_node = apxm_core::types::execution::Node {
            id: 2,
            op_type: AISOperationType::Err,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![101],
            metadata: NodeMetadata::default(),
        };
        err_node.attributes.insert(
            graph_attrs::RECOVERY_TEMPLATE.to_string(),
            Value::String("scoped child error".to_string()),
        );

        ExecutionDag {
            nodes: vec![err_node],
            edges: vec![],
            entry_nodes: vec![2],
            exit_nodes: vec![2],
            metadata: Default::default(),
        }
    }

    fn create_parameterized_flow_dag() -> ExecutionDag {
        let mut const_node = apxm_core::types::execution::Node {
            id: 3,
            op_type: AISOperationType::ConstStr,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![102],
            metadata: NodeMetadata::default(),
        };
        const_node.attributes.insert(
            graph_attrs::VALUE.to_string(),
            Value::String("topic={{topic}} audience={{audience}}".to_string()),
        );

        let mut dag = ExecutionDag {
            nodes: vec![const_node],
            edges: vec![],
            entry_nodes: vec![3],
            exit_nodes: vec![3],
            metadata: Default::default(),
        };
        dag.metadata.parameters = vec![
            FlowParameter {
                name: "topic".to_string(),
                type_name: "str".to_string(),
            },
            FlowParameter {
                name: "audience".to_string(),
                type_name: "str".to_string(),
            },
        ];
        dag
    }

    #[tokio::test]
    async fn test_flow_call_with_registered_flow() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        // Register a test flow
        flow_registry.register_flow("TestAgent", "greet", create_simple_flow_dag());

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );
        // Need to use with_inner_plan_support to set flow_registry, or modify the context
        let ctx = ExecutionContext {
            flow_registry,
            ..ctx
        };

        // Create a FLOW_CALL node
        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::FlowCall,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            Value::String("TestAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::FLOW_NAME.to_string(),
            Value::String("greet".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::String("hello from sub-flow".to_string()));

        let evidence_key = format!("{}TestAgent:greet:1", belief_keys::FLOW_CALL_OUTPUT_PREFIX);
        let evidence = ctx
            .aam
            .get_belief(&evidence_key)
            .expect("flow call child output evidence");
        let Value::Object(evidence) = evidence else {
            panic!("flow call evidence should be an object");
        };
        assert_eq!(
            evidence.get("agent"),
            Some(&Value::String("TestAgent".to_string()))
        );
        assert_eq!(
            evidence.get("flow"),
            Some(&Value::String("greet".to_string()))
        );
        let Some(Value::Object(results)) = evidence.get("results") else {
            panic!("flow call results should be an object");
        };
        assert!(
            results
                .values()
                .any(|value| value == &Value::String("hello from sub-flow".to_string()))
        );
        let Some(Value::Object(all_outputs)) = evidence.get("all_outputs") else {
            panic!("flow call all_outputs should be an object");
        };
        assert_eq!(
            all_outputs.get("100"),
            Some(&Value::String("hello from sub-flow".to_string()))
        );
        let Some(Value::Object(node_output_map)) = evidence.get("node_output_map") else {
            panic!("flow call node_output_map should be an object");
        };
        assert_eq!(
            node_output_map.get("1"),
            Some(&Value::Array(vec![Value::Number(Number::Integer(100))]))
        );
    }

    #[tokio::test]
    async fn test_flow_call_executes_in_registered_child_scope() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        flow_registry.register_flow("ScopedAgent", "err_flow", create_err_flow_dag());

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

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::FlowCall,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            Value::String("ScopedAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::FLOW_NAME.to_string(),
            Value::String("err_flow".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(
            result,
            Value::String("Error: scoped child error".to_string())
        );

        let root_beliefs = ctx.aam.beliefs();
        assert!(
            !root_beliefs
                .keys()
                .any(|key| key.starts_with(belief_keys::ERR_PREFIX))
        );

        let child_ids = ctx.scope_registry.children_of(ctx.scope_id());
        assert_eq!(child_ids.len(), 1);
        let child_entry = ctx
            .scope_registry
            .get(&child_ids[0])
            .expect("child scope should be registered");
        assert!(
            child_entry
                .aam
                .beliefs()
                .keys()
                .any(|key| key.starts_with(belief_keys::ERR_PREFIX))
        );
    }

    #[tokio::test]
    async fn test_flow_call_not_found() {
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

        // Create a FLOW_CALL node for a non-existent flow
        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::FlowCall,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            Value::String("NonExistent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::FLOW_NAME.to_string(),
            Value::String("missing".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not found in registry"));
    }

    #[tokio::test]
    async fn test_flow_call_recursion_limit() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        // Register a simple flow
        flow_registry.register_flow("TestAgent", "test", create_simple_flow_dag());

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
        // Simulate being at max depth
        let ctx = ctx.with_metadata(
            metadata::FLOW_CALL_DEPTH.to_string(),
            MAX_FLOW_CALL_DEPTH.to_string(),
        );

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::FlowCall,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            Value::String("TestAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::FLOW_NAME.to_string(),
            Value::String("test".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Maximum flow call depth"));
    }

    #[tokio::test]
    async fn test_flow_call_resolves_named_args_and_child_parameters() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());
        flow_registry.register_flow("Writer", "draft", create_parameterized_flow_dag());

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

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::FlowCall,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            Value::String("Writer".to_string()),
        );
        node.attributes.insert(
            graph_attrs::FLOW_NAME.to_string(),
            Value::String("draft".to_string()),
        );
        node.attributes.insert(
            graph_attrs::INPUT_NAMES.to_string(),
            Value::Array(vec![Value::String("topic".to_string())]),
        );
        node.attributes.insert(
            graph_attrs::ARGS.to_string(),
            Value::Object(HashMap::from([
                ("topic".to_string(), Value::String("{topic}".to_string())),
                (
                    "audience".to_string(),
                    Value::String("engineers".to_string()),
                ),
            ])),
        );

        let result = execute(&ctx, &node, vec![Value::String("apxm".to_string())])
            .await
            .unwrap();
        assert_eq!(
            result,
            Value::String("topic=apxm audience=engineers".to_string())
        );
    }

    #[tokio::test]
    async fn test_flow_call_rejects_unbound_positional_inputs() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());
        flow_registry.register_flow("Writer", "draft", create_parameterized_flow_dag());

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

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::FlowCall,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            Value::String("Writer".to_string()),
        );
        node.attributes.insert(
            graph_attrs::FLOW_NAME.to_string(),
            Value::String("draft".to_string()),
        );

        let result = execute(
            &ctx,
            &node,
            vec![
                Value::String("apxm".to_string()),
                Value::String("engineers".to_string()),
            ],
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not bound"));
    }
}
