//! APXM Server API — the trait interface between the server and the agents runtime.
//!
//! The concrete implementation lives in apxm-server's runtime_setup.rs.
//! Server handlers import only this crate, not apxm-runtime directly.

use std::sync::Arc;

// Re-export the types that server handlers need from runtime
pub use apxm_runtime::{
    Runtime,
    RuntimeExecutionResult,
    ExecutionEventEmitter,
    EmitterAdapter,
    RuntimeConfig,
    SchedulerConfig,
    ModelRouterConfig,
    TimeoutMiddleware,
    TokenBudgetMiddleware,
    ConversationMemoryMiddleware,
    PermissionInterceptor,
    Goal,
    TransitionLabel,
};
pub use apxm_runtime::capability::{
    CapabilitySystem,
};
pub use apxm_runtime::capability::executor::{
    CapabilityExecutor,
    CapabilityResult,
};
pub use apxm_runtime::capability::metadata::CapabilityMetadata;
pub use apxm_runtime::capability::builtins::guard_url_ssrf;
pub use apxm_runtime::capability::builtins::{FiredSchedule, OnFire};
pub use apxm_runtime::executor::session_ledger;
pub use apxm_runtime::executor::session_ledger::SessionLedger;
pub use apxm_runtime::{ProcessTable, AgentPool};
pub use apxm_backends::LLMRegistry;
pub use apxm_compiler::{
    AirModule, AirEdge, AirNode,
    Context as CompilerContext,
    Pipeline as CompilerPipeline,
};
pub use apxm_backends::{LLMRequest, LLMResponse, BackendRegistration};
pub use apxm_backends::{Message as LLMMessage, Role as LLMRole, StreamChunk, ToolChoice, ToolDefinition};
pub use apxm_backends::llm::wire::response_metadata;
pub use apxm_runtime::{CallSkillRequest, CallSkillResult, SkillResolver};
pub use apxm_runtime::metadata_keys;
pub use apxm_runtime::memory::{MemorySpace, MemorySystem};

/// The agent runtime interface exposed to the server.
/// Server handlers depend on this trait, not on Arc<Runtime> directly.
/// The concrete implementation is RuntimeApiAdapter in apxm-server's runtime_setup.rs.
pub trait AgentRuntimeApi: Send + Sync + 'static {
    /// Access the capability system for capability registration and discovery.
    fn capability_system(&self) -> Arc<CapabilitySystem>;
    fn capability_system_arc(&self) -> Arc<CapabilitySystem>;

    /// Access the LLM registry for LLM backend operations.
    fn llm_registry(&self) -> Arc<LLMRegistry>;

    /// Access the process table for agent process management.
    fn process_table(&self) -> Arc<ProcessTable>;

    /// Access the agent pool.
    fn agent_pool(&self) -> Arc<AgentPool>;

    /// Access the memory system for session compaction and scoped reads/writes.
    fn memory(&self) -> &MemorySystem;

    /// The underlying Runtime for operations that need full access.
    /// Only used by wiring code (runtime_setup, startup); not by handlers.
    fn runtime(&self) -> &Runtime;
    fn runtime_arc(&self) -> Arc<Runtime>;

    /// Shut down the runtime.
    fn shutdown(&self);
}

/// Concrete implementation wrapping Arc<Runtime>.
pub struct RuntimeApiAdapter(pub Arc<Runtime>);

impl AgentRuntimeApi for RuntimeApiAdapter {
    fn capability_system(&self) -> Arc<CapabilitySystem> {
        self.0.capability_system_arc()
    }

    fn capability_system_arc(&self) -> Arc<CapabilitySystem> {
        self.0.capability_system_arc()
    }

    fn llm_registry(&self) -> Arc<LLMRegistry> {
        self.0.llm_registry_arc()
    }

    fn process_table(&self) -> Arc<ProcessTable> {
        self.0.process_table_arc()
    }

    fn agent_pool(&self) -> Arc<AgentPool> {
        self.0.agent_pool_arc()
    }

    fn memory(&self) -> &MemorySystem {
        self.0.memory()
    }

    fn runtime(&self) -> &Runtime {
        &self.0
    }

    fn runtime_arc(&self) -> Arc<Runtime> {
        Arc::clone(&self.0)
    }

    fn shutdown(&self) {
        self.0.shutdown();
    }
}
