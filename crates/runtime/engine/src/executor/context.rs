//! Execution context - Holds runtime state and provides access to subsystems

use crate::dispatch::v1::{DispatchIrV1, derive_apxm_hints};
use crate::metadata_keys as metadata;
use crate::python_tools::PythonHandlerBridge;
use crate::sandbox::SandboxRegistry;
use crate::{
    aam::{Aam, ScopeSpec},
    agent_pool::AgentPool,
    capability::CapabilitySystem,
    capability::flow_registry::FlowRegistry,
    context_stack::ContextStack,
    memory::MemorySystem,
    process_table::ProcessTable,
    workspace::ScopeRegistry,
};
use apxm_backends::LLMRegistry;
use apxm_core::InstructionConfig;
use apxm_core::constants::cache;
use apxm_core::paths::ApxmPaths;
use apxm_core::types::{Agent, ApxmGraphHints, MetricsLevel, OptimizationTarget};
use std::sync::Arc;

use super::agent_scope::AgentScopeStack;
use super::cancellation::CancellationToken;
use super::dag_splicer::{DagSplicer, NoOpSplicer};
use super::events::ExecutionEventEmitter;
use super::fields_honored::FieldsHonoredCollector;
use super::graph_metrics::GraphMetricsTracker;
use super::handlers::warmup::{WarmupConfig, WarmupMetrics};
use super::hooks::HookRegistry;
use super::inner_plan_linker::{InnerPlanLinker, NoOpLinker};
use super::memoization::MemoCache;
use super::middleware::OperationMiddleware;
use super::session_ledger::SessionLedger;
use super::skill_resolver::{NoOpSkillResolver, SkillResolver};
use super::timing_tracker::TimingTracker;
use super::token_accounting::TokenAccountant;
use super::workflow_spawner::{NoOpWorkflowSpawner, WorkflowSpawner};
use crate::model_router::ModelRouter;
use crate::runtime::LlmToolDispatchConfig;

/// Execution context passed to all operation handlers.
#[derive(Clone)]
pub struct ExecutionContext {
    pub execution_id: String,
    /// Stable graph identifier for graph-aware backends and per-request hints.
    /// Worker child contexts inherit this so every node request points at the
    /// same registered graph.
    pub graph_id: String,
    pub(crate) dispatch_ir_v1: Arc<parking_lot::RwLock<Option<DispatchIrV1>>>,
    pub session_id: Option<String>,
    pub memory: Arc<MemorySystem>,
    pub llm_registry: Arc<LLMRegistry>,
    pub capability_system: Arc<CapabilitySystem>,
    pub aam: Aam,
    pub scope_id: String,
    pub scope_registry: Arc<ScopeRegistry>,
    pub inner_plan_linker: Arc<dyn InnerPlanLinker>,
    pub workflow_spawner: Arc<dyn WorkflowSpawner>,
    /// Host-supplied resolver for binding `CALL_SKILL` `skill_id` values (or
    /// `skill_id@version`) to a concrete artifact at
    /// execution time. Defaults to [`NoOpSkillResolver`], which fails
    /// `CALL_SKILL` cleanly with a `call_skill_no_resolver` capability
    /// error.
    pub skill_resolver: Arc<dyn SkillResolver>,
    pub dag_splicer: Arc<dyn DagSplicer>,
    pub flow_registry: Arc<FlowRegistry>,
    pub current_agent: Option<Arc<Agent>>,
    pub instruction_config: InstructionConfig,
    pub start_time: std::time::Instant,
    pub metadata: std::collections::HashMap<String, String>,
    pub token_budget: Option<u64>,
    /// Runtime view of the compiler/driver optimization target for this graph.
    pub optimization_target: OptimizationTarget,
    /// Metrics emission tier for this execution. `Detailed` enables in-flight
    /// observers (currently per-graph pin-peak polling); `Basic` records
    /// steady-state aggregates only.
    pub metrics_level: MetricsLevel,
    pub consumed_tokens: Arc<std::sync::atomic::AtomicU64>,
    /// Per-tool call-count budget: max calls allowed per capability
    /// name for this execution tree. `None` = unbounded. Declared by the
    /// program/request; enforced by [`Self::charge_tool_call`] at the trusted
    /// invoke seam — never by the AIR program itself.
    pub tool_call_budgets: Option<Arc<std::collections::HashMap<String, usize>>>,
    /// Consumed per-tool call counts. Shared (`Arc`) into child contexts so a
    /// spawned-agent / called-skill fan-out cannot multiply the budget — mirrors
    /// `consumed_tokens`.
    pub tool_call_counts: Arc<std::sync::Mutex<std::collections::HashMap<String, usize>>>,
    /// Per-tool credential headers: capability name → an
    /// `Authorization` header value (e.g. `"Bearer …"`), pre-resolved by the
    /// trusted host from a connection id. Injected into a tool's args at the
    /// `invoke_capability` seam so a tool acquires its auth token without the secret
    /// ever entering the AIR program or the prompt. `None` = no per-tool auth.
    pub tool_credentials: Option<Arc<std::collections::HashMap<String, String>>>,
    pub warmup_config: WarmupConfig,
    pub warmup_metrics: Arc<WarmupMetrics>,
    pub event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
    pub graph_metrics: Arc<GraphMetricsTracker>,
    /// Per-execution union of `x-apxm-fields-honored` evidence by
    /// backend. Populated by the LLM handler from each per-node
    /// response's `metadata["fields_honored"]`; drained at execution
    /// end into `dispatch_ir_metrics.fields_honored`.
    pub fields_honored: Arc<FieldsHonoredCollector>,
    pub token_accountant: Arc<TokenAccountant>,
    pub timing_tracker: Arc<TimingTracker>,
    pub response_cache: Arc<MemoCache>,
    pub max_parallel_tool_calls: usize,
    /// Dispatcher-level middleware that wraps every node execution.
    pub middlewares: Vec<Arc<dyn OperationMiddleware>>,
    pub cancellation_token: CancellationToken,
    /// Only used for INV/tool nodes; LLM operations bypass sandboxing.
    pub sandbox_registry: Arc<SandboxRegistry>,
    /// Tracks live agent processes for SPAWN_AGENT and COMMUNICATE.
    pub process_table: Arc<ProcessTable>,
    pub context_stack: Option<Arc<ContextStack>>,
    /// When set, LLM handler delegates backend selection here instead of `llm_registry`.
    pub model_router: Option<Arc<ModelRouter>>,
    pub agent_pool: Arc<AgentPool>,
    /// Python tool bridge for dispatching INV_CAP calls backed by
    /// `@apxm.tool`-decorated Python handlers. `None` when no Python
    /// tools are registered in the artifact.
    pub python_handler_bridge: Option<Arc<PythonHandlerBridge>>,
    /// Per-artifact registry of program-authored lifecycle hooks (`@hook`),
    /// resolved at load from the hooks sidecar / `REGISTER_HOOK` nodes. Shares
    /// the python tool bridge's lifetime and is inherited by child contexts so
    /// spawned-agent / called-skill turns see the same author hooks. `None`
    /// when the artifact declares no hooks.
    pub hook_registry: Option<Arc<HookRegistry>>,
    /// Per-session ledger (turn caps / per-tool budgets) keyed by
    /// `session_id`, owned by the runtime rather than the host (constitution #2).
    /// Inherited by child contexts so a one-execution-per-session conversation
    /// enforces caps across re-armed turns. `None` for non-session executions.
    pub session_ledger: Option<Arc<SessionLedger>>,
    /// Current span ID for hierarchical event nesting. Each node
    /// execution pushes a new child span; the parent is restored on
    /// completion.
    pub current_span_id: Option<String>,
    /// Current scope ID for session-scoped event isolation.
    /// Propagated to emitted events and child contexts.
    pub current_scope_id: Option<String>,
    /// Layer 2 agent-scope stack. Pushed by SPAWN_AGENT, popped when the
    /// spawned subgraph terminates. Drives the agent-layer event
    /// vocabulary (`subagent_*`, `tool_call_*`, …). See
    /// `crates/runtime/engine/src/executor/agent_scope.rs`.
    pub agent_scope_stack: Arc<AgentScopeStack>,
    /// Host id for this execution, when bound to a specific host.
    pub host_id: Option<String>,
    /// Gateway for host dispatch (tool calls, relay egress, agent channels).
    pub host_dispatch: std::sync::Arc<dyn apxm_core::types::host::HostDispatchGateway>,
    /// Consent broker for per-call host capability approval.
    pub consent_broker: std::sync::Arc<dyn apxm_core::types::consent::ConsentBroker>,
}

impl ExecutionContext {
    /// Create a new execution context with no-op inner plan support
    pub fn new(
        memory: Arc<MemorySystem>,
        llm_registry: Arc<LLMRegistry>,
        capability_system: Arc<CapabilitySystem>,
        aam: Aam,
    ) -> Self {
        let execution_id = uuid::Uuid::now_v7().to_string();
        let graph_id = execution_id.clone();
        let scope_id = uuid::Uuid::now_v7().to_string();
        let scope_registry = Arc::new(ScopeRegistry::new());
        scope_registry.register(scope_id.clone(), None, aam.clone(), ScopeSpec::default());

        let mut metadata_map = std::collections::HashMap::new();
        metadata_map.insert(metadata::SCOPE_ID.to_string(), scope_id.clone());

        // Initialize persistent SQLite cache at ~/.apxm/cache/cache.db
        let response_cache = match ApxmPaths::discover()
            .and_then(|paths| paths.cache_dir())
            .map(|cache_dir| cache_dir.join(cache::DB_FILE))
        {
            #[cfg(feature = "sqlite")]
            Ok(db_path) => match MemoCache::new_with_sqlite(&db_path) {
                Ok(cache) => Arc::new(cache),
                Err(e) => {
                    tracing::warn!(
                        "Failed to initialize SQLite cache at {:?}: {}. Falling back to L1-only cache.",
                        db_path,
                        e
                    );
                    Arc::new(MemoCache::new())
                }
            },
            #[cfg(not(feature = "sqlite"))]
            Ok(_) => Arc::new(MemoCache::new()),
            Err(e) => {
                tracing::warn!(
                    "Failed to resolve cache directory: {}. Using L1-only cache.",
                    e
                );
                Arc::new(MemoCache::new())
            }
        };

        Self {
            execution_id,
            graph_id,
            dispatch_ir_v1: Arc::new(parking_lot::RwLock::new(None)),
            session_id: None,
            memory,
            llm_registry,
            capability_system,
            aam,
            scope_id,
            scope_registry,
            inner_plan_linker: Arc::new(NoOpLinker),
            workflow_spawner: Arc::new(NoOpWorkflowSpawner),
            skill_resolver: Arc::new(NoOpSkillResolver),
            dag_splicer: Arc::new(NoOpSplicer),
            flow_registry: Arc::new(FlowRegistry::new()),
            current_agent: None,
            instruction_config: InstructionConfig::default(),
            start_time: std::time::Instant::now(),
            metadata: metadata_map,
            token_budget: None,
            optimization_target: OptimizationTarget::Balanced,
            metrics_level: MetricsLevel::default(),
            consumed_tokens: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            tool_call_budgets: None,
            tool_call_counts: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            tool_credentials: None,
            warmup_config: WarmupConfig::default(),
            warmup_metrics: Arc::new(WarmupMetrics::new()),
            event_emitter: None,
            graph_metrics: Arc::new(GraphMetricsTracker::new()),
            fields_honored: Arc::new(FieldsHonoredCollector::new()),
            token_accountant: Arc::new(TokenAccountant::new()),
            timing_tracker: Arc::new(TimingTracker::new()),
            response_cache,
            max_parallel_tool_calls: LlmToolDispatchConfig::default()
                .sanitized_max_parallel_tool_calls(),
            middlewares: Vec::new(),
            cancellation_token: CancellationToken::new(),
            sandbox_registry: Arc::new(SandboxRegistry::new()),
            process_table: Arc::new(ProcessTable::new()),
            context_stack: None,
            model_router: None,
            agent_pool: Arc::new(AgentPool::new(4, std::time::Duration::from_secs(300))),
            python_handler_bridge: None,
            hook_registry: None,
            session_ledger: None,
            current_span_id: None,
            current_scope_id: None,
            agent_scope_stack: Arc::new(AgentScopeStack::new()),
            host_id: None,
            host_dispatch: std::sync::Arc::new(crate::host_dispatch::NoOpHostDispatchGateway),
            consent_broker: std::sync::Arc::new(apxm_core::types::consent::NoOpConsentBroker),
        }
    }

    /// Create execution context with full inner plan support (linker + splicer)
    pub fn with_inner_plan_support(
        memory: Arc<MemorySystem>,
        llm_registry: Arc<LLMRegistry>,
        capability_system: Arc<CapabilitySystem>,
        aam: Aam,
        inner_plan_linker: Arc<dyn InnerPlanLinker>,
        dag_splicer: Arc<dyn DagSplicer>,
        flow_registry: Arc<FlowRegistry>,
    ) -> Self {
        let mut ctx = Self::new(memory, llm_registry, capability_system, aam);
        ctx.inner_plan_linker = inner_plan_linker;
        ctx.dag_splicer = dag_splicer;
        ctx.flow_registry = flow_registry;
        ctx
    }

    /// Get a reference to the memory subsystem
    pub fn memory(&self) -> &MemorySystem {
        &self.memory
    }

    /// Create context with specific execution ID
    pub fn with_execution_id(mut self, execution_id: String) -> Self {
        self.execution_id = execution_id;
        self
    }

    /// Create context with specific graph ID for graph-aware backend state.
    pub fn with_graph_id(mut self, graph_id: String) -> Self {
        self.graph_id = graph_id;
        self
    }

    pub(crate) fn set_dispatch_ir_v1(&self, dispatch_ir: DispatchIrV1) {
        *self.dispatch_ir_v1.write() = Some(dispatch_ir);
    }

    pub(crate) fn dispatch_hints_for_node(&self, node_id: u32) -> Option<ApxmGraphHints> {
        let guard = self.dispatch_ir_v1.read();
        let ir = guard.as_ref()?;
        let node = ir.nodes.iter().find(|node| node.node_id == node_id)?;
        Some(derive_apxm_hints(ir, node))
    }

    /// Create context with session ID
    pub fn with_session_id(mut self, session_id: String) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: String, value: String) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// Set instruction config for system prompts
    pub fn with_instruction_config(mut self, config: InstructionConfig) -> Self {
        self.instruction_config = config;
        self
    }

    /// Associate this execution context with the current runtime agent.
    pub fn with_agent(mut self, agent: Arc<Agent>) -> Self {
        self.current_agent = Some(agent);
        self
    }

    /// Set a global token budget for the execution.
    pub fn with_token_budget(mut self, budget: Option<u64>) -> Self {
        self.token_budget = budget;
        self
    }

    /// Set the per-tool call-count budget for this execution. An
    /// empty map is treated as no budget.
    pub fn with_tool_call_budgets(
        mut self,
        budgets: Option<std::collections::HashMap<String, usize>>,
    ) -> Self {
        self.tool_call_budgets = budgets.filter(|m| !m.is_empty()).map(Arc::new);
        self
    }

    /// Set pre-resolved per-tool credential headers: capability name
    /// → `Authorization` header value. An empty map is treated as no credentials.
    pub fn with_tool_credentials(
        mut self,
        credentials: Option<std::collections::HashMap<String, String>>,
    ) -> Self {
        self.tool_credentials = credentials.filter(|m| !m.is_empty()).map(Arc::new);
        self
    }

    /// Inject the per-tool credential into a call's args as
    /// `headers.Authorization`, unless the program already supplied one (never
    /// overwrite an explicit credential). No-op when the tool has no bound
    /// credential.
    fn inject_tool_credential(
        &self,
        name: &str,
        args: &mut std::collections::HashMap<String, apxm_core::types::values::Value>,
    ) {
        use apxm_core::types::values::Value;
        let Some(creds) = &self.tool_credentials else {
            return;
        };
        let Some(header) = creds.get(name) else {
            return;
        };
        let headers = args
            .entry("headers".to_string())
            .or_insert_with(|| Value::Object(std::collections::HashMap::new()));
        if let Value::Object(map) = headers
            && !map.keys().any(|k| k.eq_ignore_ascii_case("authorization"))
        {
            map.insert("Authorization".to_string(), Value::String(header.clone()));
        }
    }

    /// Invoke a capability through the per-tool call budget, using the
    /// capability system's default timeout. Both tool-call paths — the graph
    /// `INV_CAP` handler and the in-`ASK`-node model loop — route through here so
    /// the budget is the single trusted enforcement seam; the `ASK`-node calls are
    /// invisible to node-level middleware, which is why this is a ctx helper rather
    /// than an `OperationMiddleware`.
    pub async fn invoke_capability(
        &self,
        name: &str,
        mut args: std::collections::HashMap<String, apxm_core::types::values::Value>,
    ) -> Result<apxm_core::types::values::Value, apxm_core::error::RuntimeError> {
        self.charge_tool_call(name)?;
        self.inject_tool_credential(name, &mut args);
        self.capability_system.invoke(name, args).await
    }

    /// Like [`Self::invoke_capability`] but with an explicit timeout.
    pub async fn invoke_capability_with_timeout(
        &self,
        name: &str,
        mut args: std::collections::HashMap<String, apxm_core::types::values::Value>,
        timeout: std::time::Duration,
    ) -> Result<apxm_core::types::values::Value, apxm_core::error::RuntimeError> {
        self.charge_tool_call(name)?;
        self.inject_tool_credential(name, &mut args);
        let agent_code_owned = self
            .agent_scope_stack
            .peek()
            .map(|s| s.agent_code.clone())
            .or_else(|| self.current_agent.as_ref().map(|a| a.name.clone()));
        let pre_ctx = crate::capability::interceptor::PreInvokeContext {
            registry: self.capability_system.registry(),
            consent_broker: self.consent_broker.as_ref(),
            event_emitter: self
                .event_emitter
                .as_ref()
                .map(|e| e.as_ref() as &dyn crate::ExecutionEventEmitter),
            host_id: self.host_id.as_deref(),
            agent_code: agent_code_owned.as_deref(),
            grant_id: None,
            permission_timeout: crate::capability::interceptor::PreInvokeContext::permission_timeout_from_env(),
        };
        self.capability_system
            .invoke_with_timeout_ctx(name, args, timeout, Some(&pre_ctx))
            .await
    }

    /// Charge one substantive session turn via the attached ledger. Used when
    /// the execution context already holds the session ledger; the recv re-arm
    /// path charges via [`session_ledger::charge_turn_for_wake`] instead.
    pub fn charge_session_turn(&self) -> Result<usize, apxm_core::error::RuntimeError> {
        let Some(ledger) = &self.session_ledger else {
            return Ok(1);
        };
        ledger
            .charge_turn()
            .map_err(|message| apxm_core::error::RuntimeError::Capability {
                capability: "session_turn".to_string(),
                message,
            })
    }

    /// Budget-check and increment the per-tool call counter. Fail-closed: an
    /// exhausted budget denies before the capability executes. A tool with no
    /// configured budget is unbounded. The counter is shared across child
    /// contexts, so the bound spans spawned agents and called skills.
    ///
    /// When a session ledger is attached, per-session tool budgets are the SSOT;
    /// per-execution counters are skipped.
    fn charge_tool_call(&self, name: &str) -> Result<(), apxm_core::error::RuntimeError> {
        if let Some(ledger) = &self.session_ledger {
            if !ledger.charge_tool(name) {
                return Err(apxm_core::error::RuntimeError::Capability {
                    capability: name.to_string(),
                    message: format!("session tool budget exhausted for '{name}'"),
                });
            }
            return Ok(());
        }
        let Some(budgets) = &self.tool_call_budgets else {
            return Ok(());
        };
        let Some(&cap) = budgets.get(name) else {
            return Ok(());
        };
        let mut counts = self
            .tool_call_counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let used = counts.entry(name.to_string()).or_insert(0);
        if *used >= cap {
            return Err(apxm_core::error::RuntimeError::Capability {
                capability: name.to_string(),
                message: format!(
                    "tool call budget exhausted: {used}/{cap} calls for '{name}' in this execution"
                ),
            });
        }
        *used += 1;
        Ok(())
    }

    /// Snapshot of consumed per-tool call counts, for host-side session
    /// accounting (the per-conversation cap is tracked by the chat host).
    pub fn tool_call_counts_snapshot(&self) -> std::collections::HashMap<String, usize> {
        self.tool_call_counts
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// Set an execution event emitter.
    pub fn with_event_emitter(mut self, emitter: Option<Arc<dyn ExecutionEventEmitter>>) -> Self {
        self.event_emitter = emitter;
        self
    }

    /// Replace the flow registry for flow-control operation handlers.
    pub fn with_flow_registry(mut self, flow_registry: Arc<FlowRegistry>) -> Self {
        self.flow_registry = flow_registry;
        self
    }

    /// Replace the skill resolver for the `CALL_SKILL` op.
    pub fn with_skill_resolver(mut self, resolver: Arc<dyn SkillResolver>) -> Self {
        self.skill_resolver = resolver;
        self
    }

    /// Set a cancellation token (replaces the default root token).
    pub fn with_cancellation_token(mut self, token: CancellationToken) -> Self {
        self.cancellation_token = token;
        self
    }

    /// Replace the dispatcher middleware chain for this context.
    pub fn with_middlewares(mut self, middlewares: Vec<Arc<dyn OperationMiddleware>>) -> Self {
        self.middlewares = middlewares;
        self
    }

    /// Append one dispatcher middleware to this context.
    pub fn with_middleware(mut self, middleware: Arc<dyn OperationMiddleware>) -> Self {
        self.middlewares.push(middleware);
        self
    }

    /// Get elapsed time since execution started
    pub fn elapsed(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Create a child context that shares the parent's AAM (Inherit on all dimensions).
    pub fn child(&self) -> Self {
        self.child_with_scope(ScopeSpec::default())
    }

    /// Create a child context with a scoped AAM.
    ///
    /// The [`ScopeSpec`] controls which parts of the parent AAM are inherited:
    /// - [`ScopePolicy::Inherit`]  -- child shares the parent's `Arc` (writes propagate both ways)
    /// - [`ScopePolicy::Isolate`]  -- child starts with empty state
    /// - [`ScopePolicy::Snapshot`] -- child gets a point-in-time copy (writes are private)
    /// - [`ScopePolicy::Filter(keys)`] -- child gets only the listed keys (snapshot semantics)
    pub fn child_with_scope(&self, scope: ScopeSpec) -> Self {
        let child_aam = self.aam.child_scope(&scope);
        let child_scope_id = uuid::Uuid::now_v7().to_string();
        self.scope_registry.register(
            child_scope_id.clone(),
            Some(self.scope_id.clone()),
            child_aam.clone(),
            scope,
        );
        self.child_with_aam(child_aam, child_scope_id)
    }

    fn child_with_aam(&self, aam: Aam, scope_id: String) -> Self {
        let mut metadata_map = self.metadata.clone();
        metadata_map.insert(metadata::SCOPE_ID.to_string(), scope_id.clone());
        metadata_map.insert(metadata::PARENT_SCOPE_ID.to_string(), self.scope_id.clone());
        let child_scope_id_for_events = Some(scope_id.clone());

        Self {
            execution_id: uuid::Uuid::now_v7().to_string(),
            graph_id: self.graph_id.clone(),
            dispatch_ir_v1: Arc::clone(&self.dispatch_ir_v1),
            session_id: self.session_id.clone(),
            memory: Arc::clone(&self.memory),
            llm_registry: Arc::clone(&self.llm_registry),
            capability_system: Arc::clone(&self.capability_system),
            aam,
            scope_id,
            scope_registry: Arc::clone(&self.scope_registry),
            inner_plan_linker: Arc::clone(&self.inner_plan_linker),
            workflow_spawner: Arc::clone(&self.workflow_spawner),
            skill_resolver: Arc::clone(&self.skill_resolver),
            dag_splicer: Arc::clone(&self.dag_splicer),
            flow_registry: Arc::clone(&self.flow_registry),
            current_agent: self.current_agent.as_ref().map(Arc::clone),
            instruction_config: self.instruction_config.clone(),
            start_time: std::time::Instant::now(),
            metadata: metadata_map,
            token_budget: self.token_budget,
            optimization_target: self.optimization_target,
            metrics_level: self.metrics_level,
            consumed_tokens: Arc::clone(&self.consumed_tokens),
            tool_call_budgets: self.tool_call_budgets.clone(),
            tool_call_counts: Arc::clone(&self.tool_call_counts),
            tool_credentials: self.tool_credentials.clone(),
            warmup_config: self.warmup_config.clone(),
            warmup_metrics: Arc::clone(&self.warmup_metrics),
            event_emitter: self.event_emitter.as_ref().map(Arc::clone),
            graph_metrics: Arc::clone(&self.graph_metrics),
            fields_honored: Arc::clone(&self.fields_honored),
            token_accountant: Arc::clone(&self.token_accountant),
            timing_tracker: Arc::clone(&self.timing_tracker),
            response_cache: Arc::clone(&self.response_cache),
            max_parallel_tool_calls: self.max_parallel_tool_calls,
            middlewares: self.middlewares.clone(),
            cancellation_token: self.cancellation_token.child(),
            sandbox_registry: Arc::clone(&self.sandbox_registry),
            process_table: Arc::clone(&self.process_table),
            context_stack: self.context_stack.as_ref().map(Arc::clone),
            model_router: self.model_router.as_ref().map(Arc::clone),
            agent_pool: Arc::clone(&self.agent_pool),
            python_handler_bridge: self.python_handler_bridge.as_ref().map(Arc::clone),
            hook_registry: self.hook_registry.as_ref().map(Arc::clone),
            session_ledger: self.session_ledger.as_ref().map(Arc::clone),
            current_span_id: self.current_span_id.clone(),
            current_scope_id: child_scope_id_for_events,
            // Share the agent-scope stack with the child so that
            // worker tasks emit Layer 2 events under the correct
            // current agent. The stack is `Arc<Mutex<…>>`, so
            // push/pop in either context is visible to the other.
            agent_scope_stack: Arc::clone(&self.agent_scope_stack),
            host_id: self.host_id.clone(),
            host_dispatch: std::sync::Arc::clone(&self.host_dispatch),
            consent_broker: std::sync::Arc::clone(&self.consent_broker),
        }
    }

    /// Attach a ContextStack for prompt enrichment.
    pub fn with_context_stack(mut self, stack: Arc<ContextStack>) -> Self {
        self.context_stack = Some(stack);
        self
    }

    /// Set the process table for agent lifecycle management.
    pub fn with_process_table(mut self, table: Arc<ProcessTable>) -> Self {
        self.process_table = table;
        self
    }

    /// Set the sandbox registry.
    pub fn with_sandbox_registry(mut self, registry: Arc<SandboxRegistry>) -> Self {
        self.sandbox_registry = registry;
        self
    }

    /// Attach a ModelRouter for dynamic backend/model selection with circuit breakers.
    pub fn with_model_router(mut self, router: Arc<ModelRouter>) -> Self {
        self.model_router = Some(router);
        self
    }

    /// Set the Python tool bridge for dispatching to `@apxm.tool` handlers.
    pub fn with_python_handler_bridge(mut self, bridge: Arc<PythonHandlerBridge>) -> Self {
        self.python_handler_bridge = Some(bridge);
        self
    }

    /// Attach the per-artifact author hook registry (`@hook` bindings). Shares
    /// the python tool bridge's lifetime; inherited by child contexts.
    pub fn with_hook_registry(mut self, registry: Arc<HookRegistry>) -> Self {
        self.hook_registry = Some(registry);
        self
    }

    /// Author hook registry for this execution, if the artifact declared hooks.
    pub fn hook_registry(&self) -> Option<&Arc<HookRegistry>> {
        self.hook_registry.as_ref()
    }

    /// Attach the per-session ledger (turn caps / tool budgets).
    pub fn with_session_ledger(mut self, ledger: Arc<SessionLedger>) -> Self {
        self.session_ledger = Some(ledger);
        self
    }

    /// Per-session ledger for this execution, if session-scoped.
    pub fn session_ledger(&self) -> Option<&Arc<SessionLedger>> {
        self.session_ledger.as_ref()
    }

    pub fn aam(&self) -> &Aam {
        &self.aam
    }

    pub fn scope_id(&self) -> &str {
        &self.scope_id
    }

    /// Stable namespace for cross-turn graph memory (QMEM/UMEM/plan LTM).
    ///
    /// Returns `session_id` when the host supplied one, so memory written in
    /// one turn/execution is readable by later executions sharing that session;
    /// otherwise falls back to the per-execution `scope_id` (the pre-session
    /// behavior). The invariant that makes this stable: `session_id` is
    /// propagated to every child context (see `child_with_aam`), while
    /// `scope_id` is re-minted per node/execution.
    pub fn memory_scope(&self) -> &str {
        self.session_id.as_deref().unwrap_or(&self.scope_id)
    }

    /// Get a reference to the flow registry
    pub fn flow_registry(&self) -> &FlowRegistry {
        &self.flow_registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::capability::executor::EchoCapability;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use std::collections::HashMap;
    use std::sync::Arc;

    async fn test_ctx_with_ledger(ledger: SessionLedger) -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        capability_system
            .register(Arc::new(EchoCapability::new()))
            .expect("echo capability");
        ExecutionContext::new(memory, Arc::new(LLMRegistry::new()), capability_system, aam)
            .with_session_ledger(Arc::new(ledger))
    }

    #[tokio::test]
    async fn charge_session_turn_enforces_cap_on_context() {
        let ctx = test_ctx_with_ledger(SessionLedger::new(Some(2), HashMap::new())).await;
        assert_eq!(ctx.charge_session_turn().unwrap(), 1);
        assert_eq!(ctx.charge_session_turn().unwrap(), 2);
        assert!(ctx.charge_session_turn().is_err(), "third turn exceeds cap");
    }

    #[tokio::test]
    async fn session_tool_budget_enforced_at_invoke_seam() {
        let mut budgets = HashMap::new();
        budgets.insert("echo".to_string(), 1);
        let ctx = test_ctx_with_ledger(SessionLedger::new(None, budgets)).await;
        let mut args = HashMap::new();
        args.insert(
            "message".to_string(),
            apxm_core::types::values::Value::String("hi".to_string()),
        );
        let first = ctx.invoke_capability("echo", args.clone()).await;
        assert!(first.is_ok(), "first invoke failed: {:?}", first.err());
        let err = ctx.invoke_capability("echo", args).await.unwrap_err();
        assert!(
            err.to_string().contains("session tool budget exhausted"),
            "unexpected error: {err}"
        );
    }
}
