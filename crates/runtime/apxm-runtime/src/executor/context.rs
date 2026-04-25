//! Execution context - Holds runtime state and provides access to subsystems

use crate::python_tools::PythonToolBridge;
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
use apxm_core::constants::{cache, runtime::metadata};
use apxm_core::paths::ApxmPaths;
use apxm_core::types::Agent;
use std::sync::Arc;

use super::cancellation::CancellationToken;
use super::dag_splicer::{DagSplicer, NoOpSplicer};
use super::events::ExecutionEventEmitter;
use super::inner_plan_linker::{InnerPlanLinker, NoOpLinker};
use super::memoization::ResponseCache;
use super::middleware::OperationMiddleware;
use super::timing_tracker::TimingTracker;
use super::token_accounting::TokenAccountant;
use super::workflow_spawner::{NoOpWorkflowSpawner, WorkflowSpawner};
use crate::model_router::ModelRouter;

/// Execution context passed to all operation handlers.
#[derive(Clone)]
pub struct ExecutionContext {
    pub execution_id: String,
    /// Stable graph identifier for graph-aware backends and per-request hints.
    /// Worker child contexts inherit this so every node request points at the
    /// same registered graph.
    pub graph_id: String,
    pub session_id: Option<String>,
    pub memory: Arc<MemorySystem>,
    pub llm_registry: Arc<LLMRegistry>,
    pub capability_system: Arc<CapabilitySystem>,
    pub aam: Aam,
    pub scope_id: String,
    pub scope_registry: Arc<ScopeRegistry>,
    pub inner_plan_linker: Arc<dyn InnerPlanLinker>,
    pub workflow_spawner: Arc<dyn WorkflowSpawner>,
    pub dag_splicer: Arc<dyn DagSplicer>,
    pub flow_registry: Arc<FlowRegistry>,
    pub current_agent: Option<Arc<Agent>>,
    pub instruction_config: InstructionConfig,
    pub start_time: std::time::Instant,
    pub metadata: std::collections::HashMap<String, String>,
    pub token_budget: Option<u64>,
    pub consumed_tokens: Arc<std::sync::atomic::AtomicU64>,
    pub event_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
    pub token_accountant: Arc<TokenAccountant>,
    pub timing_tracker: Arc<TimingTracker>,
    pub response_cache: Arc<ResponseCache>,
    /// Dispatcher-level middleware that wraps every node execution.
    pub middlewares: Vec<Arc<dyn OperationMiddleware>>,
    pub cancellation_token: CancellationToken,
    /// Only used for INV/tool nodes; LLM operations bypass sandboxing.
    pub sandbox_registry: Arc<SandboxRegistry>,
    /// Tracks live agent processes (local + ACP). Used by SPAWN_AGENT and COMMUNICATE.
    pub process_table: Arc<ProcessTable>,
    pub context_stack: Option<Arc<ContextStack>>,
    /// When set, LLM handler delegates backend selection here instead of `llm_registry`.
    pub model_router: Option<Arc<ModelRouter>>,
    pub agent_pool: Arc<AgentPool>,
    /// Python tool bridge for dispatching INV_TOOL calls backed by
    /// `@apxm.tool`-decorated Python handlers. `None` when no Python
    /// tools are registered in the artifact.
    pub python_tool_bridge: Option<Arc<PythonToolBridge>>,
    /// Current span ID for hierarchical event nesting. Each node
    /// execution pushes a new child span; the parent is restored on
    /// completion.
    pub current_span_id: Option<String>,
    /// Current scope ID for session-scoped event isolation.
    /// Propagated to emitted events and child contexts.
    pub current_scope_id: Option<String>,
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
            Ok(db_path) => match ResponseCache::new_with_sqlite(&db_path) {
                Ok(cache) => Arc::new(cache),
                Err(e) => {
                    tracing::warn!(
                        "Failed to initialize SQLite cache at {:?}: {}. Falling back to L1-only cache.",
                        db_path,
                        e
                    );
                    Arc::new(ResponseCache::new())
                }
            },
            #[cfg(not(feature = "sqlite"))]
            Ok(_) => Arc::new(ResponseCache::new()),
            Err(e) => {
                tracing::warn!(
                    "Failed to resolve cache directory: {}. Using L1-only cache.",
                    e
                );
                Arc::new(ResponseCache::new())
            }
        };

        Self {
            execution_id,
            graph_id,
            session_id: None,
            memory,
            llm_registry,
            capability_system,
            aam,
            scope_id,
            scope_registry,
            inner_plan_linker: Arc::new(NoOpLinker),
            workflow_spawner: Arc::new(NoOpWorkflowSpawner),
            dag_splicer: Arc::new(NoOpSplicer),
            flow_registry: Arc::new(FlowRegistry::new()),
            current_agent: None,
            instruction_config: InstructionConfig::default(),
            start_time: std::time::Instant::now(),
            metadata: metadata_map,
            token_budget: None,
            consumed_tokens: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            event_emitter: None,
            token_accountant: Arc::new(TokenAccountant::new()),
            timing_tracker: Arc::new(TimingTracker::new()),
            response_cache,
            middlewares: Vec::new(),
            cancellation_token: CancellationToken::new(),
            sandbox_registry: Arc::new(SandboxRegistry::new()),
            process_table: Arc::new(ProcessTable::new()),
            context_stack: None,
            model_router: None,
            agent_pool: Arc::new(AgentPool::new(4, std::time::Duration::from_secs(300))),
            python_tool_bridge: None,
            current_span_id: None,
            current_scope_id: None,
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

    /// Set an execution event emitter.
    pub fn with_event_emitter(mut self, emitter: Option<Arc<dyn ExecutionEventEmitter>>) -> Self {
        self.event_emitter = emitter;
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
            session_id: self.session_id.clone(),
            memory: Arc::clone(&self.memory),
            llm_registry: Arc::clone(&self.llm_registry),
            capability_system: Arc::clone(&self.capability_system),
            aam,
            scope_id,
            scope_registry: Arc::clone(&self.scope_registry),
            inner_plan_linker: Arc::clone(&self.inner_plan_linker),
            workflow_spawner: Arc::clone(&self.workflow_spawner),
            dag_splicer: Arc::clone(&self.dag_splicer),
            flow_registry: Arc::clone(&self.flow_registry),
            current_agent: self.current_agent.as_ref().map(Arc::clone),
            instruction_config: self.instruction_config.clone(),
            start_time: std::time::Instant::now(),
            metadata: metadata_map,
            token_budget: self.token_budget,
            consumed_tokens: Arc::clone(&self.consumed_tokens),
            event_emitter: self.event_emitter.as_ref().map(Arc::clone),
            token_accountant: Arc::clone(&self.token_accountant),
            timing_tracker: Arc::clone(&self.timing_tracker),
            response_cache: Arc::clone(&self.response_cache),
            middlewares: self.middlewares.clone(),
            cancellation_token: self.cancellation_token.child(),
            sandbox_registry: Arc::clone(&self.sandbox_registry),
            process_table: Arc::clone(&self.process_table),
            context_stack: self.context_stack.as_ref().map(Arc::clone),
            model_router: self.model_router.as_ref().map(Arc::clone),
            agent_pool: Arc::clone(&self.agent_pool),
            python_tool_bridge: self.python_tool_bridge.as_ref().map(Arc::clone),
            current_span_id: self.current_span_id.clone(),
            current_scope_id: child_scope_id_for_events,
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
    pub fn with_python_tool_bridge(mut self, bridge: Arc<PythonToolBridge>) -> Self {
        self.python_tool_bridge = Some(bridge);
        self
    }

    pub fn aam(&self) -> &Aam {
        &self.aam
    }

    pub fn scope_id(&self) -> &str {
        &self.scope_id
    }

    /// Get a reference to the flow registry
    pub fn flow_registry(&self) -> &FlowRegistry {
        &self.flow_registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryConfig;
    use apxm_core::types::{execution::Node, values::Value};

    struct TestMiddleware;

    #[async_trait::async_trait]
    impl OperationMiddleware for TestMiddleware {
        async fn around(
            &self,
            _ctx: &ExecutionContext,
            _node: &Node,
            _inputs: Vec<Value>,
            _next: crate::executor::Next<'_>,
        ) -> crate::executor::Result<Value> {
            Ok(Value::Null)
        }
    }

    #[tokio::test]
    async fn test_execution_context_creation() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, Aam::new());

        assert!(!ctx.execution_id.is_empty());
        assert!(!ctx.scope_id.is_empty());
        assert!(ctx.session_id.is_none());
        assert!(ctx.current_agent.is_none());
        assert_eq!(ctx.metadata.get(metadata::SCOPE_ID), Some(&ctx.scope_id));
        assert_eq!(ctx.scope_registry.len(), 1);
        assert!(ctx.middlewares.is_empty());
    }

    #[tokio::test]
    async fn test_execution_context_with_session() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, Aam::new())
            .with_session_id("session_123".to_string())
            .with_metadata("key".to_string(), "value".to_string());

        assert_eq!(ctx.session_id, Some("session_123".to_string()));
        assert_eq!(ctx.metadata.get("key"), Some(&"value".to_string()));
    }

    #[tokio::test]
    async fn test_child_context() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let parent = ExecutionContext::new(memory, llm_registry, capability_system, Aam::new())
            .with_session_id("session_123".to_string())
            .with_graph_id("registered-graph".to_string());
        let child = parent.child();

        // Should have different execution ID
        assert_ne!(child.execution_id, parent.execution_id);
        // Should keep the same graph ID for graph-aware backend hints.
        assert_eq!(child.graph_id, parent.graph_id);
        // Child should execute in its own scope
        assert_ne!(child.scope_id, parent.scope_id);
        // Should inherit session ID
        assert_eq!(child.session_id, parent.session_id);
        // Should inherit current agent identity
        assert_eq!(
            child.current_agent.as_ref().map(|a| a.name.as_str()),
            parent.current_agent.as_ref().map(|a| a.name.as_str())
        );
        // Scope metadata/registry should reflect parent-child relationship
        assert_eq!(
            child.metadata.get(metadata::PARENT_SCOPE_ID),
            Some(&parent.scope_id)
        );
        assert_eq!(parent.scope_registry.len(), 2);
        assert_eq!(
            parent.scope_registry.children_of(parent.scope_id()).len(),
            1
        );
    }

    #[tokio::test]
    async fn test_child_context_inherits_middlewares() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let parent = ExecutionContext::new(memory, llm_registry, capability_system, Aam::new())
            .with_middleware(Arc::new(TestMiddleware));
        let child = parent.child();

        assert_eq!(parent.middlewares.len(), 1);
        assert_eq!(child.middlewares.len(), 1);
    }
}
