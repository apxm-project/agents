//! Runtime orchestrator - Main entry point for the APxM runtime

use crate::model_router::{ModelRouter, ModelRouterConfig};
use crate::python_tools;
use crate::python_tools::{PythonToolBridge, PythonToolRegistry};
use crate::sandbox::SandboxRegistry;
use crate::{
    aam::Aam,
    agent_pool::AgentPool,
    capability::{CapabilitySystem, flow_registry::FlowRegistry},
    context_stack::ContextStack,
    executor::{
        ExecutionContext, ExecutionEventEmitter, ExecutorEngine, InnerPlanLinker, NoOpLinker,
        NoOpWorkflowSpawner, OperationMiddleware, WorkflowSpawner,
    },
    graph_lifecycle::BackendGraphLifecycle,
    memory::{MemoryConfig, MemorySystem},
    process_table::ProcessTable,
    scheduler::{DataflowScheduler, SchedulerConfig, SessionLaneGuard},
};
use apxm_artifact::Artifact;
use apxm_backends::LLMRegistry;
use apxm_core::constants::{graph::metadata as graph_meta, runtime::metadata};
use apxm_core::log_info;
use apxm_core::{
    error::RuntimeError,
    types::{
        GraphStatusSnapshot,
        execution::{Agent, AgentFlow, ExecutionDag, ExecutionStats},
        values::Value,
    },
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path, sync::Arc};

/// Result of DAG execution
#[derive(Debug, Clone)]
pub struct RuntimeExecutionResult {
    /// Final results mapped by token ID
    pub results: HashMap<u64, Value>,
    /// Execution statistics
    pub stats: ExecutionStats,
    /// LLM request metrics (when enabled)
    #[cfg(feature = "metrics")]
    pub llm_metrics: apxm_backends::AggregatedMetrics,
    /// Scheduler overhead metrics
    pub scheduler_metrics: crate::observability::SchedulerMetrics,
    pub all_outputs: Option<HashMap<u64, Value>>,
    pub node_output_map: Option<HashMap<u64, Vec<u64>>>,
    /// Aggregate token usage collected during execution. Empty snapshot if no
    /// LLM nodes ran.
    pub token_snapshot: crate::executor::token_accounting::TokenAccountingSnapshot,
    /// Spawned-agent process and prompt metrics collected during execution.
    pub graph_metrics_snapshot: apxm_core::types::GraphMetricsSnapshot,
    /// Backend graph status snapshots captured before graph release.
    pub graph_status_snapshots: Vec<GraphStatusSnapshot>,
}

/// Runtime configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeConfig {
    /// Memory system configuration
    pub memory_config: MemoryConfig,
    /// Scheduler configuration
    pub scheduler_config: SchedulerConfig,
    /// Optional token budget (total across all LLM requests in one execution).
    pub token_budget: Option<u64>,
    /// Optional session-backed context stack input for prompt enrichment.
    #[serde(default)]
    pub context_stack: Option<crate::context_stack::ContextStackConfig>,
    /// Warmup configuration for shared-prefix optimization.
    #[serde(default)]
    pub warmup_config: crate::executor::WarmupConfig,
}

impl RuntimeConfig {
    /// Create configuration with in-memory LTM (for testing)
    pub fn in_memory() -> Self {
        Self {
            memory_config: MemoryConfig::in_memory_ltm(),
            scheduler_config: SchedulerConfig::default(),
            token_budget: None,
            context_stack: None,
            warmup_config: crate::executor::WarmupConfig::default(),
        }
    }

    /// Set scheduler configuration
    pub fn with_scheduler_config(mut self, config: SchedulerConfig) -> Self {
        self.scheduler_config = config;
        self
    }

    /// Set a global token budget for each execution.
    pub fn with_token_budget(mut self, budget: u64) -> Self {
        self.token_budget = Some(budget);
        self
    }
}

/// APxM Runtime - Main orchestrator
///
/// The runtime coordinates all subsystems and provides the main API
/// for executing APxM programs.
pub struct Runtime {
    config: RuntimeConfig,
    memory: Arc<MemorySystem>,
    llm_registry: Arc<LLMRegistry>,
    capability_system: Arc<CapabilitySystem>,
    flow_registry: Arc<FlowRegistry>,
    aam: Aam,
    scheduler: DataflowScheduler,
    session_lane_guard: SessionLaneGuard,
    inner_plan_linker: Arc<dyn InnerPlanLinker>,
    workflow_spawner: Arc<dyn WorkflowSpawner>,
    instruction_config: apxm_core::InstructionConfig,
    sandbox_registry: Arc<SandboxRegistry>,
    process_table: Arc<ProcessTable>,
    /// Optional ModelRouter for dynamic backend/model selection with circuit breakers.
    model_router: Option<Arc<ModelRouter>>,
    /// Agent warm pool for reusing spawned agent sessions.
    agent_pool: Arc<AgentPool>,
    /// Dispatcher middleware cloned into each execution context.
    middlewares: Vec<Arc<dyn OperationMiddleware>>,
}

impl Runtime {
    /// Create a new runtime with the given configuration
    pub async fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        log_info!("runtime", "Initializing APxM Runtime");

        // Initialize memory system
        let memory = Arc::new(
            MemorySystem::new(config.memory_config.clone())
                .await
                .map_err(|e| RuntimeError::State(format!("Failed to initialize memory: {}", e)))?,
        );

        // Initialize LLM registry
        let llm_registry = Arc::new(LLMRegistry::new());

        // Initialize AAM and capability system
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));

        // NOTE: No default capabilities are registered here.
        // Capabilities should be registered by the executor based on what
        // the workflow declares in its `tools: [...]` list.

        // Initialize flow registry for cross-agent flow calls
        let flow_registry = Arc::new(FlowRegistry::new());

        // Initialize scheduler
        let scheduler = DataflowScheduler::new(config.scheduler_config.clone());

        if config.token_budget.is_none() {
            tracing::warn!(
                "No token budget set. Execution has no token limit. \
                 Set RuntimeConfig::token_budget for cost protection."
            );
        }

        // Initialize agent pool with reasonable defaults:
        // - max 4 idle sessions per profile
        // - 5 minute idle timeout before reclaiming
        let agent_pool = Arc::new(AgentPool::new(4, std::time::Duration::from_secs(300)));

        log_info!("runtime", "APxM Runtime initialized successfully");

        Ok(Self {
            config,
            memory,
            llm_registry,
            capability_system,
            flow_registry,
            aam,
            scheduler,
            session_lane_guard: SessionLaneGuard::new(),
            inner_plan_linker: Arc::new(NoOpLinker),
            workflow_spawner: Arc::new(NoOpWorkflowSpawner),
            instruction_config: apxm_core::InstructionConfig::default(),
            sandbox_registry: Arc::new(SandboxRegistry::new()),
            process_table: Arc::new(ProcessTable::new()),
            model_router: None,
            agent_pool,
            middlewares: Vec::new(),
        })
    }

    fn build_context(
        &self,
        session_id: Option<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
    ) -> ExecutionContext {
        self.build_context_with_bridge(session_id, event_emitter, session_dir, None)
    }

    fn build_context_with_bridge(
        &self,
        session_id: Option<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
        python_tool_bridge: Option<Arc<PythonToolBridge>>,
    ) -> ExecutionContext {
        let mut ctx = ExecutionContext::new(
            Arc::clone(&self.memory),
            Arc::clone(&self.llm_registry),
            Arc::clone(&self.capability_system),
            self.aam.clone(),
        );
        ctx.session_id = session_id;
        ctx.inner_plan_linker = Arc::clone(&self.inner_plan_linker);
        ctx.workflow_spawner = Arc::clone(&self.workflow_spawner);
        ctx.dag_splicer = Arc::new(crate::executor::NoOpSplicer);
        ctx.flow_registry = Arc::clone(&self.flow_registry);
        ctx.instruction_config = self.instruction_config.clone();
        ctx.token_budget = self.config.token_budget;
        ctx.warmup_config = self.config.warmup_config.clone();
        ctx.event_emitter = event_emitter;
        ctx.sandbox_registry = Arc::clone(&self.sandbox_registry);
        ctx.process_table = Arc::clone(&self.process_table);
        ctx.agent_pool = Arc::clone(&self.agent_pool);
        ctx.middlewares = self.middlewares.clone();
        if let Some(dir) = session_dir {
            if let Some(root) = Path::new(&dir).parent() {
                ctx.metadata.insert(
                    metadata::SESSION_ROOT.to_string(),
                    root.to_string_lossy().to_string(),
                );
            }
            ctx.metadata.insert(metadata::SESSION_DIR.to_string(), dir);
        }
        if let Some(ref context_stack) = self.config.context_stack {
            ctx = ctx.with_context_stack(Arc::new(ContextStack::from_config(context_stack)));
        }
        if let Some(ref router) = self.model_router {
            ctx.model_router = Some(Arc::clone(router));
        }
        if let Some(bridge) = python_tool_bridge {
            ctx = ctx.with_python_tool_bridge(bridge);
        }
        ctx
    }

    /// Attach a custom inner plan linker implementation to the runtime.
    pub fn set_inner_plan_linker(&mut self, linker: Arc<dyn InnerPlanLinker>) {
        self.inner_plan_linker = linker;
    }

    /// Attach a workflow-spawn bridge implementation to the runtime.
    pub fn set_workflow_spawner(&mut self, spawner: Arc<dyn WorkflowSpawner>) {
        self.workflow_spawner = spawner;
    }

    /// Set the instruction configuration for system prompts.
    ///
    /// The instruction config is used by LLM handlers to get system prompts
    /// for operations like ask, think, reason, plan, and reflect.
    pub fn set_instruction_config(&mut self, config: apxm_core::InstructionConfig) {
        self.instruction_config = config;
    }

    /// Get the instruction configuration.
    pub fn instruction_config(&self) -> &apxm_core::InstructionConfig {
        &self.instruction_config
    }

    /// Set the sandbox registry.
    ///
    /// Host applications register their [`SandboxBackend`](crate::sandbox::SandboxBackend)
    /// implementations into a [`SandboxRegistry`] and inject it here.
    /// The runtime passes it through to every [`ExecutionContext`].
    ///
    /// LLM operations (ASK, THINK, REASON, etc.) never use the sandbox —
    /// only tool execution (INV, capabilities) does.
    pub fn set_sandbox_registry(&mut self, registry: Arc<SandboxRegistry>) {
        self.sandbox_registry = Arc::clone(&registry);
        // Also propagate to the capability system so that
        // CapabilitySystem::invoke_with_timeout() can route process-spawning
        // capabilities through the sandbox backend.
        self.capability_system.set_sandbox_registry(registry);
    }

    /// Replace the default dispatcher middleware chain for future executions.
    pub fn set_middlewares(&mut self, middlewares: Vec<Arc<dyn OperationMiddleware>>) {
        self.middlewares = middlewares;
    }

    /// Get a reference to the sandbox registry.
    pub fn sandbox_registry(&self) -> &SandboxRegistry {
        &self.sandbox_registry
    }

    /// Attach a ModelRouter to the runtime.
    ///
    /// Once set, every [`ExecutionContext`] built by this runtime will have
    /// the router available, enabling circuit-breaker-aware LLM dispatch.
    pub fn set_model_router(&mut self, router: Arc<ModelRouter>) {
        self.model_router = Some(router);
    }

    /// Build a ModelRouter from the current LLM registry and attach it.
    ///
    /// This is a convenience method for the driver — it creates the router
    /// with default config, loads `~/.apxm/models.toml`, and registers
    /// circuit breakers for all currently registered backends.
    pub fn init_model_router(&mut self, config: ModelRouterConfig) -> Result<(), RuntimeError> {
        let router = ModelRouter::new(Arc::clone(&self.llm_registry), config).map_err(|e| {
            RuntimeError::State(format!("Failed to initialize model router: {}", e))
        })?;
        self.model_router = Some(Arc::new(router));
        tracing::info!("ModelRouter initialized with circuit breakers");
        Ok(())
    }

    /// Get a reference to the model router, if one is attached.
    pub fn model_router(&self) -> Option<&Arc<ModelRouter>> {
        self.model_router.as_ref()
    }

    /// Execute a DAG with parallel dataflow execution
    ///
    /// Note: This method does NOT support inner DAG execution (multi-level planning).
    /// If you need inner DAG support, wrap the Runtime in Arc and use
    /// `execute_with_inner_support()` instead.
    pub async fn execute(&self, dag: ExecutionDag) -> Result<RuntimeExecutionResult, RuntimeError> {
        self.execute_with_event_emitter(dag, None).await
    }

    /// Execute a DAG with an optional per-execution event emitter.
    pub async fn execute_with_event_emitter(
        &self,
        dag: ExecutionDag,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
    ) -> Result<RuntimeExecutionResult, RuntimeError> {
        log_info!(
            "runtime",
            nodes = dag.nodes.len(),
            "Executing DAG with parallel scheduler"
        );

        #[cfg(feature = "metrics")]
        self.llm_registry.metrics().reset();

        // Create execution context
        let context = self
            .build_context(None, event_emitter, None)
            .with_graph_id(graph_id_from_dag(&dag));
        let lifecycles = build_graph_lifecycles(
            &self.llm_registry,
            &dag,
            &context.graph_id,
            &context.execution_id,
        )
        .await;
        let graph_emitter = context.event_emitter.as_ref().map(Arc::clone);
        let execution_id = context.execution_id.clone();
        let node_count = dag.nodes.len();
        if let Some(emitter) = &graph_emitter {
            emitter.emit_graph_start(&execution_id, node_count);
        }

        // Create executor
        let executor = Arc::new(ExecutorEngine::new(context.clone()));

        // Clone context before scheduler takes ownership, so we can snapshot
        // token accounting after execution completes
        let token_accountant = Arc::clone(&context.token_accountant);
        let graph_metrics = Arc::clone(&context.graph_metrics);

        // Execute with dataflow scheduler for automatic parallelism
        let exec_result = self.scheduler.execute(dag, executor, context, vec![]).await;

        let graph_status_snapshots = release_graph_lifecycles(&lifecycles).await;

        if let Some(emitter) = &graph_emitter {
            emitter.emit_graph_end(&execution_id, node_count, exec_result.is_ok());
        }

        let (results, stats, scheduler_metrics, all_outputs, node_output_map) = exec_result?;

        #[cfg(feature = "metrics")]
        let llm_metrics = self.llm_registry.metrics().aggregate();

        // Capture token accounting snapshot after execution completes
        let token_snapshot = token_accountant.snapshot();
        let graph_metrics_snapshot = graph_metrics.snapshot();

        Ok(RuntimeExecutionResult {
            results,
            stats,
            #[cfg(feature = "metrics")]
            llm_metrics,
            scheduler_metrics,
            all_outputs,
            node_output_map,
            token_snapshot,
            graph_metrics_snapshot,
            graph_status_snapshots,
        })
    }

    /// Execute a serialized artifact with automatic entry point detection and flow registration.
    pub async fn execute_artifact_auto(
        &self,
        artifact: Artifact,
    ) -> Result<RuntimeExecutionResult, RuntimeError> {
        let python_bridge = python_tool_bridge_from_artifact(&artifact)?;
        let entry_dag = find_entry_dag(&artifact)?;

        let agents = reconstruct_agents_from_artifact(&artifact);
        let num_registered_agents = agents.len();
        let num_registered_flows: usize = agents.iter().map(|agent| agent.flows.len()).sum();
        for agent in agents {
            log_info!(
                "runtime",
                agent = %agent.name,
                flows = agent.flows.len(),
                "Auto-registering agent from artifact"
            );
            self.flow_registry.register_agent(agent);
        }

        log_info!(
            "runtime",
            name = entry_dag.metadata.name.as_deref().unwrap_or("<unnamed>"),
            num_flows = artifact.dags().len(),
            registered_agents = num_registered_agents,
            "Executing @entry flow with {} registered flow(s)",
            num_registered_flows
        );

        #[cfg(feature = "metrics")]
        self.llm_registry.metrics().reset();

        let context = self
            .build_context_with_bridge(None, None, None, python_bridge)
            .with_graph_id(graph_id_from_dag(&entry_dag));
        let lifecycles = build_graph_lifecycles(
            &self.llm_registry,
            &entry_dag,
            &context.graph_id,
            &context.execution_id,
        )
        .await;
        let executor = Arc::new(ExecutorEngine::new(context.clone()));
        let token_accountant = Arc::clone(&context.token_accountant);
        let graph_metrics = Arc::clone(&context.graph_metrics);
        let exec_result = self
            .scheduler
            .execute(entry_dag, executor, context, vec![])
            .await;

        let graph_status_snapshots = release_graph_lifecycles(&lifecycles).await;

        let (results, stats, scheduler_metrics, all_outputs, node_output_map) = exec_result?;

        let token_snapshot = token_accountant.snapshot();
        let graph_metrics_snapshot = graph_metrics.snapshot();

        Ok(RuntimeExecutionResult {
            results,
            stats,
            #[cfg(feature = "metrics")]
            llm_metrics: self.llm_registry.metrics().aggregate(),
            scheduler_metrics,
            all_outputs,
            node_output_map,
            token_snapshot,
            graph_metrics_snapshot,
            graph_status_snapshots,
        })
    }

    /// Execute an artifact with provided arguments for entry flow parameters.
    pub async fn execute_artifact_with_args(
        &self,
        artifact: Artifact,
        args: Vec<String>,
    ) -> Result<RuntimeExecutionResult, RuntimeError> {
        self.execute_artifact_with_session_and_emitter(artifact, args, None, None, None)
            .await
    }

    /// Execute an artifact with positional arguments and an optional session identifier.
    pub async fn execute_artifact_with_session(
        &self,
        artifact: Artifact,
        args: Vec<String>,
        session_id: Option<String>,
    ) -> Result<RuntimeExecutionResult, RuntimeError> {
        self.execute_artifact_with_session_and_emitter(artifact, args, session_id, None, None)
            .await
    }

    /// Execute an artifact with optional session ID and per-execution event emitter.
    pub async fn execute_artifact_with_session_and_emitter(
        &self,
        artifact: Artifact,
        args: Vec<String>,
        session_id: Option<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
    ) -> Result<RuntimeExecutionResult, RuntimeError> {
        let _lane_permit = if let Some(ref sid) = session_id {
            Some(self.session_lane_guard.acquire(sid).await)
        } else {
            None
        };

        let python_bridge = python_tool_bridge_from_artifact(&artifact)?;
        let entry_dag = find_entry_dag(&artifact)?;
        validate_args(&entry_dag, &args)?;

        for agent in reconstruct_agents_from_artifact(&artifact) {
            self.flow_registry.register_agent(agent);
        }

        let arg_values: Vec<Value> = args.into_iter().map(Value::String).collect();
        #[cfg(feature = "metrics")]
        self.llm_registry.metrics().reset();

        let context = self
            .build_context_with_bridge(session_id, event_emitter, session_dir, python_bridge)
            .with_graph_id(graph_id_from_dag(&entry_dag));
        let graph_emitter = context.event_emitter.as_ref().map(Arc::clone);
        let execution_id = context.execution_id.clone();
        let node_count = entry_dag.nodes.len();

        let lifecycles = build_graph_lifecycles(
            &self.llm_registry,
            &entry_dag,
            &context.graph_id,
            &context.execution_id,
        )
        .await;
        if let Some(emitter) = &graph_emitter {
            emitter.emit_graph_start(&execution_id, node_count);
        }
        let executor = Arc::new(ExecutorEngine::new(context.clone()));
        let token_accountant = Arc::clone(&context.token_accountant);
        let graph_metrics = Arc::clone(&context.graph_metrics);
        let exec_result = self
            .scheduler
            .execute(entry_dag, executor, context, arg_values)
            .await;

        let graph_status_snapshots = release_graph_lifecycles(&lifecycles).await;

        if let Some(emitter) = &graph_emitter {
            emitter.emit_graph_end(&execution_id, node_count, exec_result.is_ok());
        }

        let (results, stats, scheduler_metrics, all_outputs, node_output_map) = exec_result?;

        let token_snapshot = token_accountant.snapshot();
        let graph_metrics_snapshot = graph_metrics.snapshot();

        Ok(RuntimeExecutionResult {
            results,
            stats,
            #[cfg(feature = "metrics")]
            llm_metrics: self.llm_registry.metrics().aggregate(),
            scheduler_metrics,
            all_outputs,
            node_output_map,
            token_snapshot,
            graph_metrics_snapshot,
            graph_status_snapshots,
        })
    }

    pub fn memory(&self) -> &MemorySystem {
        &self.memory
    }

    pub fn llm_registry(&self) -> &LLMRegistry {
        &self.llm_registry
    }

    pub fn llm_registry_arc(&self) -> Arc<LLMRegistry> {
        Arc::clone(&self.llm_registry)
    }

    pub fn capability_system(&self) -> &CapabilitySystem {
        &self.capability_system
    }

    pub fn capability_system_arc(&self) -> Arc<CapabilitySystem> {
        Arc::clone(&self.capability_system)
    }

    pub fn memory_system_arc(&self) -> Arc<MemorySystem> {
        Arc::clone(&self.memory)
    }

    pub fn aam(&self) -> &Aam {
        &self.aam
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn flow_registry(&self) -> &FlowRegistry {
        &self.flow_registry
    }

    pub fn process_table(&self) -> &ProcessTable {
        &self.process_table
    }

    pub fn process_table_arc(&self) -> Arc<ProcessTable> {
        Arc::clone(&self.process_table)
    }

    /// Gracefully shut down the runtime, closing all agent processes.
    ///
    /// This method should be called when execution completes to ensure
    /// all ACP sessions are properly terminated. Without this, agent
    /// processes may leak.
    pub fn shutdown(&self) {
        tracing::info!("Runtime shutting down, closing all agent processes");
        self.process_table.close_all();

        // Shut down the agent pool asynchronously
        let pool = Arc::clone(&self.agent_pool);
        tokio::spawn(async move {
            pool.shutdown().await;
        });
    }

    /// Get a reference to the agent pool.
    pub fn agent_pool(&self) -> &AgentPool {
        &self.agent_pool
    }

    /// Get an Arc reference to the agent pool.
    pub fn agent_pool_arc(&self) -> Arc<AgentPool> {
        Arc::clone(&self.agent_pool)
    }
}

/// Build graph lifecycle guards for every graph-aware backend.
///
/// An empty vector means no registered backend opts into graph extensions.
/// Errors from `register_graph` are demoted to warnings so backend graph
/// control-plane outages do not kill the user's execution.
async fn build_graph_lifecycles(
    registry: &LLMRegistry,
    dag: &ExecutionDag,
    graph_id: &str,
    exec_id: &str,
) -> Vec<BackendGraphLifecycle> {
    let mut lifecycles = Vec::new();
    for (backend_name, backend) in registry.find_graph_aware_backends() {
        match BackendGraphLifecycle::register(
            backend.clone(),
            graph_id.to_string(),
            exec_id.to_string(),
            dag,
        )
        .await
        {
            Ok(lifecycle) => lifecycles.push(lifecycle),
            Err(e) => {
                tracing::warn!(
                    graph_id = %graph_id,
                    backend = %backend_name,
                    error = %e,
                    "Backend register_graph failed; continuing without graph-aware hints"
                );
            }
        }
    }
    lifecycles
}

async fn release_graph_lifecycles(
    lifecycles: &[BackendGraphLifecycle],
) -> Vec<GraphStatusSnapshot> {
    let mut graph_status_snapshots = Vec::new();
    for lifecycle in lifecycles {
        if let Err(e) = lifecycle.release().await {
            tracing::warn!(error = %e, "Backend graph release failed (non-fatal)");
        }
        if let Some(status) = lifecycle.take_status().await {
            graph_status_snapshots.push(status);
        }
    }
    graph_status_snapshots
}

fn graph_id_from_dag(dag: &ExecutionDag) -> String {
    dag.metadata.name.clone().unwrap_or_else(|| {
        format!(
            "{}{}",
            graph_meta::GENERATED_GRAPH_ID_PREFIX,
            uuid::Uuid::new_v4()
        )
    })
}

/// Artifact section kind for Python tool manifests.
const PYTHON_TOOLS_SECTION_KIND: &str = python_tools::CAPABILITY_NAME;

/// Extract a `PythonToolBridge` from an artifact's `python_tools` section, if present.
///
/// The section's `data` field is the UTF-8 JSON array produced by the Python
/// frontend (`tools.json` sidecar format). Returns `Ok(None)` when the artifact
/// has no such section, or `Err` if the section is present but malformed.
fn python_tool_bridge_from_artifact(
    artifact: &Artifact,
) -> Result<Option<Arc<PythonToolBridge>>, RuntimeError> {
    let section = artifact
        .sections()
        .iter()
        .find(|s| s.kind == PYTHON_TOOLS_SECTION_KIND);

    let Some(section) = section else {
        return Ok(None);
    };

    let json = std::str::from_utf8(&section.data).map_err(|e| RuntimeError::Capability {
        capability: python_tools::CAPABILITY_NAME.into(),
        message: format!(
            "{} section is not valid UTF-8: {}",
            PYTHON_TOOLS_SECTION_KIND, e
        ),
    })?;

    let registry = PythonToolRegistry::from_json(json)?;
    let tool_count = registry.len();
    let bridge = PythonToolBridge::new(registry);

    log_info!(
        "runtime",
        tools = tool_count,
        "Loaded Python tool bridge from artifact ({} tool(s))",
        tool_count
    );

    Ok(Some(Arc::new(bridge)))
}

fn find_entry_dag(artifact: &Artifact) -> Result<ExecutionDag, RuntimeError> {
    artifact.entry_dag().cloned().ok_or_else(|| {
        let name = artifact
            .dags()
            .first()
            .and_then(|d| d.metadata.name.as_deref())
            .unwrap_or("<unnamed>");
        RuntimeError::State(format!(
            "No @entry flow found in artifact '{}'. Mark a flow with @entry to designate the entry point.",
            name
        ))
    })
}

fn validate_args(dag: &ExecutionDag, args: &[String]) -> Result<(), RuntimeError> {
    let params = &dag.metadata.parameters;
    if args.len() != params.len() {
        let flow_name = dag.metadata.name.as_deref().unwrap_or("<unnamed>");
        let param_desc = if params.is_empty() {
            "no parameters".to_string()
        } else {
            params
                .iter()
                .map(|p| format!("{}: {}", p.name, p.type_name))
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Err(RuntimeError::State(format!(
            "Flow '{}' requires {} argument(s) ({}) but {} were provided",
            flow_name,
            params.len(),
            param_desc,
            args.len()
        )));
    }
    Ok(())
}

fn reconstruct_agents_from_artifact(artifact: &Artifact) -> Vec<Agent> {
    let mut agents: HashMap<String, Agent> = HashMap::new();

    for (index, dag) in artifact.dags().iter().enumerate() {
        let fallback_name = format!("default.flow_{index}");
        let dag_name = dag.metadata.name.as_deref().unwrap_or(&fallback_name);
        let (agent_name, flow_name) = parse_flow_name(dag_name);

        let flow = AgentFlow {
            name: flow_name.clone(),
            is_entry: dag.metadata.is_entry,
            parameters: dag.metadata.parameters.clone(),
            task_dag: None,
            execution_dag: dag.clone(),
        };

        agents
            .entry(agent_name.clone())
            .or_insert_with(|| Agent::new(agent_name))
            .flows
            .insert(flow_name, flow);
    }

    agents.into_values().collect()
}

/// Parse flow name in "Agent.flow" format
fn parse_flow_name(name: &str) -> (String, String) {
    if let Some((agent, flow)) = name.split_once('.') {
        (agent.to_string(), flow.to_string())
    } else {
        ("default".to_string(), name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_artifact::ArtifactMetadata;
    use apxm_core::types::{
        execution::{Node, NodeMetadata},
        operations::AISOperationType,
        values::Value,
    };
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_runtime_creation() {
        let config = RuntimeConfig::in_memory();
        let runtime = Runtime::new(config).await.unwrap();

        assert!(runtime.memory().stm().len().await == 0);
    }

    #[tokio::test]
    async fn test_runtime_execute_simple_dag() {
        let config = RuntimeConfig::in_memory();
        let runtime = Runtime::new(config).await.unwrap();

        // Create a simple DAG with one CONST_STR node
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::ConstStr,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            "value".to_string(),
            Value::String("Hello from runtime!".to_string()),
        );

        let dag = ExecutionDag {
            nodes: vec![node],
            edges: vec![],
            entry_nodes: vec![1],
            exit_nodes: vec![1],
            metadata: Default::default(),
        };

        let result = runtime.execute(dag).await.unwrap();

        assert_eq!(result.stats.executed_nodes, 1);
        assert_eq!(result.stats.failed_nodes, 0);
        assert!(result.results.contains_key(&100));
    }

    #[tokio::test]
    async fn test_runtime_memory_persistence() {
        let config = RuntimeConfig::in_memory();
        let runtime = Runtime::new(config).await.unwrap();

        // Write to memory
        runtime
            .memory()
            .write(
                crate::memory::MemorySpace::Stm,
                "test_key".to_string(),
                Value::String("test_value".to_string()),
            )
            .await
            .unwrap();

        // Read back
        let result = runtime
            .memory()
            .read(crate::memory::MemorySpace::Stm, "test_key")
            .await
            .unwrap();

        assert_eq!(result, Some(Value::String("test_value".to_string())));
    }

    #[tokio::test]
    async fn test_build_context_derives_session_root_from_session_dir() {
        let runtime = Runtime::new(RuntimeConfig::in_memory()).await.unwrap();
        let ctx = runtime.build_context(None, None, Some("/tmp/apxm-sessions/run-1".to_string()));

        assert_eq!(
            ctx.metadata.get(metadata::SESSION_ROOT).map(String::as_str),
            Some("/tmp/apxm-sessions")
        );
        assert_eq!(
            ctx.metadata.get(metadata::SESSION_DIR).map(String::as_str),
            Some("/tmp/apxm-sessions/run-1")
        );
    }

    #[test]
    fn test_reconstruct_agents_from_artifact_groups_flows() {
        let mut research_main = ExecutionDag::new();
        research_main.metadata.name = Some("Research.main".to_string());
        research_main.metadata.is_entry = true;

        let mut research_lookup = ExecutionDag::new();
        research_lookup.metadata.name = Some("Research.lookup".to_string());

        let mut writer_compose = ExecutionDag::new();
        writer_compose.metadata.name = Some("Writer.compose".to_string());

        let artifact = Artifact::new(
            ArtifactMetadata::new(Some("multi-agent".to_string()), "test"),
            vec![research_main, research_lookup, writer_compose],
        );

        let mut agents = reconstruct_agents_from_artifact(&artifact);
        agents.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(agents.len(), 2);

        let research = agents
            .iter()
            .find(|agent| agent.name == "Research")
            .expect("Research agent should be present");
        assert!(research.get_flow("main").is_some());
        assert!(research.get_flow("lookup").is_some());
        assert_eq!(
            research.entry_flow().map(|flow| flow.name.as_str()),
            Some("main")
        );

        let writer = agents
            .iter()
            .find(|agent| agent.name == "Writer")
            .expect("Writer agent should be present");
        assert!(writer.get_flow("compose").is_some());
    }

    #[test]
    fn test_python_tool_bridge_from_artifact_no_section() {
        let mut dag = ExecutionDag::new();
        dag.metadata.is_entry = true;
        dag.metadata.name = Some("test.main".into());

        let artifact = Artifact::new(
            ArtifactMetadata::new(Some("test".into()), "test"),
            vec![dag],
        );

        // No python_tools section → returns None
        let bridge = python_tool_bridge_from_artifact(&artifact).unwrap();
        assert!(bridge.is_none());
    }

    #[test]
    fn test_python_tool_bridge_from_artifact_with_section() {
        use apxm_artifact::ArtifactSection;

        let tools_json = r#"[{
            "handler_id": "sha256:abc123",
            "module": "mytools",
            "qualname": "add",
            "name": "add",
            "schema": {"type": "object"}
        }]"#;

        let mut dag = ExecutionDag::new();
        dag.metadata.is_entry = true;
        dag.metadata.name = Some("test.main".into());

        let mut artifact = Artifact::new(
            ArtifactMetadata::new(Some("test".into()), "test"),
            vec![dag],
        );
        artifact.add_section(ArtifactSection {
            kind: PYTHON_TOOLS_SECTION_KIND.to_string(),
            data: tools_json.as_bytes().to_vec(),
        });

        // Roundtrip through serialization to verify section survives
        let bytes = artifact.to_bytes().unwrap();
        let loaded = Artifact::from_bytes(&bytes).unwrap();

        let bridge = python_tool_bridge_from_artifact(&loaded).unwrap();
        assert!(bridge.is_some());
        let bridge = bridge.unwrap();
        assert!(bridge.has_tool("add"));
        assert!(!bridge.has_tool("subtract"));
    }

    #[test]
    fn test_python_tool_bridge_from_artifact_invalid_section() {
        use apxm_artifact::ArtifactSection;

        let mut dag = ExecutionDag::new();
        dag.metadata.is_entry = true;
        dag.metadata.name = Some("test.main".into());

        let mut artifact = Artifact::new(
            ArtifactMetadata::new(Some("test".into()), "test"),
            vec![dag],
        );
        artifact.add_section(ArtifactSection {
            kind: PYTHON_TOOLS_SECTION_KIND.to_string(),
            data: b"not valid json".to_vec(),
        });

        let result = python_tool_bridge_from_artifact(&artifact);
        assert!(result.is_err());
    }
}
