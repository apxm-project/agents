//! Runtime coordinator - Main entry point for the APxM runtime

use crate::metadata_keys as metadata;
use crate::model_router::{ModelRouter, ModelRouterConfig};
use crate::python_tools;
use crate::python_tools::{PythonToolBridge, PythonToolRegistry};
use crate::sandbox::SandboxRegistry;
use crate::{
    aam::Aam,
    agent_pool::AgentPool,
    capability::{CapabilitySystem, flow_registry::FlowRegistry},
    context_stack::ContextStack,
    dispatch::v1::{
        DispatchFallback, DispatchIrV1, dispatch_ir_accounting_json, evaluate_required_capabilities,
    },
    executor::{
        CancellationToken, ExecutionContext, ExecutionEventEmitter, ExecutionHook,
        ExecutionHookContext, ExecutorEngine, InnerPlanLinker, NoOpLinker, NoOpSkillResolver,
        NoOpWorkflowSpawner, OperationMiddleware, SkillResolver, WorkflowSpawner,
    },
    graph_lifecycle::{BackendGraphLifecycle, graph_dispatch_ir_from_dag},
    memory::{MemoryConfig, MemorySystem},
    process_table::ProcessTable,
    scheduler::{DataflowScheduler, SchedulerConfig, SessionLaneGuard},
};
use apxm_artifact::Artifact;
use apxm_backends::LLMRegistry;
use apxm_core::constants::graph::metadata as graph_meta;
use apxm_core::log_info;
use apxm_core::{
    error::RuntimeError,
    types::{
        BackendGraphCapabilities, GraphStatusSnapshot, OptimizationTarget,
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
    /// Backend graph capability evidence captured for this execution.
    pub backend_graph_capabilities: HashMap<String, BackendGraphCapabilities>,
    /// Runtime-owned Dispatch IR accounting projected to JSON for metrics.
    pub dispatch_ir_metrics: serde_json::Value,
    /// Consumed per-tool call counts for this execution tree. Lets a
    /// host (e.g. the chat REPL) maintain a cross-turn session budget. Empty when
    /// no per-tool budget was set.
    pub tool_call_counts: HashMap<String, usize>,
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
    /// Function-calling tool dispatch controls for LLM tool loops.
    #[serde(default)]
    pub llm_tool_dispatch: LlmToolDispatchConfig,
    /// Optimization target selected by the compiler/driver for this execution.
    ///
    /// Runtime side effects such as synthetic shared-prefix warmup are enabled
    /// only for latency-oriented targets. Artifact-only runs default to
    /// `balanced` unless a caller explicitly constructs a runtime config.
    #[serde(default)]
    pub optimization_target: OptimizationTarget,
    /// Metrics emission tier. `Detailed` enables in-flight observers
    /// (currently per-graph pin-peak polling); `Basic` records steady-state
    /// aggregates only.
    #[serde(default)]
    pub metrics_level: apxm_core::types::MetricsLevel,
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
            llm_tool_dispatch: LlmToolDispatchConfig::default(),
            optimization_target: OptimizationTarget::Balanced,
            metrics_level: apxm_core::types::MetricsLevel::default(),
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

/// Runtime-owned controls for LLM function-calling tool dispatch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LlmToolDispatchConfig {
    pub max_parallel_tool_calls: usize,
}

impl LlmToolDispatchConfig {
    pub const DEFAULT_MAX_PARALLEL_TOOL_CALLS: usize = 8;
    pub const HARD_MAX_PARALLEL_TOOL_CALLS: usize = 64;

    pub fn sanitized_max_parallel_tool_calls(self) -> usize {
        self.max_parallel_tool_calls
            .clamp(1, Self::HARD_MAX_PARALLEL_TOOL_CALLS)
    }
}

impl Default for LlmToolDispatchConfig {
    fn default() -> Self {
        Self {
            max_parallel_tool_calls: Self::DEFAULT_MAX_PARALLEL_TOOL_CALLS,
        }
    }
}

/// APxM Runtime - Main coordinator
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
    skill_resolver: Arc<dyn SkillResolver>,
    instruction_config: apxm_core::InstructionConfig,
    sandbox_registry: Arc<SandboxRegistry>,
    process_table: Arc<ProcessTable>,
    /// Optional ModelRouter for dynamic backend/model selection with circuit breakers.
    model_router: Option<Arc<ModelRouter>>,
    /// Agent warm pool for reusing spawned agent sessions.
    agent_pool: Arc<AgentPool>,
    /// Dispatcher middleware cloned into each execution context.
    middlewares: Vec<Arc<dyn OperationMiddleware>>,
    /// Scheduler lifecycle hooks cloned into each graph execution.
    execution_hooks: Vec<Arc<dyn ExecutionHook>>,
}

impl Runtime {
    /// Create a new runtime with the given configuration
    pub async fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        log_info!("runtime", "Initializing APxM Runtime");

        let memory = Arc::new(
            MemorySystem::new(config.memory_config.clone())
                .await
                .map_err(|e| RuntimeError::State(format!("Failed to initialize memory: {}", e)))?,
        );

        let llm_registry = Arc::new(LLMRegistry::new());

        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));

        let flow_registry = Arc::new(FlowRegistry::new());

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
            skill_resolver: Arc::new(NoOpSkillResolver),
            instruction_config: apxm_core::InstructionConfig::default(),
            sandbox_registry: Arc::new(SandboxRegistry::new()),
            process_table: Arc::new(ProcessTable::new()),
            model_router: None,
            agent_pool,
            middlewares: Vec::new(),
            execution_hooks: Vec::new(),
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
        ctx.skill_resolver = Arc::clone(&self.skill_resolver);
        ctx.dag_splicer = Arc::new(crate::executor::NoOpSplicer);
        ctx.flow_registry = Arc::clone(&self.flow_registry);
        ctx.instruction_config = self.instruction_config.clone();
        ctx.token_budget = self.config.token_budget;
        ctx.warmup_config = self.config.warmup_config.clone();
        ctx.max_parallel_tool_calls = self
            .config
            .llm_tool_dispatch
            .sanitized_max_parallel_tool_calls();
        ctx.optimization_target = self.config.optimization_target;
        ctx.metrics_level = self.config.metrics_level;
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
        // Attach a per-artifact hook registry (lifetime = the python tool bridge,
        // inherited by child contexts). REGISTER_HOOK nodes populate it; the
        // interceptor/middleware/pre-step drivers read it. Empty when the
        // artifact declares no hooks.
        ctx = ctx.with_hook_registry(std::sync::Arc::new(
            crate::executor::hooks::HookRegistry::new(),
        ));
        // Attach the per-session ledger (turn caps / tool budgets / grants),
        // seeded by the server at execution start and keyed by session_id, so
        // the runtime owns per-session limits rather than the host.
        if let Some(sid) = ctx.session_id.clone()
            && let Some(ledger) = crate::executor::session_ledger::get(&sid)
        {
            ctx = ctx.with_session_ledger(ledger);
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

    /// Attach a skill-resolution bridge for the `CALL_SKILL` op.
    ///
    /// The runtime defaults to [`NoOpSkillResolver`], which fails every
    /// `CALL_SKILL` invocation with a `call_skill:<id>` capability error.
    /// Hosts that ship a `SkillLibrary` (or an equivalent manifest catalog)
    /// install their resolver implementation here so the runtime can link
    /// child skills by manifest identity.
    pub fn set_skill_resolver(&mut self, resolver: Arc<dyn SkillResolver>) {
        self.skill_resolver = resolver;
    }

    /// Set the instruction configuration for system prompts.
    ///
    /// LLM handlers read system prompts for ask, think, reason, plan, and
    /// reflect from this config.
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

    /// Append one dispatcher middleware for future executions, preserving the
    /// existing chain. Hosts use this to add guards (token budget, redaction,
    /// approval) without having to re-list the built-in chain.
    pub fn add_middleware(&mut self, middleware: Arc<dyn OperationMiddleware>) {
        self.middlewares.push(middleware);
    }

    /// Replace the scheduler lifecycle hook chain for future executions.
    pub fn set_execution_hooks(&mut self, hooks: Vec<Arc<dyn ExecutionHook>>) {
        self.execution_hooks = hooks;
    }

    /// Append one scheduler lifecycle hook for future executions.
    pub fn add_execution_hook(&mut self, hook: Arc<dyn ExecutionHook>) {
        self.execution_hooks.push(hook);
    }

    /// Get a reference to the sandbox registry.
    pub fn sandbox_registry(&self) -> &SandboxRegistry {
        &self.sandbox_registry
    }

    /// Select an OS-isolating sandbox backend for the python tool/hook worker,
    /// gated on the `APXM_SANDBOX_PYTHON` opt-in so default behavior is
    /// unchanged. Returns `None` when the opt-in is unset or no isolating
    /// backend (e.g. bubblewrap) is available — the worker then runs directly.
    fn python_worker_sandbox(&self) -> Option<Arc<dyn crate::sandbox::SandboxBackend>> {
        if !Self::python_sandbox_required() {
            return None;
        }
        self.sandbox_registry
            .select(crate::sandbox::IsolationLevel::OsLevel)
            .ok()
    }

    /// Whether the operator requires the python worker to be sandboxed
    /// (`APXM_SANDBOX_PYTHON`). When true the worker spawn fails closed if no
    /// OS-isolating backend is available, so the trust gate's isolation
    /// guarantee cannot silently fail open.
    fn python_sandbox_required() -> bool {
        std::env::var_os("APXM_SANDBOX_PYTHON").is_some()
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
        let dispatch_ir =
            graph_dispatch_ir_from_dag(&context.graph_id, &context.execution_id, &dag);
        context.set_dispatch_ir_v1(dispatch_ir.clone());
        let (lifecycles, dispatch_fallbacks) =
            build_graph_lifecycles(&self.llm_registry, &dispatch_ir).await;
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
        let fields_honored = Arc::clone(&context.fields_honored);
        let graph_metrics = Arc::clone(&context.graph_metrics);
        // Shared per-tool call counter: snapshot after execution for
        // the host's cross-turn session budget. Empty unless a budget was set.
        let tool_call_counts = Arc::clone(&context.tool_call_counts);

        // Execute with dataflow scheduler for automatic parallelism
        let hook_context = ExecutionHookContext::new(
            context.execution_id.clone(),
            context.graph_id.clone(),
            self.execution_hooks.clone(),
        );

        // Partial replay (`rerun-from-node`): when the host stamped a replay seed
        // into execution metadata, compute it against this (recompiled) DAG so
        // only `from_node` and its descendants re-execute; the upstream nodes are
        // pre-completed from the prior run's boundary token values.
        let replay_seed = replay_seed_from_metadata(&context, &dag);
        let exec_result = self
            .scheduler
            .execute_with_hooks_and_seed(
                dag,
                executor,
                context,
                vec![],
                hook_context,
                replay_seed.as_ref(),
            )
            .await;

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
        let backend_graph_capabilities = self.llm_registry.graph_capabilities();
        let fields_honored_by_backend = fields_honored.snapshot();
        let dispatch_ir_metrics = dispatch_ir_accounting_json(
            Some(&dispatch_ir),
            &backend_graph_capabilities,
            &graph_status_snapshots,
            &dispatch_fallbacks,
            &fields_honored_by_backend,
        );

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
            backend_graph_capabilities,
            dispatch_ir_metrics,
            tool_call_counts: tool_call_counts
                .lock()
                .map(|m| m.clone())
                .unwrap_or_default(),
        })
    }

    /// Execute a serialized artifact with automatic entry point detection and flow registration.
    pub async fn execute_artifact_auto(
        &self,
        artifact: Artifact,
    ) -> Result<RuntimeExecutionResult, RuntimeError> {
        let python_bridge = python_tool_bridge_from_artifact(
            &artifact,
            self.python_worker_sandbox(),
            Self::python_sandbox_required(),
        )?;
        let entry_dag = find_entry_dag(&artifact)?;

        let agents = reconstruct_agents_from_artifact(&artifact);
        let num_registered_agents = agents.len();
        let num_registered_flows: usize = agents.iter().map(|agent| agent.flows.len()).sum();
        let artifact_flow_registry = Arc::new(FlowRegistry::new());
        for agent in agents {
            log_info!(
                "runtime",
                agent = %agent.name,
                flows = agent.flows.len(),
                "Auto-registering agent from artifact"
            );
            artifact_flow_registry.register_agent(agent);
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
            .with_flow_registry(artifact_flow_registry)
            .with_graph_id(graph_id_from_dag(&entry_dag));
        let dispatch_ir =
            graph_dispatch_ir_from_dag(&context.graph_id, &context.execution_id, &entry_dag);
        context.set_dispatch_ir_v1(dispatch_ir.clone());
        let (lifecycles, dispatch_fallbacks) =
            build_graph_lifecycles(&self.llm_registry, &dispatch_ir).await;
        let executor = Arc::new(ExecutorEngine::new(context.clone()));
        let token_accountant = Arc::clone(&context.token_accountant);
        let fields_honored = Arc::clone(&context.fields_honored);
        let graph_metrics = Arc::clone(&context.graph_metrics);
        // Shared per-tool call counter: snapshot after execution for
        // the host's cross-turn session budget. Empty unless a budget was set.
        let tool_call_counts = Arc::clone(&context.tool_call_counts);
        let hook_context = ExecutionHookContext::new(
            context.execution_id.clone(),
            context.graph_id.clone(),
            self.execution_hooks.clone(),
        );

        let exec_result = self
            .scheduler
            .execute_with_hooks(entry_dag, executor, context, vec![], hook_context)
            .await;

        let graph_status_snapshots = release_graph_lifecycles(&lifecycles).await;

        let (results, stats, scheduler_metrics, all_outputs, node_output_map) = exec_result?;

        let token_snapshot = token_accountant.snapshot();
        let graph_metrics_snapshot = graph_metrics.snapshot();
        let backend_graph_capabilities = self.llm_registry.graph_capabilities();
        let fields_honored_by_backend = fields_honored.snapshot();
        let dispatch_ir_metrics = dispatch_ir_accounting_json(
            Some(&dispatch_ir),
            &backend_graph_capabilities,
            &graph_status_snapshots,
            &dispatch_fallbacks,
            &fields_honored_by_backend,
        );

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
            backend_graph_capabilities,
            dispatch_ir_metrics,
            tool_call_counts: tool_call_counts
                .lock()
                .map(|m| m.clone())
                .unwrap_or_default(),
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
        self.execute_artifact_inner(
            artifact,
            args,
            session_id,
            event_emitter,
            session_dir,
            HashMap::new(),
            None,
            None,
        )
        .await
    }

    /// Execute a top-level artifact, seeding extra execution metadata.
    ///
    /// Identical to [`Self::execute_artifact_with_session_and_emitter`] but
    /// layers `extra_metadata` (e.g. the launching skill's `side_effect_policy`)
    /// onto the root context so CALL_SKILL admission can compare a child against
    /// the real top-level grant rather than the conservative `read_only`
    /// default. This stays a *top-level* entry (no `parent_execution_id`), so
    /// metrics still reset.
    pub async fn execute_artifact_with_session_emitter_and_metadata(
        &self,
        artifact: Artifact,
        args: Vec<String>,
        session_id: Option<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
        extra_metadata: HashMap<String, String>,
    ) -> Result<RuntimeExecutionResult, RuntimeError> {
        self.execute_artifact_inner(
            artifact,
            args,
            session_id,
            event_emitter,
            session_dir,
            extra_metadata,
            None,
            None,
        )
        .await
    }

    /// Execute a top-level artifact, seeding metadata AND pre-resolved per-tool
    /// credentials. The host resolves connection ids to bearer
    /// headers and passes them here; the runtime injects them at the trusted
    /// `invoke_tool` seam so the secret never enters the AIR or the prompt.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_artifact_with_session_emitter_metadata_and_credentials(
        &self,
        artifact: Artifact,
        args: Vec<String>,
        session_id: Option<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
        extra_metadata: HashMap<String, String>,
        tool_credentials: Option<HashMap<String, String>>,
    ) -> Result<RuntimeExecutionResult, RuntimeError> {
        self.execute_artifact_inner(
            artifact,
            args,
            session_id,
            event_emitter,
            session_dir,
            extra_metadata,
            None,
            tool_credentials,
        )
        .await
    }

    /// Execute a top-level artifact with host-owned cancellation and
    /// pre-resolved per-tool credentials.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_artifact_with_session_emitter_metadata_credentials_and_cancellation(
        &self,
        artifact: Artifact,
        args: Vec<String>,
        session_id: Option<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
        extra_metadata: HashMap<String, String>,
        tool_credentials: Option<HashMap<String, String>>,
        cancellation_token: CancellationToken,
    ) -> Result<RuntimeExecutionResult, RuntimeError> {
        self.execute_artifact_inner(
            artifact,
            args,
            session_id,
            event_emitter,
            session_dir,
            extra_metadata,
            Some(cancellation_token),
            tool_credentials,
        )
        .await
    }

    /// Execute a top-level artifact with a host-owned cancellation token.
    ///
    /// Server-managed background workflows use this token so public-execution
    /// cancellation also cancels nested WORKFLOW_SPAWN children and parked wake
    /// handles.
    pub async fn execute_artifact_with_session_emitter_metadata_and_cancellation(
        &self,
        artifact: Artifact,
        args: Vec<String>,
        session_id: Option<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
        extra_metadata: HashMap<String, String>,
        cancellation_token: CancellationToken,
    ) -> Result<RuntimeExecutionResult, RuntimeError> {
        self.execute_artifact_inner(
            artifact,
            args,
            session_id,
            event_emitter,
            session_dir,
            extra_metadata,
            Some(cancellation_token),
            None,
        )
        .await
    }

    /// Execute an artifact as the child of a parent execution.
    ///
    /// `parent_metadata` is layered onto the child context's metadata map so
    /// the child handler sees `call_skill_depth`, `parent_execution_id`, and
    /// `parent_scope_id` from the parent. `CALL_SKILL` uses this entry point
    /// via the [`SkillResolver`] bridge to dispatch child entry DAGs while
    /// preserving the no-widen / depth-limit invariants.
    pub async fn execute_artifact_as_child(
        &self,
        artifact: Artifact,
        args: Vec<String>,
        session_id: Option<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
        parent_metadata: HashMap<String, String>,
    ) -> Result<RuntimeExecutionResult, RuntimeError> {
        self.execute_artifact_inner(
            artifact,
            args,
            session_id,
            event_emitter,
            session_dir,
            parent_metadata,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_artifact_inner(
        &self,
        artifact: Artifact,
        args: Vec<String>,
        session_id: Option<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
        extra_metadata: HashMap<String, String>,
        cancellation_token: Option<CancellationToken>,
        tool_credentials: Option<HashMap<String, String>>,
    ) -> Result<RuntimeExecutionResult, RuntimeError> {
        if !Self::python_sandbox_required() && artifact_has_python_tools_section(&artifact) {
            return Err(RuntimeError::Capability {
                capability: python_tools::CAPABILITY_NAME.to_string(),
                message: "python tool artifacts require APXM_SANDBOX_PYTHON".to_string(),
            });
        }

        let _lane_permit = if let Some(ref sid) = session_id {
            Some(self.session_lane_guard.acquire(sid).await)
        } else {
            None
        };

        let python_bridge = python_tool_bridge_from_artifact(
            &artifact,
            self.python_worker_sandbox(),
            Self::python_sandbox_required(),
        )?;
        let entry_dag = find_entry_dag(&artifact)?;
        let arg_values = bind_args(&entry_dag, args)?;

        let artifact_flow_registry = Arc::new(FlowRegistry::new());
        for agent in reconstruct_agents_from_artifact(&artifact) {
            artifact_flow_registry.register_agent(agent);
        }

        #[cfg(feature = "metrics")]
        if !extra_metadata.contains_key(metadata::PARENT_EXECUTION_ID) {
            // Child executions inherit the parent's accounting; only reset for
            // a top-level entry call so we don't zero out the parent's in-flight
            // metrics. A child is identified by an inherited parent_execution_id
            // (not merely by having any extra metadata — a top-level call may
            // seed e.g. side_effect_policy and is still a fresh entry).
            self.llm_registry.metrics().reset();
        }

        let mut context = self
            .build_context_with_bridge(session_id, event_emitter, session_dir, python_bridge)
            .with_flow_registry(artifact_flow_registry)
            .with_graph_id(graph_id_from_dag(&entry_dag));
        if let Some(cancellation_token) = cancellation_token {
            context = context.with_cancellation_token(cancellation_token);
        }
        for (key, value) in extra_metadata {
            context.metadata.insert(key, value);
        }
        // Seed the per-tool call budget from metadata. The program /
        // request DECLARES the budget as data; enforcement is the trusted
        // `ctx.invoke_tool` seam. Child contexts share the counter, so the bound
        // spans the in-process execution tree (called skills, inner DAGs).
        if let Some(budgets) = context
            .metadata
            .get(metadata::TOOL_CALL_BUDGETS)
            .and_then(|raw| serde_json::from_str::<HashMap<String, usize>>(raw).ok())
        {
            context = context.with_tool_call_budgets(Some(budgets));
        }
        // Pre-resolved per-tool credentials ride a dedicated context
        // field (NOT the metadata map) so the secret is never serialized into the
        // propagated metadata or events; the runtime injects it at `invoke_tool`.
        if tool_credentials.is_some() {
            context = context.with_tool_credentials(tool_credentials);
        }
        let context = context;
        let graph_emitter = context.event_emitter.as_ref().map(Arc::clone);
        let execution_id = context.execution_id.clone();
        let node_count = entry_dag.nodes.len();

        let dispatch_ir =
            graph_dispatch_ir_from_dag(&context.graph_id, &context.execution_id, &entry_dag);
        context.set_dispatch_ir_v1(dispatch_ir.clone());
        let (lifecycles, dispatch_fallbacks) =
            build_graph_lifecycles(&self.llm_registry, &dispatch_ir).await;
        if let Some(emitter) = &graph_emitter {
            emitter.emit_graph_start(&execution_id, node_count);
        }
        let executor = Arc::new(ExecutorEngine::new(context.clone()));
        let token_accountant = Arc::clone(&context.token_accountant);
        let fields_honored = Arc::clone(&context.fields_honored);
        let graph_metrics = Arc::clone(&context.graph_metrics);
        // Shared per-tool call counter: snapshot after execution for
        // the host's cross-turn session budget. Empty unless a budget was set.
        let tool_call_counts = Arc::clone(&context.tool_call_counts);
        let hook_context = ExecutionHookContext::new(
            context.execution_id.clone(),
            context.graph_id.clone(),
            self.execution_hooks.clone(),
        );

        // Partial replay (`rerun-from-node`): when the host stamped a replay seed
        // into execution metadata, only `from_node` and its descendants
        // re-execute; the upstream nodes are pre-completed from the prior run's
        // boundary token values.
        let replay_seed = replay_seed_from_metadata(&context, &entry_dag);
        let exec_result = self
            .scheduler
            .execute_with_hooks_and_seed(
                entry_dag,
                executor,
                context,
                arg_values,
                hook_context,
                replay_seed.as_ref(),
            )
            .await;

        let graph_status_snapshots = release_graph_lifecycles(&lifecycles).await;

        if let Some(emitter) = &graph_emitter {
            emitter.emit_graph_end(&execution_id, node_count, exec_result.is_ok());
        }

        let (results, stats, scheduler_metrics, all_outputs, node_output_map) = exec_result?;

        let token_snapshot = token_accountant.snapshot();
        let graph_metrics_snapshot = graph_metrics.snapshot();
        let backend_graph_capabilities = self.llm_registry.graph_capabilities();
        let fields_honored_by_backend = fields_honored.snapshot();
        let dispatch_ir_metrics = dispatch_ir_accounting_json(
            Some(&dispatch_ir),
            &backend_graph_capabilities,
            &graph_status_snapshots,
            &dispatch_fallbacks,
            &fields_honored_by_backend,
        );

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
            backend_graph_capabilities,
            dispatch_ir_metrics,
            tool_call_counts: tool_call_counts
                .lock()
                .map(|m| m.clone())
                .unwrap_or_default(),
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
    dispatch_ir: &DispatchIrV1,
) -> (Vec<BackendGraphLifecycle>, Vec<DispatchFallback>) {
    let mut lifecycles = Vec::new();
    let mut fallbacks = Vec::new();
    for (backend_name, backend) in registry.find_graph_aware_backends() {
        // Runtime-time capability gating. Before registering the graph,
        // verify the backend declares it can
        // honor every `required` dispatch field. A missing required
        // capability MUST NOT silently proceed (which would silently
        // ship hints the backend will drop); skip registration for
        // that backend and record the fallback in dispatch_ir_metrics.
        let backend_caps = backend.graph_capabilities();
        if let Some(fallback) =
            evaluate_required_capabilities(&backend_name, &backend_caps, dispatch_ir)
        {
            tracing::warn!(
                graph_id = %dispatch_ir.graph.graph_id,
                backend = %backend_name,
                missing_required = ?fallback.missing_required,
                "Backend missing required dispatch capability; falling back \
                 to flat-HTTP dispatch (no graph registration)"
            );
            fallbacks.push(fallback);
            continue;
        }

        match BackendGraphLifecycle::register_dispatch_ir(backend.clone(), dispatch_ir).await {
            Ok(lifecycle) => lifecycles.push(lifecycle),
            Err(e) => {
                tracing::warn!(
                    graph_id = %dispatch_ir.graph.graph_id,
                    backend = %backend_name,
                    error = %e,
                    "Backend register_graph failed; continuing without graph-aware hints"
                );
                // Registration failed at the transport layer (e.g. fork
                // route returned 500). Record this as a fallback too so
                // the claim cannot inherit "we sent X" semantics.
                fallbacks.push(DispatchFallback {
                    backend_name: backend_name.clone(),
                    missing_required: Vec::new(),
                    reason: format!("register_graph transport failure: {e}"),
                });
            }
        }
    }
    (lifecycles, fallbacks)
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

/// Decode a [`crate::scheduler::ReplaySeed`] from execution metadata for a
/// partial replay (`rerun-from-node`). The host stamps `replay_from_node` +
/// `replay_token_values` (JSON `{token_id: Value}`); the seed is computed
/// against the recompiled `dag`. Returns `None` (full run) when the keys are
/// absent, malformed, or `from_node` is not a node in `dag`.
fn replay_seed_from_metadata(
    context: &ExecutionContext,
    dag: &ExecutionDag,
) -> Option<crate::scheduler::ReplaySeed> {
    let seed = crate::scheduler::ReplaySeed::from_metadata(&context.metadata, dag)?;
    log_info!(
        "runtime",
        execution_id = %context.execution_id,
        from_node = seed.from_node,
        replayed = seed.replayed_count(),
        completed = seed.completed_nodes.len(),
        "partial replay: re-executing from_node + descendants only"
    );
    Some(seed)
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

fn artifact_has_python_tools_section(artifact: &Artifact) -> bool {
    artifact
        .sections()
        .iter()
        .any(|section| section.kind == PYTHON_TOOLS_SECTION_KIND)
}

/// Extract a `PythonToolBridge` from an artifact's `python_tools` section, if present.
///
/// The section's `data` field is the UTF-8 JSON array produced by the Python
/// frontend (`tools.json` sidecar format). Returns `Ok(None)` when the artifact
/// has no such section, or `Err` if the section is present but malformed.
fn python_tool_bridge_from_artifact(
    artifact: &Artifact,
    sandbox: Option<Arc<dyn crate::sandbox::SandboxBackend>>,
    sandbox_required: bool,
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
    let bridge = PythonToolBridge::new(registry).with_sandbox(sandbox, sandbox_required);

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

/// True when `dag` is an in-graph conversation loop entry: it contains an
/// AUTONOMOUS `recv` anchor that binds its reserved turn parameter from the
/// server turn-input endpoint at runtime (park/wake), not from launch args.
fn is_turn_input_entry(dag: &ExecutionDag) -> bool {
    dag.nodes.iter().any(|node| {
        node.op_type == apxm_core::types::operations::AISOperationType::Autonomous
            && node
                .attributes
                .get("mode")
                .and_then(|v| v.as_str())
                .map(|m| m == "recv")
                .unwrap_or(false)
    })
}

fn validate_args(dag: &ExecutionDag, args: &[String]) -> Result<(), RuntimeError> {
    let params = &dag.metadata.parameters;
    if args.len() != params.len() {
        // A turn-input (recv) loop entry binds its reserved turn parameter from
        // the server turn-input endpoint at runtime, so launching it with zero
        // args is valid because the recv anchor parks for each user message.
        if args.is_empty() && is_turn_input_entry(dag) {
            return Ok(());
        }
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

fn bind_args(dag: &ExecutionDag, args: Vec<String>) -> Result<Vec<Value>, RuntimeError> {
    validate_args(dag, &args)?;
    if args.is_empty() && is_turn_input_entry(dag) {
        return Ok(Vec::new());
    }
    dag.metadata
        .parameters
        .iter()
        .zip(args)
        .map(|(param, raw)| {
            if param.type_name == "json" {
                let json = serde_json::from_str::<serde_json::Value>(&raw).map_err(|error| {
                    RuntimeError::State(format!(
                        "Flow argument '{}' expects json but received invalid JSON: {error}",
                        param.name
                    ))
                })?;
                Value::try_from(json).map_err(|error| {
                    RuntimeError::State(format!(
                        "Flow argument '{}' could not be converted from JSON: {error}",
                        param.name
                    ))
                })
            } else {
                Ok(Value::String(raw))
            }
        })
        .collect()
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
    use apxm_artifact::{ArtifactMetadata, ArtifactSection};
    use apxm_core::constants::graph::attrs as graph_attrs;
    use apxm_core::types::execution::FlowParameter;
    use apxm_core::types::{AISOperationType, DagMetadata, Node, Value};

    fn artifact(dags: Vec<ExecutionDag>) -> Artifact {
        Artifact::new(
            ArtifactMetadata::new(Some("test".to_string()), "test"),
            dags,
        )
    }

    fn single_node_dag(name: &str, is_entry: bool, mut node: Node) -> ExecutionDag {
        let node_id = node.id;
        if node.output_tokens.is_empty() {
            node.add_output_token(node_id);
        }
        ExecutionDag {
            nodes: vec![node],
            edges: Vec::new(),
            entry_nodes: vec![node_id],
            exit_nodes: vec![node_id],
            metadata: DagMetadata {
                name: Some(name.to_string()),
                is_entry,
                parameters: Vec::new(),
            },
        }
    }

    #[test]
    fn bind_args_parses_json_parameters() {
        let mut dag = single_node_dag("main", true, Node::new(1, AISOperationType::Nop));
        dag.metadata.parameters = vec![FlowParameter {
            name: "data".to_string(),
            type_name: "json".to_string(),
        }];

        let args = bind_args(
            &dag,
            vec![r#"{"event":{"subject":"studio-manual-run"}}"#.to_string()],
        )
        .unwrap();

        let first = args[0].as_object().expect("json arg should bind as object");
        let event = first
            .get("event")
            .and_then(Value::as_object)
            .expect("event object");
        assert_eq!(
            event.get("subject").and_then(Value::as_str),
            Some("studio-manual-run")
        );
    }

    #[tokio::test]
    async fn artifact_flow_registry_is_execution_scoped() {
        let runtime = Runtime::new(RuntimeConfig::in_memory()).await.unwrap();

        let stale_researcher_artifact = artifact(vec![
            single_node_dag("main", true, Node::new(1, AISOperationType::Nop)),
            single_node_dag(
                "researcher.main",
                false,
                Node::new(2, AISOperationType::Nop),
            ),
        ]);
        runtime
            .execute_artifact_with_args(stale_researcher_artifact, Vec::new())
            .await
            .unwrap();
        assert!(
            runtime
                .flow_registry()
                .flows_for_agent("researcher")
                .is_empty()
        );

        let mut spawn = Node::new(1, AISOperationType::SpawnAgent);
        spawn.set_attribute(
            graph_attrs::AGENT_NAME.to_string(),
            Value::String("researcher".to_string()),
        );
        let spawn_artifact = artifact(vec![single_node_dag("main", true, spawn)]);

        runtime
            .execute_artifact_with_args(spawn_artifact, Vec::new())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn python_tool_sections_require_sandbox_flag() {
        let runtime = Runtime::new(RuntimeConfig::in_memory()).await.unwrap();
        let mut artifact = artifact(vec![single_node_dag(
            "main",
            true,
            Node::new(1, AISOperationType::Nop),
        )]);
        artifact.add_section(ArtifactSection {
            kind: python_tools::CAPABILITY_NAME.to_string(),
            data: b"[]".to_vec(),
        });

        let err = runtime
            .execute_artifact_with_args(artifact, Vec::new())
            .await
            .expect_err("python section must fail closed without sandbox opt-in");

        assert!(err.to_string().contains("APXM_SANDBOX_PYTHON"));
    }
}
