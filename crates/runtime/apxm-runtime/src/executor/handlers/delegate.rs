//! DELEGATE operation - Delegate a task to a sub-agent
//!
//! Creates a sub-task and assigns it to a target agent. The target agent
//! executes its flow and returns the result. Returns an object with
//! task_handle and result fields.
//!
//! ## Attributes
//! - `task_spec`     (required): description of the task to delegate
//! - `target_agent`  (required): name of the agent to delegate to

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use crate::aam::{ScopeSpec, TransitionLabel};
use crate::executor::ExecutorEngine;
use crate::flow_names::DELEGATE_FLOWS as DELEGATE_FLOW_NAMES;
use crate::metadata_keys as metadata;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::{belief_keys, response_keys};
use apxm_core::error::RuntimeError;
use std::collections::HashMap;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let task_spec = get_string_attribute(node, graph_attrs::TASK_SPEC)?;
    let target_agent = get_string_attribute(node, graph_attrs::TARGET_AGENT)?;

    tracing::info!(
        execution_id = %ctx.execution_id,
        target_agent = %target_agent,
        task_spec = %task_spec,
        "Executing DELEGATE operation"
    );

    // Record delegation in AAM
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::DELEGATE_PREFIX, target_agent),
        Value::String(task_spec.clone()),
        TransitionLabel::Custom(format!("delegate:{}", target_agent)),
    );

    // Look up the target agent's flow in the FlowRegistry
    let sub_dag = {
        let mut found = None;
        for flow_name in DELEGATE_FLOW_NAMES {
            if let Some(dag) = ctx.flow_registry.get_flow(&target_agent, flow_name) {
                found = Some(dag);
                break;
            }
        }
        match found {
            Some(dag) => dag,
            None => {
                return Err(RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!(
                        "Agent '{}' not found or has no 'delegate'/'main' flow",
                        target_agent
                    ),
                });
            }
        }
    };

    // Create a child context for sub-flow execution
    let child_ctx = ctx
        .child_with_scope(ScopeSpec::snapshot_all())
        .with_metadata(
            metadata::PARENT_EXECUTION_ID.to_string(),
            ctx.execution_id.clone(),
        )
        .with_metadata(metadata::DELEGATE_TASK_SPEC.to_string(), task_spec.clone())
        .with_metadata(metadata::DELEGATE_TARGET.to_string(), target_agent.clone());

    // Inject the task spec and any input into STM
    let _ = child_ctx
        .memory
        .write_scoped(
            crate::memory::MemorySpace::Stm,
            child_ctx.scope_id(),
            belief_keys::DELEGATE_TASK_SPEC.to_string(),
            Value::String(task_spec.clone()),
        )
        .await;

    if let Some(input) = inputs.first() {
        let _ = child_ctx
            .memory
            .write_scoped(
                crate::memory::MemorySpace::Stm,
                child_ctx.scope_id(),
                belief_keys::DELEGATE_INPUT.to_string(),
                input.clone(),
            )
            .await;
    }

    // Execute the sub-flow DAG
    let engine = ExecutorEngine::new(child_ctx);
    let dag_to_execute = (*sub_dag).clone();

    let result = engine.execute_dag(dag_to_execute).await.map_err(|e| {
        tracing::error!(
            target_agent = %target_agent,
            error = %e,
            "DELEGATE sub-flow execution failed"
        );
        RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Delegation to agent '{}' failed: {}", target_agent, e),
        }
    })?;

    // Extract the response from the sub-flow's exit nodes
    let response = result
        .results
        .values()
        .find(|v| !matches!(v, Value::Null))
        .cloned()
        .unwrap_or(Value::Null);

    // Clear pending delegation belief
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::DELEGATE_PREFIX, target_agent),
        Value::Null,
        TransitionLabel::Custom(format!("delegate_completed:{}", target_agent)),
    );

    // Return result with task handle metadata
    let task_handle = format!("delegate_{}_{}", target_agent, ctx.execution_id);
    let mut result_obj = HashMap::new();
    result_obj.insert(
        response_keys::TASK_HANDLE.to_string(),
        Value::String(task_handle),
    );
    result_obj.insert(response_keys::RESULT.to_string(), response);

    tracing::info!(
        execution_id = %ctx.execution_id,
        target_agent = %target_agent,
        "DELEGATE completed successfully"
    );

    let result = Value::Object(result_obj);

    // Emit node output for session recording
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_node_output_with_name(node.id, node.metadata.name.as_deref(), &result);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::capability::flow_registry::FlowRegistry;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::constants::graph::attrs as graph_attrs;
    use apxm_core::constants::runtime::response_keys;
    use apxm_core::types::{
        execution::{ExecutionDag, NodeMetadata},
        operations::AISOperationType,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

    /// Create a simple sub-flow DAG that returns a constant string.
    fn create_delegate_flow_dag() -> ExecutionDag {
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
            Value::String("delegated result".to_string()),
        );

        ExecutionDag {
            nodes: vec![const_node],
            edges: vec![],
            entry_nodes: vec![1],
            exit_nodes: vec![1],
            metadata: Default::default(),
        }
    }

    fn make_delegate_node(
        target_agent: &str,
        task_spec: &str,
    ) -> apxm_core::types::execution::Node {
        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Delegate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::TASK_SPEC.to_string(),
            Value::String(task_spec.to_string()),
        );
        node.attributes.insert(
            graph_attrs::TARGET_AGENT.to_string(),
            Value::String(target_agent.to_string()),
        );
        node
    }

    #[tokio::test]
    async fn test_delegate_success_with_sub_flow() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        // Register a "delegate" flow for the target agent
        flow_registry.register_flow("worker", "delegate", create_delegate_flow_dag());

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

        let node = make_delegate_node("worker", "summarize the report");
        let result = execute(&ctx, &node, vec![]).await.unwrap();

        // Result should be an object with task_handle and result
        match &result {
            Value::Object(obj) => {
                assert!(obj.contains_key(response_keys::TASK_HANDLE));
                let task_handle = obj.get(response_keys::TASK_HANDLE).unwrap();
                assert!(task_handle.as_string().unwrap().contains("worker"));

                assert!(obj.contains_key(response_keys::RESULT));
                assert_eq!(
                    obj.get(response_keys::RESULT).unwrap(),
                    &Value::String("delegated result".to_string())
                );
            }
            _ => panic!("Expected Value::Object, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_delegate_uses_main_flow_fallback() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        // Register a "main" flow (not "delegate") - should still be found via fallback
        flow_registry.register_flow("worker", "main", create_delegate_flow_dag());

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

        let node = make_delegate_node("worker", "do some work");
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_ok(), "Should succeed with 'main' flow fallback");
    }

    #[tokio::test]
    async fn test_delegate_agent_not_found() {
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

        let node = make_delegate_node("nonexistent_agent", "do something");
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_delegate_missing_task_spec() {
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

        // Node without task_spec attribute
        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Delegate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::TARGET_AGENT.to_string(),
            Value::String("worker".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("task_spec"));
    }

    #[tokio::test]
    async fn test_delegate_records_aam_belief() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        flow_registry.register_flow("worker", "delegate", create_delegate_flow_dag());

        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());
        let ctx = ExecutionContext {
            flow_registry,
            ..ctx
        };

        let node = make_delegate_node("worker", "test task");
        let _ = execute(&ctx, &node, vec![]).await.unwrap();

        // After completion the delegate belief should be cleared (set to Null)
        let beliefs = ctx.aam.beliefs();
        let delegate_key = format!("{}worker", belief_keys::DELEGATE_PREFIX);
        let belief = beliefs.get(&delegate_key);
        assert!(
            belief.is_none() || matches!(belief, Some(Value::Null)),
            "Delegate belief should be cleared after completion"
        );
    }
}
