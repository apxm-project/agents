//! Driver runtime executor for compiled DAGs.

use std::sync::Arc;
use std::time::Duration;

use apxm_artifact::Artifact;
use apxm_core::types::execution::ExecutionDag;
use apxm_runtime::{
    ExecutionEventEmitter, LoopGuardMiddleware, OperationMiddleware, Runtime,
    RuntimeExecutionResult, TimeoutMiddleware,
};

use crate::{config::MiddlewareConfig, error::DriverError, hooks, linker::LinkerConfig};

mod llm;
use llm::configure_llm_registry;
mod capabilities;
use capabilities::configure_capability_registry;
mod call_skill;
use call_skill::{UnsupportedCallSkillResolver, reject_unsupported_call_skill};
pub mod sandbox;
use sandbox::configure_sandbox_registry;
mod inner_plan;
use apxm_core::utils::build::MlirEnvReport;
use apxm_runtime::NoOpLinker;
use inner_plan::CompilerInnerPlanLinker;
pub mod agents;
use agents::configure_agent_registry;
pub mod link_spawner;
pub use link_spawner::{LinkAgentSpawner, LinkHostRegistry, RelaySessionHandle};
mod workflow_spawn;
use workflow_spawn::DriverWorkflowSpawner;

/// Parse an operator-configured routing target string into a
/// [`apxm_runtime::RoutingTarget`], defaulting to `Balanced` for `None` or
/// unrecognized values.
fn parse_routing_target(target: Option<&str>) -> apxm_runtime::RoutingTarget {
    use apxm_runtime::RoutingTarget;
    match target
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("cost") => RoutingTarget::Cost,
        Some("latency") => RoutingTarget::Latency,
        Some("quality") => RoutingTarget::Quality,
        _ => RoutingTarget::Balanced,
    }
}

/// Build the [`apxm_runtime::ModelRouterConfig`]'s `operation_policies` from
/// `config.chat.routing.operation_routes` — the shared config surface. Every
/// runtime host (driver/CLI, server) that calls `Runtime::init_model_router`
/// should feed it through this helper so the per-operation routing policy
/// (ASK/THINK/etc. -> configured backend/model/target) is consulted
/// consistently regardless of transport.
pub fn operation_policies_from_config(
    apxm_config: &crate::config::ApXmConfig,
) -> Vec<apxm_runtime::OperationPolicy> {
    apxm_config
        .chat
        .routing
        .operation_routes
        .iter()
        .filter_map(|(op_str, route)| {
            let operation = op_str.parse::<apxm_core::types::AISOperationType>().ok()?;
            Some(apxm_runtime::OperationPolicy {
                operation,
                model: route.model.clone(),
                backend: route.backend.clone(),
                target: parse_routing_target(route.target.as_deref()),
            })
        })
        .collect::<Vec<_>>()
}

/// Driver runtime executor for compiled DAGs.
pub struct RuntimeExecutor {
    runtime: Arc<Runtime>,
    configured_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
}

impl RuntimeExecutor {
    pub async fn new(config: &LinkerConfig) -> Result<Self, DriverError> {
        let mut runtime = Runtime::new(config.runtime_config.clone())
            .await
            .map_err(DriverError::Runtime)?;

        configure_llm_registry(runtime.llm_registry(), &config.apxm_config).await?;
        configure_capability_registry(runtime.capability_system_arc(), &config.apxm_config)?;

        // Initialize ModelRouter after LLM backends are registered.
        // Populate operation_policies from config.chat.routing.operation_routes so
        // the ModelRouter correctly routes ASK/THINK/etc. to their configured backends.
        let mut router_config = apxm_runtime::ModelRouterConfig::default();
        router_config.operation_policies = operation_policies_from_config(&config.apxm_config);

        runtime
            .init_model_router(router_config)
            .map_err(DriverError::Runtime)?;

        // Instantiate ProfileRegistry beside ModelRouter: loads
        // `~/.apxm/model_profiles.toml` so `model_profile` node attributes
        // resolve to a candidate model before `ModelRouter::select` runs.
        runtime.init_profile_registry();

        // The driver/CLI has no SkillLibrary-backed skill catalog (out of
        // scope for W2.4 — see `call_skill` module doc). Install the named
        // rejection resolver so a `CALL_SKILL` that somehow reaches dispatch
        // still fails fail-closed with a distinguishable tag instead of the
        // generic `NoOpSkillResolver` default. `reject_unsupported_call_skill`
        // (called from every execute* entry point below) is the primary,
        // pre-dispatch admission gate; this is the defense-in-depth fallback.
        runtime.set_skill_resolver(std::sync::Arc::new(UnsupportedCallSkillResolver));

        let sandbox_registry = configure_sandbox_registry();
        runtime.set_sandbox_registry(std::sync::Arc::clone(&sandbox_registry));

        configure_agent_registry(
            runtime.process_table(),
            runtime.capability_system_arc(),
            sandbox_registry,
        )
        .await?;

        runtime.set_instruction_config(config.apxm_config.instruction.clone());
        runtime.set_middlewares(build_middlewares(&config.apxm_config.middlewares));
        // Conversation-memory middleware fires the pre_turn/post_turn/post_ask
        // lifecycle hooks and accrues the turn transcript. Register it on the
        // driver/CLI path too (the server already does) so author hooks fire with
        // transport parity (constitution #1), not only over the HTTP surface.
        runtime.add_middleware(std::sync::Arc::new(
            apxm_runtime::ConversationMemoryMiddleware::new(),
        ));

        // Use CompilerInnerPlanLinker when MLIR is available, otherwise fall back to
        // NoOpLinker (graph-direct mode). Same behavior as Linker when MLIR is unavailable.
        let report = MlirEnvReport::detect();
        report.apply_env();
        if report.is_ready() {
            match CompilerInnerPlanLinker::new(config.pipeline_config.clone()) {
                Ok(linker) => {
                    apxm_core::log_info!("driver", "MLIR inner-plan linker initialized");
                    runtime.set_inner_plan_linker(Arc::new(linker));
                }
                Err(e) => {
                    apxm_core::log_info!(
                        "driver",
                        "MLIR inner-plan linker init failed ({}); using NoOp",
                        e
                    );
                    runtime.set_inner_plan_linker(Arc::new(NoOpLinker));
                }
            }
        } else {
            apxm_core::log_info!(
                "driver",
                "MLIR not available; using NoOp inner-plan linker (graph-direct mode)"
            );
            runtime.set_inner_plan_linker(Arc::new(NoOpLinker));
        }

        let configured_emitter = hooks::emitter_from_config(&config.apxm_config.hooks);
        let workflow_spawner = Arc::new(DriverWorkflowSpawner::new(
            configured_emitter.as_ref().map(Arc::clone),
        ));
        runtime.set_workflow_spawner(workflow_spawner.clone());
        let runtime = Arc::new(runtime);
        workflow_spawner.attach_runtime(Arc::downgrade(&runtime));

        Ok(Self {
            runtime,
            configured_emitter,
        })
    }

    pub async fn execute(&self, dag: ExecutionDag) -> Result<RuntimeExecutionResult, DriverError> {
        reject_unsupported_call_skill(std::slice::from_ref(&dag))?;
        self.runtime
            .execute_with_event_emitter(dag, self.compose_emitter(None))
            .await
            .map_err(DriverError::Runtime)
    }

    /// Execute an artifact with @entry flow requirement.
    ///
    /// This method will return an error if the artifact doesn't have an @entry flow.
    pub async fn execute_artifact_auto(
        &self,
        artifact: Artifact,
    ) -> Result<RuntimeExecutionResult, DriverError> {
        reject_unsupported_call_skill(artifact.dags())?;
        self.runtime
            .execute_artifact_with_session_and_emitter(
                artifact,
                vec![],
                None,
                self.compose_emitter(None),
                None,
            )
            .await
            .map_err(DriverError::Runtime)
    }

    /// Execute an artifact with provided arguments for entry flow parameters.
    ///
    /// Validates that the number of arguments matches the entry flow's parameter count.
    pub async fn execute_artifact_with_args(
        &self,
        artifact: Artifact,
        args: Vec<String>,
    ) -> Result<RuntimeExecutionResult, DriverError> {
        reject_unsupported_call_skill(artifact.dags())?;
        self.runtime
            .execute_artifact_with_session_and_emitter(
                artifact,
                args,
                None,
                self.compose_emitter(None),
                None,
            )
            .await
            .map_err(DriverError::Runtime)
    }

    /// Execute an artifact with arguments and an optional event emitter.
    pub async fn execute_artifact_with_emitter(
        &self,
        artifact: Artifact,
        args: Vec<String>,
        emitter: Option<Arc<dyn ExecutionEventEmitter>>,
        session_dir: Option<String>,
    ) -> Result<RuntimeExecutionResult, DriverError> {
        reject_unsupported_call_skill(artifact.dags())?;
        self.runtime
            .execute_artifact_with_session_and_emitter(
                artifact,
                args,
                None,
                self.compose_emitter(emitter),
                session_dir,
            )
            .await
            .map_err(DriverError::Runtime)
    }

    fn compose_emitter(
        &self,
        emitter: Option<Arc<dyn ExecutionEventEmitter>>,
    ) -> Option<Arc<dyn ExecutionEventEmitter>> {
        hooks::compose_emitters(emitter, self.configured_emitter.as_ref().map(Arc::clone))
    }

    /// Get the LLM registry from the runtime
    pub fn llm_registry(&self) -> Arc<apxm_backends::LLMRegistry> {
        self.runtime.llm_registry_arc()
    }

    /// Get list of available capability names
    ///
    /// Returns the names of all capabilities registered in the runtime.
    /// Useful for validation, UI display, and constraining LLM generation.
    pub fn capability_names(&self) -> Vec<String> {
        self.runtime.capability_system().list_capability_names()
    }

    /// Get capability system reference (for advanced usage)
    ///
    /// Provides direct access to the capability system for
    /// advanced operations like capability inspection or registration.
    pub fn capability_system(&self) -> &apxm_runtime::capability::CapabilitySystem {
        self.runtime.capability_system()
    }

    /// Get memory system reference
    ///
    /// Provides access to the memory system for integration with session output
    pub fn memory_system(&self) -> Arc<apxm_runtime::memory::MemorySystem> {
        self.runtime.memory_system_arc()
    }

    /// Gracefully shut down the runtime, closing all agent processes.
    ///
    /// This method should be called when execution completes to ensure
    /// all ACP sessions are properly terminated. Without this, agent
    /// processes may leak.
    pub fn shutdown(&self) {
        self.runtime.shutdown();
    }
}

/// Install the driver workflow-spawn bridge on a server-owned runtime.
///
/// APXM server constructs `Runtime` directly instead of going through
/// [`RuntimeExecutor`], but server/MCP graph execution still needs the same
/// `WORKFLOW_SPAWN` host bridge as the CLI. The bridge keeps only a weak handle
/// back to the runtime to avoid a reference cycle.
pub fn install_workflow_spawner(
    runtime: &mut Arc<Runtime>,
    configured_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
) -> Result<(), apxm_runtime::RuntimeError> {
    let installed = {
        let runtime = Arc::get_mut(runtime).ok_or_else(|| {
            apxm_runtime::RuntimeError::State(
                "cannot install workflow spawner after runtime has been shared".to_string(),
            )
        })?;
        install_workflow_spawner_unattached(runtime, configured_emitter)
    };
    installed.attach_runtime(runtime);
    Ok(())
}

pub struct InstalledWorkflowSpawner {
    spawner: Arc<DriverWorkflowSpawner>,
}

impl InstalledWorkflowSpawner {
    pub fn attach_runtime(&self, runtime: &Arc<Runtime>) {
        self.spawner.attach_runtime(Arc::downgrade(runtime));
    }
}

pub fn install_workflow_spawner_unattached(
    runtime: &mut Runtime,
    configured_emitter: Option<Arc<dyn ExecutionEventEmitter>>,
) -> InstalledWorkflowSpawner {
    let workflow_spawner = Arc::new(DriverWorkflowSpawner::new(configured_emitter));
    runtime.set_workflow_spawner(workflow_spawner.clone());
    InstalledWorkflowSpawner {
        spawner: workflow_spawner,
    }
}

fn build_middlewares(configs: &[MiddlewareConfig]) -> Vec<Arc<dyn OperationMiddleware>> {
    configs
        .iter()
        .map(|config| match config {
            MiddlewareConfig::Timeout { default_timeout_ms } => Arc::new(TimeoutMiddleware::new(
                default_timeout_ms.map(Duration::from_millis),
            ))
                as Arc<dyn OperationMiddleware>,
            MiddlewareConfig::LoopGuard { max_repeats } => {
                Arc::new(LoopGuardMiddleware::new(*max_repeats)) as Arc<dyn OperationMiddleware>
            }
        })
        .collect()
}

#[cfg(test)]
mod operation_policy_tests {
    use super::operation_policies_from_config;
    use crate::config::{ApXmConfig, OperationRouteConfig};

    // every runtime host that calls `Runtime::init_model_router` must
    // feed it `operation_policies` derived from the same shared config
    // surface (`chat.routing.operation_routes`) — this is the mechanism the
    // driver/CLI path already relied on, and the server path (see
    // apxm-server's `runtime_setup.rs`) now reuses this exact helper.
    #[test]
    fn builds_operation_policies_from_chat_routing_config() {
        let mut config = ApXmConfig::default();
        config.chat.routing.operation_routes.insert(
            "ask".to_string(),
            OperationRouteConfig {
                backend: Some("anthropic".to_string()),
                model: Some("claude-sonnet".to_string()),
                target: Some("quality".to_string()),
            },
        );

        let policies = operation_policies_from_config(&config);

        assert_eq!(policies.len(), 1);
        let policy = &policies[0];
        assert_eq!(policy.operation, apxm_core::types::AISOperationType::Ask);
        assert_eq!(policy.backend.as_deref(), Some("anthropic"));
        assert_eq!(policy.model.as_deref(), Some("claude-sonnet"));
        assert_eq!(policy.target, apxm_runtime::RoutingTarget::Quality);
    }

    #[test]
    fn skips_unparseable_operation_names_and_defaults_missing_target() {
        let mut config = ApXmConfig::default();
        config.chat.routing.operation_routes.insert(
            "not_a_real_operation".to_string(),
            OperationRouteConfig {
                backend: Some("anthropic".to_string()),
                model: None,
                target: None,
            },
        );
        config.chat.routing.operation_routes.insert(
            "think".to_string(),
            OperationRouteConfig {
                backend: Some("openai".to_string()),
                model: None,
                target: None,
            },
        );

        let policies = operation_policies_from_config(&config);

        assert_eq!(policies.len(), 1);
        assert_eq!(
            policies[0].operation,
            apxm_core::types::AISOperationType::Think
        );
        assert_eq!(policies[0].target, apxm_runtime::RoutingTarget::Balanced);
    }

    #[test]
    fn empty_operation_routes_yield_empty_policies() {
        let config = ApXmConfig::default();
        assert!(operation_policies_from_config(&config).is_empty());
    }
}

/// Required evidence (W2.4): a `CALL_SKILL` node under `--driver` fails with
/// the named rejection at admission time, before any node dispatches -- not
/// merely a generic mid-execution failure, and with zero partial-execution
/// side effects from nodes that would otherwise have run first.
#[cfg(test)]
mod call_skill_admission_tests {
    use super::RuntimeExecutor;
    use crate::config::ApXmConfig;
    use crate::linker::LinkerConfig;
    use apxm_core::constants::graph::attrs as graph_attrs;
    use apxm_core::types::execution::{ExecutionDag, Node};
    use apxm_core::types::values::Value;
    use apxm_core::types::AISOperationType;
    use apxm_runtime::executor::skill_resolver::CALL_SKILL_UNSUPPORTED_HERE_TAG;

    async fn test_executor() -> RuntimeExecutor {
        // `RuntimeExecutor::new` requires at least one LLM backend to be
        // configured; the mock backend keeps this test hermetic (no network,
        // no real provider credentials) since these tests never dispatch a
        // node that talks to an LLM.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var(apxm_core::constants::env::APXM_MOCK_BACKEND, "1");
        }
        let config = LinkerConfig::from_apxm_config(ApXmConfig::default());
        RuntimeExecutor::new(&config)
            .await
            .expect("RuntimeExecutor::new should succeed with default config")
    }

    #[tokio::test]
    async fn call_skill_node_rejected_before_any_node_dispatches() {
        let executor = test_executor().await;

        let marker = std::env::temp_dir().join(format!(
            "apxm-driver-call-skill-admission-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&marker);

        // Node 1 would perform a real side effect (write `marker` to disk)
        // if it ever dispatched. Node 2 is the unsupported CALL_SKILL. Both
        // are handed to `execute()` in one DAG; if admission worked lazily
        // (only rejecting once dispatch reaches the CALL_SKILL node) node 1
        // could still have run first.
        let mut write_node = Node::new(1, AISOperationType::InvCap);
        write_node.attributes.insert(
            graph_attrs::CAPABILITY.to_string(),
            Value::String("write".to_string()),
        );
        write_node.attributes.insert(
            graph_attrs::PARAMS_JSON.to_string(),
            Value::String(format!(
                "{{\"path\":\"{}\",\"content\":\"side-effect\"}}",
                marker.display()
            )),
        );
        let call_skill_node = Node::new(2, AISOperationType::CallSkill);

        let mut dag = ExecutionDag::new();
        dag.nodes = vec![write_node, call_skill_node];

        let result = executor.execute(dag).await;

        let error = result.expect_err("CALL_SKILL under --driver must be rejected");
        let message = error.to_string();
        assert!(
            message.contains(CALL_SKILL_UNSUPPORTED_HERE_TAG),
            "expected the named rejection tag, got: {message}"
        );
        assert!(
            !marker.exists(),
            "node 1's write side effect must not have run: admission must reject before any \
             node dispatches"
        );
    }

    #[tokio::test]
    async fn dag_without_call_skill_is_not_rejected_by_admission_gate() {
        let executor = test_executor().await;
        let nop_dag = {
            let mut dag = ExecutionDag::new();
            dag.nodes = vec![Node::new(1, AISOperationType::Nop)];
            dag
        };

        // The admission gate itself must not reject this DAG. (The overall
        // execute() call may still fail for unrelated runtime-setup reasons
        // in a minimal test config; the point is that failure, if any, is
        // not this WP's CALL_SKILL rejection.)
        let result = executor.execute(nop_dag).await;
        if let Err(error) = result {
            assert!(
                !error.to_string().contains(CALL_SKILL_UNSUPPORTED_HERE_TAG),
                "a DAG with no CALL_SKILL node must not trip the CALL_SKILL admission gate"
            );
        }
    }
}
