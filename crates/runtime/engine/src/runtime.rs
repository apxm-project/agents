//! Runtime coordinator - Main entry point for the APxM runtime

use crate::metadata_keys as metadata;
use crate::model_router::{ModelRouter, ModelRouterConfig, ProfileRegistry};
use crate::python_tools::{PythonHandlerBridge, PythonHandlerRegistry};
use crate::sandbox::SandboxRegistry;
use crate::typescript_tools::{TypeScriptHandlerBridge, TypeScriptHandlerRegistry};
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
        BackendGraphCapabilities, GraphStatusSnapshot, HANDLER_MANIFEST_ARTIFACT_SECTION,
        HandlerManifest, OptimizationTarget,
        execution::{Agent, AgentFlow, ExecutionDag, ExecutionStats},
        values::Value,
    },
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path, sync::Arc};

/// Outcome of [`Runtime::execute_artifact_with_session_emitter_and_metadata_or_park`]
/// ( narrow park observability): either the artifact ran to completion, or
/// a node parked on the conversation-loop's session-recv key before that —
/// whichever happened first.
///
/// `Parked` does not mean the execution stopped: the DAG keeps running in the
/// background (see [`crate::scheduler::SchedulerOutcome::Parked`]) so the
/// session loop keeps making progress; this variant only lets the caller stop
/// waiting once "the next turn boundary was reached" is known, instead of
/// blocking for the lifetime of the session.
pub enum ExecutionOutcome {
    Completed(RuntimeExecutionResult),
    Parked {
        session_id: String,
        /// Handle to the detached background task that keeps running the DAG
        /// to its eventual real completion (or host cancellation) and
        /// releases runtime-owned per-execution resources (backend graph
        /// lifecycle) when that happens.
        ///
        /// A caller with its OWN per-execution bookkeeping that must not
        /// finalize until the execution is truly done (e.g. the server's
        /// cross-execution admission slot, which the in-graph conversation
        /// loop parks and un-parks repeatedly across turns) MUST chain onto
        /// this handle rather than finalizing immediately on `Parked` — the
        /// execution is NOT done just because it parked once.
        background: tokio::task::JoinHandle<()>,
    },
}

impl std::fmt::Debug for ExecutionOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Completed(result) => f.debug_tuple("Completed").field(result).finish(),
            Self::Parked { session_id, .. } => f
                .debug_struct("Parked")
                .field("session_id", session_id)
                .finish_non_exhaustive(),
        }
    }
}

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
    /// Package-level default `model_profile`, sourced from the
    /// owning package's `agent.toml [runtime].default_model_profile` by the
    /// host loading the package. Applied to LLM requests whose node declares
    /// no `model_profile` of its own; an explicit node-level `model_profile`
    /// (or `model`/`backend`) always overrides this default.
    #[serde(default)]
    pub default_model_profile: Option<String>,
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
            default_model_profile: None,
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

    /// Set the package-level default `model_profile`.
    pub fn with_default_model_profile(mut self, profile: impl Into<String>) -> Self {
        self.default_model_profile = Some(profile.into());
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
    /// Optional ProfileRegistry, instantiated beside `model_router`,
    /// resolving `model_profile` node attributes into candidate models.
    profile_registry: Option<Arc<ProfileRegistry>>,
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
            profile_registry: None,
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
        self.build_context_with_bridge(session_id, event_emitter, session_dir, None, None)
    }

    fn build_context_with_bridge(
        &self,
        session_id: Option<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
        python_handler_bridge: Option<Arc<PythonHandlerBridge>>,
        typescript_handler_bridge: Option<Arc<TypeScriptHandlerBridge>>,
    ) -> ExecutionContext {
        // Bound to a concrete `Arc<CapabilitySystem>` local first: passing
        // `Arc::clone(&self.capability_system)` directly would fix `Arc::clone`'s
        // generic `T` from the call's expected `Arc<dyn CapabilityFacade>` type
        // before checking the argument, which fails to unify with the actual
        // `&Arc<CapabilitySystem>` receiver. Binding the concrete type first,
        // then passing that binding, lets the unsize coercion apply normally
        // at the `ExecutionContext::new` argument position.
        let capability_system: Arc<CapabilitySystem> = Arc::clone(&self.capability_system);
        let mut ctx = ExecutionContext::new(
            Arc::clone(&self.memory),
            Arc::clone(&self.llm_registry),
            capability_system,
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
        ctx.scheduler_config = self.config.scheduler_config.clone();
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
        if let Some(ref registry) = self.profile_registry {
            ctx.profile_registry = Some(Arc::clone(registry));
        }
        ctx.default_model_profile = self.config.default_model_profile.clone();
        if let Some(bridge) = python_handler_bridge {
            ctx = ctx.with_python_handler_bridge(bridge);
        }
        if let Some(bridge) = typescript_handler_bridge {
            ctx = ctx.with_typescript_handler_bridge(bridge);
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

    /// Select an OS-isolating sandbox backend for the python/typescript
    /// tool/hook worker, gated on the `APXM_SANDBOX_PYTHON` opt-in so default
    /// behavior is unchanged. Returns `None` when the opt-in is unset or no
    /// isolating backend (e.g. bubblewrap) is available — the worker then
    /// runs directly.
    fn python_worker_sandbox(&self) -> Option<Arc<dyn crate::sandbox::SandboxBackend>> {
        if !Self::script_sandbox_required() {
            return None;
        }
        self.sandbox_registry
            .select(crate::sandbox::IsolationLevel::OsLevel)
            .ok()
    }

    /// Whether the operator requires script workers (Python or TypeScript)
    /// to be sandboxed (`APXM_SANDBOX_PYTHON`). When true the worker spawn
    /// fails closed if no OS-isolating backend is available, so the trust
    /// gate's isolation guarantee cannot silently fail open. Delegates to
    /// the shared [`crate::script_admission`] policy so this crate, the
    /// driver's attach step, and the server's admission all read one
    /// definition.
    fn script_sandbox_required() -> bool {
        crate::script_admission::script_sandbox_required()
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

    /// Attach a ProfileRegistry to the runtime.
    ///
    /// Once set, every [`ExecutionContext`] built by this runtime will have
    /// the registry available, letting the LLM dispatch path resolve a
    /// node's `model_profile` attribute into a candidate model — via a
    /// transient `ProfileRouter` built alongside `model_router` — before
    /// `ModelRouter::select` runs.
    pub fn set_profile_registry(&mut self, registry: Arc<ProfileRegistry>) {
        self.profile_registry = Some(registry);
    }

    /// Load `~/.apxm/model_profiles.toml` and attach the registry.
    ///
    /// Convenience method for model profile setup — call after LLM
    /// backends and the model router are configured so profile resolution is
    /// available from the first request. A missing config file yields an
    /// empty (inert) registry rather than an error.
    pub fn init_profile_registry(&mut self) {
        self.profile_registry = Some(Arc::new(ProfileRegistry::load_from_default_path()));
        tracing::info!("ProfileRegistry initialized");
    }

    /// Get a reference to the profile registry, if one is attached.
    pub fn profile_registry(&self) -> Option<&Arc<ProfileRegistry>> {
        self.profile_registry.as_ref()
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
        let replay_seed = replay_seed_from_metadata(&context, &dag)?;
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
        let (python_bridge, typescript_bridge) = handler_bridges_from_artifact(
            &artifact,
            self.python_worker_sandbox(),
            Self::script_sandbox_required(),
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
            .build_context_with_bridge(None, None, None, python_bridge, typescript_bridge)
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

    /// Identical to [`Self::execute_artifact_with_session_emitter_and_metadata`],
    /// except it returns as soon as EITHER the artifact completes OR a node
    /// parks on the conversation-loop's session-recv key ( narrow park
    /// observability), whichever happens first.
    ///
    /// This is a genuinely new entry point built on
    /// [`crate::scheduler::DataflowScheduler::execute_or_park`]; it does not
    /// change the behavior of `execute_artifact_with_session_emitter_and_metadata`
    /// or any other existing `execute*` method — those still block until full
    /// completion exactly as before.
    ///
    /// Intended caller: a host (e.g. `POST /v1/skills/{id}/execute`) that wants
    /// to know "this execution just started waiting for the next turn's
    /// message" without blocking for the lifetime of the conversation session.
    pub async fn execute_artifact_with_session_emitter_and_metadata_or_park(
        &self,
        artifact: Artifact,
        args: Vec<String>,
        session_id: Option<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
        extra_metadata: HashMap<String, String>,
    ) -> Result<ExecutionOutcome, RuntimeError> {
        self.execute_artifact_inner_or_park(
            artifact,
            args,
            session_id,
            event_emitter,
            session_dir,
            extra_metadata,
        )
        .await
    }

    /// Execute a top-level artifact, seeding metadata AND pre-resolved per-tool
    /// credentials. The host resolves connection ids to bearer
    /// headers and passes them here; the runtime injects them at the trusted
    /// `invoke_capability` seam so the secret never enters the AIR or the prompt.
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
        if !crate::script_admission::script_artifacts_trusted()
            && artifact_has_handler_manifest(&artifact)
        {
            return Err(RuntimeError::Capability {
                capability: HANDLER_MANIFEST_ARTIFACT_SECTION.to_string(),
                message:
                    "handler artifacts require APXM_TRUST_PYTHON_ARTIFACTS and APXM_SANDBOX_PYTHON"
                        .to_string(),
            });
        }

        let _lane_permit = if let Some(ref sid) = session_id {
            Some(self.session_lane_guard.acquire(sid).await)
        } else {
            None
        };

        let (python_bridge, typescript_bridge) = handler_bridges_from_artifact(
            &artifact,
            self.python_worker_sandbox(),
            Self::script_sandbox_required(),
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
            .build_context_with_bridge(
                session_id,
                event_emitter,
                session_dir,
                python_bridge,
                typescript_bridge,
            )
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
        // `ctx.invoke_capability` seam. Child contexts share the counter, so the bound
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
        // propagated metadata or events; the runtime injects it at `invoke_capability`.
        if tool_credentials.is_some() {
            context = context.with_tool_credentials(tool_credentials);
        }
        let context = context;
        let graph_emitter = context.event_emitter.as_ref().map(Arc::clone);
        let execution_id = context.execution_id.clone();
        let node_count = entry_dag.nodes.len();
        let replay_seed = replay_seed_from_metadata(&context, &entry_dag)?;

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

    /// Park-observable twin of [`Self::execute_artifact_inner`]. Shares the
    /// same setup (python bridge, entry DAG resolution, flow registry,
    /// context/hooks construction, graph-lifecycle registration) but drives
    /// execution through [`crate::scheduler::DataflowScheduler::execute_or_park`]
    /// instead of `execute_with_hooks_and_seed`, so it can return as soon as a
    /// session-recv park is observed instead of only at full completion.
    ///
    /// On `Parked`, the backend graph lifecycle is NOT released here — the DAG
    /// is still running in the background. Lifecycle release + the
    /// `emit_graph_end` hook are chained onto the scheduler's background
    /// completion handle instead, so backend graph resources are still
    /// released exactly once, just later (when the session eventually ends or
    /// the host cancels it) rather than immediately.
    async fn execute_artifact_inner_or_park(
        &self,
        artifact: Artifact,
        args: Vec<String>,
        session_id: Option<String>,
        event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
        extra_metadata: HashMap<String, String>,
    ) -> Result<ExecutionOutcome, RuntimeError> {
        if !crate::script_admission::script_artifacts_trusted()
            && artifact_has_handler_manifest(&artifact)
        {
            return Err(RuntimeError::Capability {
                capability: HANDLER_MANIFEST_ARTIFACT_SECTION.to_string(),
                message:
                    "handler artifacts require APXM_TRUST_PYTHON_ARTIFACTS and APXM_SANDBOX_PYTHON"
                        .to_string(),
            });
        }

        let _lane_permit = if let Some(ref sid) = session_id {
            Some(self.session_lane_guard.acquire(sid).await)
        } else {
            None
        };

        let (python_bridge, typescript_bridge) = handler_bridges_from_artifact(
            &artifact,
            self.python_worker_sandbox(),
            Self::script_sandbox_required(),
        )?;
        let entry_dag = find_entry_dag(&artifact)?;
        let arg_values = bind_args(&entry_dag, args)?;

        let artifact_flow_registry = Arc::new(FlowRegistry::new());
        for agent in reconstruct_agents_from_artifact(&artifact) {
            artifact_flow_registry.register_agent(agent);
        }

        #[cfg(feature = "metrics")]
        if !extra_metadata.contains_key(metadata::PARENT_EXECUTION_ID) {
            self.llm_registry.metrics().reset();
        }

        let mut context = self
            .build_context_with_bridge(
                session_id,
                event_emitter,
                session_dir,
                python_bridge,
                typescript_bridge,
            )
            .with_flow_registry(artifact_flow_registry)
            .with_graph_id(graph_id_from_dag(&entry_dag));
        for (key, value) in extra_metadata {
            context.metadata.insert(key, value);
        }
        if let Some(budgets) = context
            .metadata
            .get(metadata::TOOL_CALL_BUDGETS)
            .and_then(|raw| serde_json::from_str::<HashMap<String, usize>>(raw).ok())
        {
            context = context.with_tool_call_budgets(Some(budgets));
        }
        let context = context;
        let graph_emitter = context.event_emitter.as_ref().map(Arc::clone);
        let execution_id = context.execution_id.clone();
        let node_count = entry_dag.nodes.len();
        let replay_seed = replay_seed_from_metadata(&context, &entry_dag)?;

        let dispatch_ir =
            graph_dispatch_ir_from_dag(&context.graph_id, &context.execution_id, &entry_dag);
        context.set_dispatch_ir_v1(dispatch_ir.clone());
        let (lifecycles, _dispatch_fallbacks) =
            build_graph_lifecycles(&self.llm_registry, &dispatch_ir).await;
        if let Some(emitter) = &graph_emitter {
            emitter.emit_graph_start(&execution_id, node_count);
        }
        let executor = Arc::new(ExecutorEngine::new(context.clone()));
        let token_accountant = Arc::clone(&context.token_accountant);
        let fields_honored = Arc::clone(&context.fields_honored);
        let graph_metrics = Arc::clone(&context.graph_metrics);
        let tool_call_counts = Arc::clone(&context.tool_call_counts);
        let hook_context = ExecutionHookContext::new(
            context.execution_id.clone(),
            context.graph_id.clone(),
            self.execution_hooks.clone(),
        );

        let outcome = self
            .scheduler
            .execute_or_park(
                entry_dag,
                executor,
                context,
                arg_values,
                hook_context,
                replay_seed.as_ref(),
            )
            .await;

        match outcome {
            Ok(crate::scheduler::SchedulerOutcome::Parked {
                session_id,
                background: scheduler_background,
            }) => {
                // Not done: release the backend graph lifecycle + emit
                // graph_end only once the background completion actually
                // finishes, not now. Exposed as `background` on
                // `ExecutionOutcome::Parked` too, so a caller with its own
                // "not really done yet" bookkeeping (e.g. the server's
                // cross-execution admission slot) can chain onto the SAME
                // real-completion event instead of guessing when it's safe.
                let background = tokio::spawn(async move {
                    let _ = scheduler_background.await;
                    release_graph_lifecycles(&lifecycles).await;
                    if let Some(emitter) = &graph_emitter {
                        emitter.emit_graph_end(&execution_id, node_count, true);
                    }
                });
                Ok(ExecutionOutcome::Parked {
                    session_id,
                    background,
                })
            }
            Ok(crate::scheduler::SchedulerOutcome::Completed((
                results,
                stats,
                scheduler_metrics,
                all_outputs,
                node_output_map,
            ))) => {
                let graph_status_snapshots = release_graph_lifecycles(&lifecycles).await;
                if let Some(emitter) = &graph_emitter {
                    emitter.emit_graph_end(&execution_id, node_count, true);
                }

                let token_snapshot = token_accountant.snapshot();
                let graph_metrics_snapshot = graph_metrics.snapshot();
                let backend_graph_capabilities = self.llm_registry.graph_capabilities();
                let fields_honored_by_backend = fields_honored.snapshot();
                let dispatch_ir_metrics = dispatch_ir_accounting_json(
                    Some(&dispatch_ir),
                    &backend_graph_capabilities,
                    &graph_status_snapshots,
                    &_dispatch_fallbacks,
                    &fields_honored_by_backend,
                );

                Ok(ExecutionOutcome::Completed(RuntimeExecutionResult {
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
                }))
            }
            Err(error) => {
                let _graph_status_snapshots = release_graph_lifecycles(&lifecycles).await;
                if let Some(emitter) = &graph_emitter {
                    emitter.emit_graph_end(&execution_id, node_count, false);
                }
                Err(error)
            }
        }
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
/// against the recompiled `dag`. Missing metadata leaves this as a full run;
/// an explicit but incomplete or unsafe partial replay is rejected.
fn replay_seed_from_metadata(
    context: &ExecutionContext,
    dag: &ExecutionDag,
) -> Result<Option<crate::scheduler::ReplaySeed>, RuntimeError> {
    let seed = crate::scheduler::ReplaySeed::from_metadata_checked(&context.metadata, dag)
        .map_err(|error| RuntimeError::Scheduler {
            message: format!("partial replay rejected: {error}"),
        })?;
    if let Some(seed) = &seed {
        log_info!(
            "runtime",
            execution_id = %context.execution_id,
            from_node = seed.from_node,
            replayed = seed.replayed_count(),
            completed = seed.completed_nodes.len(),
            "partial replay: re-executing from_node + descendants only"
        );
    }
    Ok(seed)
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

fn artifact_has_handler_manifest(artifact: &Artifact) -> bool {
    artifact
        .sections()
        .iter()
        .any(|section| section.kind == HANDLER_MANIFEST_ARTIFACT_SECTION)
}

fn handler_bridges_from_artifact(
    artifact: &Artifact,
    sandbox: Option<Arc<dyn crate::sandbox::SandboxBackend>>,
    sandbox_required: bool,
) -> Result<
    (
        Option<Arc<PythonHandlerBridge>>,
        Option<Arc<TypeScriptHandlerBridge>>,
    ),
    RuntimeError,
> {
    let section = artifact
        .sections()
        .iter()
        .find(|section| section.kind == HANDLER_MANIFEST_ARTIFACT_SECTION);

    let Some(section) = section else {
        return Ok((None, None));
    };

    let manifest = HandlerManifest::from_json_slice(&section.data).map_err(|error| {
        RuntimeError::Capability {
            capability: HANDLER_MANIFEST_ARTIFACT_SECTION.into(),
            message: format!("Failed to parse handler manifest: {error}"),
        }
    })?;
    manifest
        .validate()
        .map_err(|error| RuntimeError::Capability {
            capability: HANDLER_MANIFEST_ARTIFACT_SECTION.into(),
            message: format!("Invalid handler manifest: {error}"),
        })?;

    let python_registry = PythonHandlerRegistry::from_manifest(manifest.clone())?;
    let typescript_registry = TypeScriptHandlerRegistry::from_manifest(manifest)?;
    let python_count = python_registry.len();
    let typescript_count = typescript_registry.len();

    let python_bridge = (!python_registry.is_empty()).then(|| {
        Arc::new(
            PythonHandlerBridge::new(python_registry)
                .with_sandbox(sandbox.clone(), sandbox_required),
        )
    });
    let typescript_bridge = (!typescript_registry.is_empty()).then(|| {
        Arc::new(
            TypeScriptHandlerBridge::new(typescript_registry)
                .with_sandbox(sandbox, sandbox_required),
        )
    });

    log_info!(
        "runtime",
        python_tools = python_count,
        typescript_tools = typescript_count,
        "Loaded handler bridges from artifact"
    );

    Ok((python_bridge, typescript_bridge))
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

    fn artifact_with_script_section() -> Artifact {
        let mut art = artifact(vec![single_node_dag(
            "main",
            true,
            Node::new(1, AISOperationType::Nop),
        )]);
        let data = serde_json::to_vec(&HandlerManifest::new(Vec::new()))
            .expect("empty handler manifest serializes");
        art.add_section(ArtifactSection {
            kind: HANDLER_MANIFEST_ARTIFACT_SECTION.to_string(),
            data,
        });
        art
    }

    #[tokio::test]
    async fn handler_manifest_requires_sandbox_flag() {
        let _lock = crate::script_admission::test_support::ENV_LOCK
            .lock()
            .unwrap();
        let _guard = crate::script_admission::test_support::EnvGuard;
        crate::script_admission::test_support::set_vars(false, false);

        let runtime = Runtime::new(RuntimeConfig::in_memory()).await.unwrap();
        let artifact = artifact_with_script_section();

        let err = runtime
            .execute_artifact_with_args(artifact, Vec::new())
            .await
            .expect_err("handler manifest must fail closed without trust+sandbox opt-in");

        assert!(err.to_string().contains("APXM_SANDBOX_PYTHON"));
    }

    /// Environment matrix for script-artifact admission: a script
    /// section (Python or TypeScript) is admitted only when BOTH
    /// `APXM_TRUST_PYTHON_ARTIFACTS` and `APXM_SANDBOX_PYTHON` are set —
    /// trust-only and sandbox-only must fail closed identically to no vars
    /// at all. This is the guard that also protects the CLI's precompiled
    /// `.apxmobj` path, which never passes through the driver's attach gate
    /// or the Server's admission route.
    #[tokio::test]
    async fn env_matrix_script_sections_admitted_only_when_fully_trusted() {
        let _lock = crate::script_admission::test_support::ENV_LOCK
            .lock()
            .unwrap();
        let _guard = crate::script_admission::test_support::EnvGuard;

        for (trust, sandbox, expect_admitted) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, true),
        ] {
            crate::script_admission::test_support::set_vars(trust, sandbox);
            let runtime = Runtime::new(RuntimeConfig::in_memory()).await.unwrap();
            let result = runtime
                .execute_artifact_with_args(artifact_with_script_section(), Vec::new())
                .await;
            assert_eq!(
                result.is_ok(),
                expect_admitted,
                "trust={trust} sandbox={sandbox}: expected admitted={expect_admitted}, got {result:?}"
            );
        }
    }

    /// Recovery: a rejection under no vars is not sticky/cached — setting
    /// both vars and re-running the identical artifact on the same runtime
    /// instance succeeds, proving the gate is a pure function of env state.
    #[tokio::test]
    async fn script_admission_recovers_after_trust_and_sandbox_are_set() {
        let _lock = crate::script_admission::test_support::ENV_LOCK
            .lock()
            .unwrap();
        let _guard = crate::script_admission::test_support::EnvGuard;

        let runtime = Runtime::new(RuntimeConfig::in_memory()).await.unwrap();

        crate::script_admission::test_support::set_vars(false, false);
        let rejected = runtime
            .execute_artifact_with_args(artifact_with_script_section(), Vec::new())
            .await;
        assert!(
            rejected.is_err(),
            "expected no-vars rejection before recovery"
        );

        crate::script_admission::test_support::set_vars(true, true);
        let admitted = runtime
            .execute_artifact_with_args(artifact_with_script_section(), Vec::new())
            .await;
        assert!(
            admitted.is_ok(),
            "expected identical artifact to be admitted once both vars are set: {admitted:?}"
        );
    }

    // --  narrow park observability: `execute_artifact_with_session_emitter_and_metadata_or_park` --

    /// Regression-equivalent to the old (blocking) behavior: a normal
    /// completing artifact returns `Completed` via the new park-observable
    /// entry point.
    #[tokio::test]
    async fn or_park_returns_completed_for_a_normal_execution() {
        let runtime = Runtime::new(RuntimeConfig::in_memory()).await.unwrap();
        let art = artifact(vec![single_node_dag(
            "main",
            true,
            Node::new(1, AISOperationType::Nop),
        )]);

        let outcome = runtime
            .execute_artifact_with_session_emitter_and_metadata_or_park(
                art,
                Vec::new(),
                None,
                None,
                None,
                HashMap::new(),
            )
            .await
            .expect("execution should succeed");

        match outcome {
            ExecutionOutcome::Completed(_) => {}
            ExecutionOutcome::Parked { session_id, .. } => {
                panic!("expected Completed, got Parked({session_id})");
            }
        }
    }

    /// An execution whose in-graph conversation loop reaches its
    /// `session_recv` park point returns `Parked { session_id }` promptly —
    /// as soon as the park happens, not after some fixed timeout and not only
    /// at full completion (a session-recv park never completes on its own).
    #[tokio::test]
    async fn or_park_returns_parked_for_a_session_recv_park() {
        let runtime = Runtime::new(RuntimeConfig::in_memory()).await.unwrap();

        // AUTONOMOUS mode=recv with no `recv_url` and no seed input parks on
        // `park_registry::session_recv_key(session_id)` (see
        // `executor/handlers/autonomous.rs::recv_loop`) instead of completing.
        let mut node = Node::new(1, AISOperationType::Autonomous);
        node.set_attribute("mode".to_string(), Value::String("recv".to_string()));
        let art = artifact(vec![single_node_dag("main", true, node)]);

        let session_id = "test-session-recv-park".to_string();

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            runtime.execute_artifact_with_session_emitter_and_metadata_or_park(
                art,
                Vec::new(),
                Some(session_id.clone()),
                None,
                None,
                HashMap::new(),
            ),
        )
        .await
        .expect("should observe the park promptly, not hang until some timeout")
        .expect("execution should not error");

        match outcome {
            ExecutionOutcome::Parked {
                session_id: observed,
                ..
            } => {
                assert_eq!(observed, session_id);
            }
            ExecutionOutcome::Completed(_) => {
                panic!("expected Parked — a session-recv park never completes on its own");
            }
        }
    }

    /// An execution that parks for a DIFFERENT reason (RESUME on a checkpoint
    /// id, not a session-recv key) must NOT be reported as `Parked` through
    /// this narrow API — it has to keep blocking exactly like the old
    /// behavior, since this signal is scoped to session-recv parks only.
    #[tokio::test]
    async fn or_park_does_not_report_a_non_session_recv_park() {
        let runtime = Runtime::new(RuntimeConfig::in_memory()).await.unwrap();

        let mut node = Node::new(1, AISOperationType::Resume);
        node.set_attribute(
            apxm_core::constants::graph::attrs::CHECKPOINT.to_string(),
            Value::String("some_other_checkpoint".to_string()),
        );
        let art = artifact(vec![single_node_dag("main", true, node)]);

        // Give the RESUME node time to park, then wake it via the ordinary
        // (non-session-recv) checkpoint wait key so the execution can
        // complete and the test doesn't hang.
        let wake_after = async {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            crate::scheduler::park_registry::wake(
                "some_other_checkpoint",
                Value::String("resumed".to_string()),
            );
        };

        let (outcome, _) = tokio::join!(
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                runtime.execute_artifact_with_session_emitter_and_metadata_or_park(
                    art,
                    Vec::new(),
                    Some("unrelated-session".to_string()),
                    None,
                    None,
                    HashMap::new(),
                ),
            ),
            wake_after,
        );

        let outcome = outcome
            .expect("should not hang: a non-session-recv park must not surface as Parked")
            .expect("execution should complete once woken");

        match outcome {
            ExecutionOutcome::Completed(_) => {}
            ExecutionOutcome::Parked { session_id, .. } => {
                panic!(
                    "a RESUME (non-session-recv) park must not be reported as Parked, got session_id={session_id}"
                );
            }
        }
    }
}
