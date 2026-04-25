//! Executor engine - Main orchestrator for DAG execution

use super::{
    ExecutionContext, Result, dispatcher::OperationDispatcher, handlers::get_u32_array_attribute,
};
use crate::scheduler::{DataflowScheduler, SchedulerConfig};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{
    GraphMetadata, NodeSpec, PriorityClass,
    execution::{ExecutionDag, ExecutionStats, Node, NodeStatus, OpStatus},
    values::Value,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

/// Executor engine orchestrates DAG execution
pub struct ExecutorEngine {
    context: ExecutionContext,
}

impl ExecutorEngine {
    /// Create a new executor engine with the given context
    pub fn new(context: ExecutionContext) -> Self {
        Self { context }
    }

    /// Execute an operation using the provided context
    pub async fn execute_with_context(
        &self,
        node: &Node,
        inputs: Vec<Value>,
        ctx: &ExecutionContext,
    ) -> Result<OperationOutcome> {
        let value = OperationDispatcher::dispatch(ctx, node, inputs).await?;
        Ok(OperationOutcome { value })
    }

    /// Execute a complete DAG using the dataflow scheduler for parallel execution.
    ///
    /// Delegates to [`DataflowScheduler`] which provides automatic parallelism
    /// via token-based dataflow execution, work stealing, and backpressure.
    /// Falls back to sequential execution only when the scheduler is unavailable.
    pub async fn execute_dag(&self, dag: ExecutionDag) -> Result<ExecutionResult> {
        // Propagate scope_id to the event emitter for session isolation.
        if let Some(emitter) = &self.context.event_emitter {
            emitter.set_current_scope_id(self.context.current_scope_id.clone());
        }

        tracing::info!(
            execution_id = %self.context.execution_id,
            nodes = dag.nodes.len(),
            "Starting DAG execution"
        );

        // Register graph with backends for KV-cache scheduling hints
        let graph_id = self.context.execution_id.clone();
        self.register_graph_metadata(&dag, &graph_id).await;

        let mut result = self.execute_dag_inner(dag).await;

        // Capture graph status from graph-aware backends before releasing pins.
        let vllm_graphs = self
            .context
            .llm_registry
            .pre_release_status_all(&graph_id)
            .await;

        // Release graph from backends (both success and error paths)
        self.context.llm_registry.release_graph_all(&graph_id).await;

        if let Ok(ref mut exec_result) = result {
            exec_result.vllm_graphs = vllm_graphs;
        }

        result
    }

    /// Inner DAG execution logic (parallel with sequential fallback).
    async fn execute_dag_inner(&self, dag: ExecutionDag) -> Result<ExecutionResult> {
        if dag.nodes.len() > 1 {
            match self.execute_dag_parallel(dag.clone()).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    tracing::warn!(
                        execution_id = %self.context.execution_id,
                        error = %e,
                        "Dataflow scheduler failed, falling back to sequential execution"
                    );
                }
            }
        }

        self.execute_dag_sequential(dag).await
    }

    /// Build and register graph metadata with all backends.
    async fn register_graph_metadata(&self, dag: &ExecutionDag, graph_id: &str) {
        let node_specs: Vec<NodeSpec> = dag
            .nodes
            .iter()
            .map(|node| {
                let downstream_nodes = get_u32_array_attribute(node, graph_attrs::DOWNSTREAM_NODES);

                let reuse_group = node
                    .attributes
                    .get(graph_attrs::REUSE_GROUP)
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_string());

                let priority = node.metadata.priority;
                let is_critical_path = priority >= 90;
                let priority_class = Some(if is_critical_path {
                    PriorityClass::CriticalPath
                } else {
                    PriorityClass::Parallel
                });

                NodeSpec {
                    node_id: node.id as u32,
                    node_name: node.metadata.name.clone(),
                    estimated_prompt_tokens: node
                        .attributes
                        .get(graph_attrs::SHARED_PREFIX_EST_TOKENS)
                        .and_then(|v| v.as_u64())
                        .map(|u| u as u32),
                    downstream_nodes,
                    priority_class,
                    reuse_group,
                    is_critical_path,
                }
            })
            .collect();

        let metadata =
            GraphMetadata::new(graph_id, &self.context.execution_id).with_nodes(node_specs);

        if let Ok(metadata_json) = serde_json::to_value(&metadata) {
            self.context
                .llm_registry
                .register_graph_all(metadata_json)
                .await;
        }
    }

    /// Execute a DAG using the dataflow scheduler for automatic parallelism.
    async fn execute_dag_parallel(&self, dag: ExecutionDag) -> Result<ExecutionResult> {
        let config = SchedulerConfig::default();
        let scheduler = DataflowScheduler::new(config);

        let executor = Arc::new(ExecutorEngine::new(self.context.clone()));

        let (results, stats, _scheduler_metrics, _, _) = scheduler
            .execute(dag, executor, self.context.clone(), vec![])
            .await?;

        Ok(ExecutionResult {
            results,
            stats,
            token_snapshot: self.context.token_accountant.snapshot(),
            vllm_graphs: vec![],
        })
    }

    /// Sequential fallback executor for DAGs.
    ///
    /// Processes nodes in order, suitable for single-node DAGs, testing, and
    /// as a fallback when the dataflow scheduler encounters an error.
    async fn execute_dag_sequential(&self, dag: ExecutionDag) -> Result<ExecutionResult> {
        let start_time = std::time::Instant::now();
        let node_statuses = Arc::new(RwLock::new(HashMap::<u64, NodeStatus>::new()));
        let node_results = Arc::new(RwLock::new(HashMap::<u64, Value>::new()));

        for node in &dag.nodes {
            let node_start = std::time::Instant::now();

            // Mark as running
            {
                let mut statuses = node_statuses.write().await;
                statuses.insert(
                    node.id,
                    NodeStatus {
                        node_id: node.id,
                        status: OpStatus::Running,
                        retries: 0,
                        last_error: None,
                        started_at_ms: Some(node_start.elapsed().as_millis()),
                        finished_at_ms: None,
                        duration_ms: None,
                        input_tokens: None,
                        output_tokens: None,
                    },
                );
            }

            // Gather inputs from dependencies
            let mut inputs = Vec::new();
            for input_token_id in &node.input_tokens {
                let results = node_results.read().await;
                if let Some(value) = results.get(input_token_id) {
                    inputs.push(value.clone());
                }
            }

            // Execute operation
            let result = OperationDispatcher::dispatch(&self.context, node, inputs).await;

            let node_end = node_start.elapsed();

            // Store result and update status
            match result {
                Ok(value) => {
                    // Store result for output tokens
                    {
                        let mut results = node_results.write().await;
                        for output_token_id in &node.output_tokens {
                            results.insert(*output_token_id, value.clone());
                        }
                    }
                    if let Some(emitter) = &self.context.event_emitter {
                        emitter.emit_node_output(node.id, &value);
                    }

                    // Mark as completed
                    {
                        let mut statuses = node_statuses.write().await;
                        statuses.insert(
                            node.id,
                            NodeStatus {
                                node_id: node.id,
                                status: OpStatus::Completed,
                                retries: 0,
                                last_error: None,
                                started_at_ms: Some(node_start.elapsed().as_millis()),
                                finished_at_ms: Some(node_end.as_millis()),
                                duration_ms: Some(node_end.as_millis()),
                                input_tokens: None,
                                output_tokens: None,
                            },
                        );
                    }
                }
                Err(e) => {
                    // Mark as failed
                    {
                        let mut statuses = node_statuses.write().await;
                        statuses.insert(
                            node.id,
                            NodeStatus {
                                node_id: node.id,
                                status: OpStatus::Failed,
                                retries: 0,
                                last_error: Some(e.to_string()),
                                started_at_ms: Some(node_start.elapsed().as_millis()),
                                finished_at_ms: Some(node_end.as_millis()),
                                duration_ms: Some(node_end.as_millis()),
                                input_tokens: None,
                                output_tokens: None,
                            },
                        );
                    }

                    tracing::error!(
                        execution_id = %self.context.execution_id,
                        node_id = node.id,
                        task_source_id = ?node.metadata.task_source_id,
                        error = %e,
                        "Node execution failed"
                    );

                    return Err(e);
                }
            }
        }

        // Collect final results from exit nodes
        let mut final_results = HashMap::new();
        {
            let results = node_results.read().await;
            for exit_node_id in &dag.exit_nodes {
                // Find the output token for this exit node
                if let Some(node) = dag.nodes.iter().find(|n| n.id == *exit_node_id) {
                    for output_token_id in &node.output_tokens {
                        if let Some(value) = results.get(output_token_id) {
                            final_results.insert(*output_token_id, value.clone());
                        }
                    }
                }
            }
        }

        // Build execution stats
        let statuses = node_statuses.read().await;
        let stats = ExecutionStats {
            executed_nodes: statuses.len(),
            failed_nodes: statuses
                .values()
                .filter(|s| matches!(s.status, OpStatus::Failed))
                .count(),
            duration_ms: start_time.elapsed().as_millis(),
            node_statuses: statuses.values().cloned().collect(),
        };

        tracing::info!(
            execution_id = %self.context.execution_id,
            duration_ms = stats.duration_ms,
            executed = stats.executed_nodes,
            failed = stats.failed_nodes,
            "DAG execution completed"
        );

        Ok(ExecutionResult {
            results: final_results,
            stats,
            token_snapshot: self.context.token_accountant.snapshot(),
            vllm_graphs: vec![],
        })
    }

    /// Execute a single node (for testing/debugging)
    pub async fn execute_node(
        &self,
        node: &apxm_core::types::execution::Node,
        inputs: Vec<Value>,
    ) -> Result<Value> {
        let outcome = self
            .execute_with_context(node, inputs, &self.context)
            .await?;
        Ok(outcome.value)
    }

    /// Execute a sub-DAG identified by a flow registry label.
    ///
    /// Used by `TRY_CATCH` to drive try/catch branches without circular
    /// dependencies. Creates a child execution context and runs the
    /// referenced sub-DAG, returning the first non-null exit value.
    pub async fn run_subgraph_by_label(&self, label: &str, inputs: Vec<Value>) -> Result<Value> {
        // Labels are stored as "AgentName.flowName" or just "flowName".
        // Search the flow registry for a matching flow.
        let sub_dag = self
            .context
            .flow_registry
            .find_flow_by_label(label)
            .ok_or_else(|| apxm_core::error::RuntimeError::Operation {
                op_type: apxm_core::types::operations::AISOperationType::TryCatch,
                message: format!("Sub-DAG label '{}' not found in flow registry", label),
            })?;

        let child_ctx = self.context.child();

        // Store inputs in STM for sub-DAG entry nodes
        for (i, input) in inputs.iter().enumerate() {
            let _ = child_ctx
                .memory
                .write_scoped(
                    crate::memory::MemorySpace::Stm,
                    child_ctx.scope_id(),
                    format!("flow_arg_{}", i),
                    input.clone(),
                )
                .await;
        }

        let child_engine = ExecutorEngine::new(child_ctx);
        let dag_to_execute = (*sub_dag).clone();
        let result = child_engine.execute_dag(dag_to_execute).await?;

        // Return first non-null exit value
        Ok(result
            .results
            .values()
            .find(|v| !matches!(v, Value::Null))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Get a reference to the execution context.
    pub fn context(&self) -> &ExecutionContext {
        &self.context
    }
}

/// Outcome of a single operation execution
#[derive(Debug, Clone)]
pub struct OperationOutcome {
    pub value: Value,
}

/// Result of DAG execution
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Final results mapped by token ID
    pub results: HashMap<u64, Value>,
    /// Execution statistics
    pub stats: ExecutionStats,
    /// Aggregate token usage collected during execution. Empty if no LLM nodes ran.
    pub token_snapshot: crate::executor::token_accounting::TokenAccountingSnapshot,
    /// vLLM graph status snapshots captured before graph release.
    pub vllm_graphs: Vec<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::{
        execution::Node, execution::NodeMetadata, operations::AISOperationType,
    };
    use std::sync::Arc;

    #[tokio::test]
    async fn test_executor_single_node() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );
        let engine = ExecutorEngine::new(ctx);

        // Create a CONST_STR node
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::ConstStr,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            "value".to_string(),
            Value::String("Hello, World!".to_string()),
        );

        let result = engine.execute_node(&node, vec![]).await.unwrap();
        assert_eq!(result, Value::String("Hello, World!".to_string()));
    }

    #[tokio::test]
    async fn test_executor_simple_dag() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );
        let engine = ExecutorEngine::new(ctx);

        // Create a simple DAG: CONST_STR -> UMEM
        let mut const_node = Node {
            id: 1,
            op_type: AISOperationType::ConstStr,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        const_node
            .attributes
            .insert("value".to_string(), Value::String("test_value".to_string()));

        let mut umem_node = Node {
            id: 2,
            op_type: AISOperationType::UMem,
            attributes: HashMap::new(),
            input_tokens: vec![100],
            output_tokens: vec![101],
            metadata: NodeMetadata::default(),
        };
        umem_node
            .attributes
            .insert("key".to_string(), Value::String("test_key".to_string()));

        let dag = ExecutionDag {
            nodes: vec![const_node, umem_node],
            edges: vec![],
            entry_nodes: vec![1],
            exit_nodes: vec![2],
            metadata: Default::default(),
        };

        let result = engine.execute_dag(dag).await.unwrap();
        assert_eq!(result.stats.executed_nodes, 2);
        assert_eq!(result.stats.failed_nodes, 0);
    }
}
