//! Execute canonical `apxm.air` through the canonical runtime driver.

// Declared inline rather than as `canonical_execute/capability_port.rs`: this
// file is reached through two module paths (`commands::canonical_execute` and
// the `#[path]` re-declaration in `lib.rs`), and those two paths resolve a
// child module file to two different directories.
mod capability_port {
    //! The admitted Capability port for canonical local execution.
    //!
    //! This is the seam that turns `capability.invoke` from a refusal into a real
    //! effect: it carries an exact [`CapabilityRequest`] into the `apxm-capability`
    //! runtime capability system and carries the typed [`CapabilityOutcome`] back.
    //!
    //! Three boundaries are crossed here and each one is explicit rather than
    //! best-effort:
    //!
    //! 1. **Two value contracts.** The port carries canonical JSON bytes
    //!    ([`apxm_program::CanonicalCapabilityArguments`]); the capability system
    //!    takes a named `HashMap<String, apxm_core::Value>`. The mapping is total
    //!    only for a JSON *object* root, so every other root is a definite failure
    //!    with a diagnostic that names why, never a silently-wrapped argument.
    //! 2. **Three outcomes from two.** [`apxm_capability::CapabilitySystem::invoke`]
    //!    returns `Result<Value, RuntimeError>`. A timeout is the one error that
    //!    leaves the effect *unobserved*, so it maps to
    //!    [`CapabilityOutcome::OutcomeUnknown`]; collapsing it into `Failed` would
    //!    assert the effect did not happen when the runtime does not know that.
    //! 3. **What local execution may do at all.** The canonical composition root
    //!    has no admitted sandbox backend and no Auth/Server-issued grants, so it
    //!    admits only the capability surface whose own metadata declares it
    //!    read-only. Everything else is denied at the interceptor chokepoint,
    //!    before any argument reaches an implementation.

    use std::collections::{BTreeSet, HashMap};
    use std::sync::Arc;

    use apxm_capability::CapabilitySystem;
    use apxm_capability::builtins::{BashConfig, ToolsConfig, register_standard_tools};
    use apxm_capability::interceptor::{CapabilityInterceptor, InterceptDecision};
    use apxm_core::error::RuntimeError;
    use apxm_core::types::values::{Value as CapabilityValue, ValueError};
    use apxm_execution::{CapabilityOutcome, CapabilityPort, CapabilityRequest};
    use async_trait::async_trait;

    /// Interceptor name reported by the local admission gate.
    const LOCAL_ADMISSION_INTERCEPTOR: &str = "canonical-local-admission";

    /// Standard-tool configuration for a composition root with no admitted sandbox.
    ///
    /// `bash` declares an `ExecRequest` through
    /// `CapabilityExecutor::to_exec_request`, so `CapabilitySystem` *always* routes
    /// it through a sandbox backend and `BashCapability::execute` refuses direct
    /// execution outright. With no sandbox registry bound, registering `bash` would
    /// publish a capability that fails 100% of the time, so the local root leaves it
    /// unregistered instead of advertising it. A sandbox-backed local profile is the
    /// thing that turns it back on, not a config toggle here.
    fn local_tools_config() -> ToolsConfig {
        ToolsConfig {
            bash: BashConfig {
                enabled: false,
                ..BashConfig::default()
            },
            ..ToolsConfig::default()
        }
    }

    /// Deny every capability outside the locally admitted set, at the chokepoint
    /// every invocation passes through.
    ///
    /// The admitted set is captured once at construction from the registered
    /// capabilities' own metadata, so the gate holds no back-reference to the
    /// system it guards.
    struct LocalAdmissionPolicy {
        admitted: BTreeSet<String>,
    }

    #[async_trait]
    impl CapabilityInterceptor for LocalAdmissionPolicy {
        fn name(&self) -> &'static str {
            LOCAL_ADMISSION_INTERCEPTOR
        }

        async fn pre_invoke(
            &self,
            name: &str,
            _args: &HashMap<String, CapabilityValue>,
        ) -> InterceptDecision {
            if self.admitted.contains(name) {
                return InterceptDecision::allow();
            }
            InterceptDecision::deny(format!(
                "capability '{name}' is not admitted by canonical local execution: the local \
                 composition root binds no sandbox backend and no issued Capability grant, so it \
                 admits only the read-only capability surface [{}]",
                self.admitted
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
    }

    /// The locally admitted capability names: those whose own metadata declares
    /// them read-only.
    fn admitted_capability_names(system: &CapabilitySystem) -> BTreeSet<String> {
        system
            .list_capabilities()
            .into_iter()
            .filter(|capability| capability.read_only)
            .map(|capability| capability.name)
            .collect()
    }

    /// The canonical local Capability port.
    pub struct LocalCapabilityPort {
        system: CapabilitySystem,
    }

    impl LocalCapabilityPort {
        /// Build the local capability surface: register the standard tools that can
        /// run without a sandbox, then gate them behind the read-only admission
        /// policy.
        ///
        /// # Errors
        ///
        /// Returns the registration error if a standard tool cannot be registered.
        pub fn new() -> Result<Self, RuntimeError> {
            let system = CapabilitySystem::new();
            register_standard_tools(&system, &local_tools_config())?;
            system.register_interceptor(Arc::new(LocalAdmissionPolicy {
                admitted: admitted_capability_names(&system),
            }));
            Ok(Self { system })
        }

        /// Capability names this port admits, in canonical order.
        #[cfg(test)]
        fn admitted_names(&self) -> BTreeSet<String> {
            admitted_capability_names(&self.system)
        }

        /// Every capability registered on the local surface, admitted or not.
        #[cfg(test)]
        fn registered_names(&self) -> BTreeSet<String> {
            self.system.list_capability_names().into_iter().collect()
        }
    }

    #[async_trait]
    impl CapabilityPort for LocalCapabilityPort {
        async fn invoke(&self, request: CapabilityRequest) -> CapabilityOutcome {
            let capability_ref = request.capability_ref();
            let arguments = match named_arguments(&request) {
                Ok(arguments) => arguments,
                Err(message) => return CapabilityOutcome::Failed { message },
            };
            match self.system.invoke(capability_ref, arguments).await {
                Ok(value) => match capability_result_text(value) {
                    Ok(result) => CapabilityOutcome::Completed { result },
                    Err(error) => CapabilityOutcome::Failed {
                        message: format!(
                            "capability '{capability_ref}' returned a result with no canonical JSON \
                             representation: {error}"
                        ),
                    },
                },
                // A timeout elapses without observing whether the effect ran. That
                // is exactly the runtime's uncertainty semantics, and reporting
                // `Failed` here would assert the effect did not happen.
                Err(RuntimeError::Timeout { timeout, .. }) => CapabilityOutcome::OutcomeUnknown {
                    message: format!(
                        "capability '{capability_ref}' reported no outcome within {timeout:?}; whether \
                         the effect was applied is unobserved"
                    ),
                },
                // Every other error is raised either before the implementation is
                // reached (not found, denied, schema-invalid) or by the
                // implementation itself reporting failure, so the effect state is
                // known.
                Err(error) => CapabilityOutcome::Failed {
                    message: error.to_string(),
                },
            }
        }
    }

    /// Decode the canonical argument bytes into the capability system's named
    /// argument map.
    ///
    /// The canonical bytes are produced by `serde_json`, so no non-finite float can
    /// survive round-tripping and the `serde_json::Value` → `apxm_core::Value`
    /// conversion is total for them. The one root that has no mapping is a
    /// non-object: capability arguments are named, and inventing a name for a bare
    /// scalar or array would fabricate an argument the author never wrote.
    fn named_arguments(
        request: &CapabilityRequest,
    ) -> Result<HashMap<String, CapabilityValue>, String> {
        let capability_ref = request.capability_ref();
        let decoded = request.arguments().value().map_err(|error| {
            format!(
                "canonical arguments for capability '{capability_ref}' are not decodable: {error}"
            )
        })?;
        let serde_json::Value::Object(fields) = decoded else {
            return Err(format!(
                "canonical arguments for capability '{capability_ref}' must be a JSON object; the \
                 admitted argument contract is a named map and a non-object root has no named-argument \
                 mapping"
            ));
        };
        let mut arguments = HashMap::with_capacity(fields.len());
        for (name, value) in fields {
            let value = CapabilityValue::try_from(value).map_err(|error| {
                format!(
                    "argument '{name}' of capability '{capability_ref}' has no runtime value \
                     mapping: {error}"
                )
            })?;
            arguments.insert(name, value);
        }
        Ok(arguments)
    }

    /// Project a capability result onto the port's `Completed { result: String }`.
    ///
    /// A string result is carried verbatim so a `read` returns file contents rather
    /// than a quoted JSON string; every other shape is rendered as canonical JSON.
    fn capability_result_text(value: CapabilityValue) -> Result<String, ValueError> {
        match value {
            CapabilityValue::String(text) => Ok(text),
            other => Ok(serde_json::to_string(&other.to_json()?)
                .expect("a serde_json::Value always serializes")),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use apxm_program::CapabilityInvocationAuthority;

        fn authority() -> CapabilityInvocationAuthority {
            CapabilityInvocationAuthority::new(
                "test.acting-principal",
                "test.agent-identity",
                "test.capability-grant",
                Vec::new(),
            )
            .expect("test authority")
        }

        fn request(capability_ref: &str, arguments: serde_json::Value) -> CapabilityRequest {
            CapabilityRequest::prepare(
                capability_ref,
                "ArgumentValue",
                arguments,
                "test.invocation",
                "test.node-execution",
                authority(),
            )
            .expect("test capability request")
        }

        #[test]
        fn the_local_surface_registers_no_capability_that_needs_a_sandbox() {
            let port = LocalCapabilityPort::new().expect("local capability port");
            let registered = port.registered_names();
            assert!(
                !registered.contains("bash"),
                "bash always routes through a sandbox backend, so an unsandboxed local \
                 surface must not publish it: {registered:?}"
            );
        }

        #[test]
        fn only_the_read_only_surface_is_admitted() {
            let port = LocalCapabilityPort::new().expect("local capability port");
            assert!(port.admitted_names().contains("read"));
            assert!(
                port.registered_names().contains("write"),
                "write is registered so its denial is a policy decision, not a registry miss"
            );
            assert!(
                !port.admitted_names().contains("write"),
                "write is not read-only, so local execution must not admit it"
            );
        }

        #[tokio::test]
        async fn an_authored_read_reaches_the_read_capability() {
            let directory = tempfile::tempdir().expect("temporary directory");
            let file = directory.path().join("payload.txt");
            std::fs::write(&file, "canonical capability payload\n").expect("write payload");

            let port = LocalCapabilityPort::new().expect("local capability port");
            let outcome = port
                .invoke(request(
                    "read",
                    serde_json::json!({"file_path": file.to_str().expect("utf-8 path")}),
                ))
                .await;

            let CapabilityOutcome::Completed { result } = outcome else {
                panic!("an admitted read must complete: {outcome:?}");
            };
            assert!(
                result.contains("canonical capability payload"),
                "the read capability returns file contents: {result}"
            );
        }

        #[tokio::test]
        async fn a_capability_outside_the_admitted_surface_fails_closed() {
            let directory = tempfile::tempdir().expect("temporary directory");
            let target = directory.path().join("must-not-exist.txt");

            let port = LocalCapabilityPort::new().expect("local capability port");
            let outcome = port
                .invoke(request(
                    "write",
                    serde_json::json!({
                        "file_path": target.to_str().expect("utf-8 path"),
                        "content": "this effect must never be applied",
                    }),
                ))
                .await;

            let CapabilityOutcome::Failed { message } = outcome else {
                panic!("a capability outside the admitted surface must fail: {outcome:?}");
            };
            assert!(
                message.contains("not admitted by canonical local execution"),
                "the denial names the local admission policy: {message}"
            );
            assert!(
                !target.exists(),
                "the denial happens before the implementation receives its arguments"
            );
        }

        #[tokio::test]
        async fn a_non_object_argument_root_is_a_definite_failure() {
            let port = LocalCapabilityPort::new().expect("local capability port");
            let outcome = port
                .invoke(request("read", serde_json::json!(["not", "a", "map"])))
                .await;

            let CapabilityOutcome::Failed { message } = outcome else {
                panic!("a non-object argument root has no named-argument mapping: {outcome:?}");
            };
            assert!(
                message.contains("must be a JSON object"),
                "the failure names the unmappable argument root: {message}"
            );
        }

        #[tokio::test]
        async fn an_unregistered_capability_fails_closed() {
            let port = LocalCapabilityPort::new().expect("local capability port");
            let outcome = port
                .invoke(request("cap.absent", serde_json::json!({})))
                .await;
            assert!(
                matches!(outcome, CapabilityOutcome::Failed { .. }),
                "an unregistered capability never completes: {outcome:?}"
            );
        }
    }
}

// Declared inline for the same reason as `capability_port` above: this file is
// reached through two module paths, which resolve a child module file to two
// different directories.
mod model_port {
    //! The admitted model-inference port for canonical local execution.
    //!
    //! This is the seam that turns `model.call` from a sentinel into a real
    //! inference: it carries an exact [`ModelCallRequest`] into an
    //! [`apxm_backends::llm::LLMBackend`] through the registry that binds one
    //! exact model reference to one backend, and carries a typed
    //! [`AttemptDisposition`] back.
    //!
    //! Four boundaries are crossed here and each is explicit rather than
    //! best-effort:
    //!
    //! 1. **Sync port, async backends.** [`ModelInferencePort::attempt`] is
    //!    synchronous; every `LLMBackend` method is `async`. The bridge owns a
    //!    *separate* tokio runtime and never calls `block_on` — see
    //!    [`BackendRuntime`] for why that distinction is load bearing.
    //! 2. **Two request contracts.** The port receives the authored SSA request
    //!    value; the backend takes an [`LLMRequest`]. The mapping is closed and
    //!    rejects an unknown field rather than dropping it, because dropping it
    //!    would send a request the author did not write.
    //! 3. **Configured or not.** With no admitted binding for the authored
    //!    target the port fails *before send* with a `Configuration` typed error
    //!    that names the target, the roster it consulted, and every reason a
    //!    registration was skipped. It never substitutes a stub response.
    //! 4. **Sent or not.** `LLMBackend::generate` is one opaque await, so the
    //!    adapter cannot observe when the request left the client. Every failure
    //!    raised by the adapter *before* that await is definite
    //!    [`AttemptDisposition::FailedBeforeSend`]; every failure raised *by*
    //!    that await is [`AttemptDisposition::FailedAfterSend`], which the
    //!    non-idempotent retry policy commits as `model_outcome_unknown` rather
    //!    than asserting the request never reached the provider.

    use std::collections::BTreeSet;
    use std::future::Future;
    use std::sync::{Arc, Mutex};

    use anyhow::Result;
    use apxm_backend_registry::BackendStore;
    #[cfg(test)]
    use apxm_backends::llm::backends::LLMBackend;
    use apxm_backends::llm::{
        BackendRegistration, LLMRegistry, LLMRequest, LLMResponse, Message, Role,
    };
    use apxm_core::types::FinishReason;
    use apxm_inference::{
        AttemptDisposition, ErrorCategory, IdempotencyKey, ModelCallPreparation, ModelCallRequest,
        ModelCallRequestMetadata, ModelCallRequestMetadataPort, ModelContextEnvelopeRef,
        ModelInferencePort, ModelStreamMode, TypedError, Usage,
    };
    use serde::Deserialize;
    use serde_json::{Value, json};

    /// Prefix of the sealed model-context identity minted by the local root.
    const LOCAL_MODEL_CONTEXT_PREFIX: &str = "apxm.canonical.local.model-context.";
    /// The one idempotency scope canonical local execution commits within.
    const LOCAL_IDEMPOTENCY_SCOPE_REF: &str = "apxm.canonical.local.idempotency-scope";

    /// Host-owned metadata for one already-admitted local model request.
    ///
    /// Every field is a value the runtime has already computed for this exact
    /// effect, so nothing here is invented:
    ///
    /// - the sealed context digest **is** the digest of the Context this call
    ///   was prepared against, and the local composition root is its sealer;
    /// - the idempotency key **is** the request digest, so the key is stable
    ///   across the retries that reuse one request identity and differs for any
    ///   request whose identity differs;
    /// - the delivery mode is `Buffered` because this port returns one buffered
    ///   disposition. `ModelStreamPort` is a separate seam and is not bound
    ///   here, so declaring `Streamed` would promise a stream nothing delivers.
    pub struct LocalModelRequestMetadata;

    impl ModelCallRequestMetadataPort for LocalModelRequestMetadata {
        fn materialize(
            &self,
            preparation: &ModelCallPreparation,
        ) -> Result<ModelCallRequestMetadata, TypedError> {
            Ok(ModelCallRequestMetadata {
                model_context_envelope_ref: ModelContextEnvelopeRef {
                    context_id: format!(
                        "{LOCAL_MODEL_CONTEXT_PREFIX}{}",
                        preparation.node_execution_id().as_str()
                    ),
                    sealed_digest: preparation.context_digest().into(),
                },
                idempotency: IdempotencyKey {
                    key_id: preparation.request_digest().into(),
                    scope_ref: LOCAL_IDEMPOTENCY_SCOPE_REF.into(),
                },
                stream_mode: ModelStreamMode::Buffered,
            })
        }
    }

    /// The bridge across the synchronous port seam.
    ///
    /// `attempt` is called from inside the driver's async execution, so the two
    /// obvious bridges are wrong by construction: building a runtime here, or
    /// calling `Runtime::block_on`/`Handle::block_on`, panics with "Cannot start
    /// a runtime from within a runtime" — at run time, not compile time.
    /// `block_in_place` avoids that panic only on a multi-threaded runtime and
    /// panics on the current-thread flavor every `#[tokio::test]` uses.
    ///
    /// So this owns a runtime of its own and never blocks *on* it: the future is
    /// `spawn`ed onto that independent runtime, which is legal from any thread,
    /// and the calling thread waits on a plain std channel. The backend future
    /// therefore makes progress on threads the caller does not own, so no
    /// caller-runtime flavor can deadlock. The cost is honest and bounded: one
    /// caller thread is parked for the duration of one model attempt, which is
    /// exactly what a synchronous port seam means.
    struct BackendRuntime {
        /// `None` only while dropping. Held as an option so [`Drop`] can hand
        /// the runtime to `shutdown_background`: dropping a tokio runtime *value*
        /// inside an async context panics, and this port is dropped inside one.
        runtime: Option<tokio::runtime::Runtime>,
        handle: tokio::runtime::Handle,
    }

    /// The backend runtime stopped before the attempt reported an outcome.
    struct BridgeInterrupted;

    impl BackendRuntime {
        fn new() -> std::io::Result<Self> {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .thread_name("apxm-canonical-inference")
                .build()?;
            let handle = runtime.handle().clone();
            Ok(Self {
                runtime: Some(runtime),
                handle,
            })
        }

        fn run<F>(&self, future: F) -> Result<F::Output, BridgeInterrupted>
        where
            F: Future + Send + 'static,
            F::Output: Send + 'static,
        {
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            self.handle.spawn(async move {
                let _ = sender.send(future.await);
            });
            receiver.recv().map_err(|_| BridgeInterrupted)
        }
    }

    impl Drop for BackendRuntime {
        fn drop(&mut self) {
            if let Some(runtime) = self.runtime.take() {
                runtime.shutdown_background();
            }
        }
    }

    /// What the composition root found when it consulted the backend roster.
    ///
    /// This is retained so an unresolvable target reports *why* it is
    /// unresolvable instead of only that it is.
    struct RosterEvidence {
        source: String,
        bound_models: BTreeSet<String>,
        skipped: Vec<String>,
    }

    impl RosterEvidence {
        fn describe(&self) -> String {
            let bound = if self.bound_models.is_empty() {
                "binds no model reference".to_string()
            } else {
                format!(
                    "binds model references [{}]",
                    self.bound_models
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let mut described = format!("{} {bound}", self.source);
            for skipped in &self.skipped {
                described.push_str("; ");
                described.push_str(skipped);
            }
            described
        }
    }

    /// The canonical local model-inference port.
    pub struct LocalModelInferencePort {
        registry: Arc<LLMRegistry>,
        backend_runtime: BackendRuntime,
        roster: RosterEvidence,
        /// Adapter-observed diagnostics for attempts whose typed outcome cannot
        /// carry them. A post-send failure commits as `model_outcome_unknown`,
        /// whose contract has no message field; discarding what the adapter saw
        /// would leave an operator with an uncertain outcome and no evidence, so
        /// the text is reported alongside the outcome instead of inside it.
        attempt_diagnostics: Mutex<Vec<String>>,
    }

    impl LocalModelInferencePort {
        /// Build the local inference surface from the APXM backend roster at
        /// `$APXM_HOME/config.toml`.
        ///
        /// A backend that cannot be registered — an unset `env:` credential
        /// reference, a missing endpoint — does not abort construction. It is
        /// recorded, so a later unresolvable target names that exact reason
        /// rather than reporting a bare "unknown model".
        ///
        /// # Errors
        ///
        /// Returns an error only when the dedicated backend runtime cannot be
        /// created; an empty or unreadable roster is a typed per-attempt
        /// failure, not a construction failure.
        pub fn from_backend_roster() -> Result<Self> {
            let backend_runtime = BackendRuntime::new()?;
            let registry = Arc::new(LLMRegistry::new());
            let mut bound_models = BTreeSet::new();
            let mut skipped = Vec::new();

            let store = BackendStore::open()?;
            let source = store.path().display().to_string();
            match store.list() {
                Ok(configs) => {
                    for config in configs {
                        let name = config.name.clone();
                        let registration = match BackendRegistration::from_backend_config(&config) {
                            Ok(registration) => registration,
                            Err(error) => {
                                skipped.push(format!("backend '{name}' is unusable: {error}"));
                                continue;
                            }
                        };
                        let models = registration
                            .default_model
                            .iter()
                            .cloned()
                            .chain(registration.models.iter().map(|model| model.id.clone()))
                            .collect::<Vec<_>>();
                        let shared = registry.clone();
                        match backend_runtime
                            .run(async move { registration.register(&shared).await })
                        {
                            Ok(Ok(())) => bound_models.extend(models),
                            Ok(Err(error)) => {
                                skipped.push(format!("backend '{name}' did not register: {error}"));
                            }
                            Err(BridgeInterrupted) => skipped.push(format!(
                                "backend '{name}' did not register: the backend runtime stopped"
                            )),
                        }
                    }
                }
                Err(error) => skipped.push(format!("the roster is unreadable: {error}")),
            }

            Ok(Self {
                registry,
                backend_runtime,
                roster: RosterEvidence {
                    source,
                    bound_models,
                    skipped,
                },
                attempt_diagnostics: Mutex::new(Vec::new()),
            })
        }

        /// Adapter-observed diagnostics for every attempt that did not succeed,
        /// in attempt order.
        pub fn attempt_diagnostics(&self) -> Vec<String> {
            self.attempt_diagnostics
                .lock()
                .expect("model attempt diagnostics mutex poisoned")
                .clone()
        }

        fn record(&self, diagnostic: String) {
            self.attempt_diagnostics
                .lock()
                .expect("model attempt diagnostics mutex poisoned")
                .push(diagnostic);
        }

        /// Fail one attempt before anything is sent, recording the same text the
        /// typed error carries.
        fn before_send(&self, error: TypedError) -> AttemptDisposition {
            self.record(format!("{}: {}", error.code, error.message));
            AttemptDisposition::FailedBeforeSend(error)
        }

        /// Bind one exact model reference to one backend directly, for tests
        /// that drive the adapter without a machine-local roster.
        #[cfg(test)]
        pub(super) fn for_bound_backend(
            backend_name: &str,
            model: &str,
            backend: Arc<dyn LLMBackend>,
        ) -> Result<Self> {
            let backend_runtime = BackendRuntime::new()?;
            let registry = Arc::new(LLMRegistry::new());
            registry.register_arc(backend_name, backend)?;
            registry.bind_model(model, backend_name)?;
            Ok(Self {
                registry,
                backend_runtime,
                roster: RosterEvidence {
                    source: "the test-bound backend roster".into(),
                    bound_models: BTreeSet::from([model.to_string()]),
                    skipped: Vec::new(),
                },
                attempt_diagnostics: Mutex::new(Vec::new()),
            })
        }

        /// An inference surface with nothing registered, for tests that pin the
        /// unconfigured degradation without depending on `$APXM_HOME`.
        #[cfg(test)]
        pub(super) fn unconfigured(reason: &str) -> Result<Self> {
            Ok(Self {
                registry: Arc::new(LLMRegistry::new()),
                backend_runtime: BackendRuntime::new()?,
                roster: RosterEvidence {
                    source: "the test-bound backend roster".into(),
                    bound_models: BTreeSet::new(),
                    skipped: vec![reason.to_string()],
                },
                attempt_diagnostics: Mutex::new(Vec::new()),
            })
        }
    }

    impl ModelInferencePort for LocalModelInferencePort {
        fn attempt(&self, request: &ModelCallRequest, _attempt: u32) -> AttemptDisposition {
            let target = request.target().0.clone();
            let llm_request = match authored_llm_request(request.authored_request(), &target) {
                Ok(llm_request) => llm_request,
                Err(error) => return self.before_send(error),
            };
            // Resolution is exact: the authored target names one bound backend
            // or none. There is no default, alias, or first-available backend.
            let backend_name = match self.registry.resolve_backend(&llm_request) {
                Ok(backend_name) => backend_name,
                Err(error) => {
                    return self.before_send(TypedError {
                        category: ErrorCategory::Configuration,
                        code: "model_target_not_registered".into(),
                        message: format!(
                            "model target '{target}' is bound to no admitted inference backend \
                             ({error}). Canonical local execution resolves a target only through \
                             the APXM backend roster, and {}. Register the backend that serves \
                             this target with `apxm backend add`, and export the environment \
                             variable its `api_key = \"env:VAR\"` reference names.",
                            self.roster.describe()
                        ),
                    });
                }
            };

            let dispatch_target = backend_name.clone();
            let registry = self.registry.clone();
            let dispatched = self.backend_runtime.run(async move {
                registry
                    .generate_with_backend(&dispatch_target, llm_request)
                    .await
            });

            match dispatched {
                Ok(Ok(response)) => {
                    let disposition = model_attempt_disposition(&target, &backend_name, response);
                    if let AttemptDisposition::DeliveredTypedFailure(error) = &disposition {
                        self.record(format!("{}: {}", error.code, error.message));
                    }
                    disposition
                }
                // The single `generate` await is opaque: it covers the client
                // build, the connection, the send, and the response. The adapter
                // cannot observe which of those failed, so it never claims the
                // request was not sent.
                Ok(Err(error)) => {
                    let message = format!(
                        "model target '{target}' failed on backend '{backend_name}' after the \
                         request was handed to it: {error:#}. Whether the provider observed the \
                         request is unobserved, so it is never resent."
                    );
                    self.record(format!("model_attempt_failed: {message}"));
                    AttemptDisposition::FailedAfterSend(TypedError {
                        category: ErrorCategory::Unavailable,
                        code: "model_attempt_failed".into(),
                        message,
                    })
                }
                Err(BridgeInterrupted) => {
                    let message = format!(
                        "model target '{target}' was dispatched to backend '{backend_name}' and \
                         the backend runtime stopped before reporting an outcome"
                    );
                    self.record(format!("model_attempt_abandoned: {message}"));
                    AttemptDisposition::FailedAfterSend(TypedError {
                        category: ErrorCategory::OutcomeUnknown,
                        code: "model_attempt_abandoned".into(),
                        message,
                    })
                }
            }
        }
    }

    /// The closed authored request shape canonical local execution admits.
    ///
    /// `deny_unknown_fields` is the point: an authored field this adapter does
    /// not carry is a request the author wrote and the provider would never see.
    /// Rejecting it names the gap; dropping it would hide one.
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AuthoredModelRequest {
        #[serde(default)]
        prompt: Option<String>,
        #[serde(default)]
        system_prompt: Option<String>,
        #[serde(default)]
        messages: Vec<AuthoredMessage>,
        #[serde(default)]
        temperature: Option<f64>,
        #[serde(default)]
        max_tokens: Option<usize>,
        #[serde(default)]
        top_p: Option<f64>,
        #[serde(default)]
        stop_sequences: Vec<String>,
    }

    /// One authored conversation turn.
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AuthoredMessage {
        role: Role,
        content: String,
    }

    /// Map the authored SSA request value onto the exact backend request.
    fn authored_llm_request(authored: &Value, target: &str) -> Result<LLMRequest, TypedError> {
        let authored: AuthoredModelRequest =
            serde_json::from_value(authored.clone()).map_err(|error| TypedError {
                category: ErrorCategory::Validation,
                code: "model_request_not_admitted".into(),
                message: format!(
                    "the authored request for model target '{target}' is not an admitted model \
                     request: {error}. The admitted shape is a JSON object with any of prompt, \
                     system_prompt, messages, temperature, max_tokens, top_p, and stop_sequences; \
                     an unknown field is rejected rather than dropped, because dropping it would \
                     send a request the author did not write."
                ),
            })?;

        let mut llm_request = LLMRequest::new(authored.prompt.unwrap_or_default());
        llm_request.messages = authored
            .messages
            .into_iter()
            .map(|message| Message::text(message.role, message.content))
            .collect();
        llm_request.system_prompt = authored.system_prompt;
        if let Some(temperature) = authored.temperature {
            llm_request.temperature = temperature;
        }
        llm_request.max_tokens = authored.max_tokens;
        llm_request.top_p = authored.top_p;
        llm_request.stop_sequences = authored.stop_sequences;
        llm_request.model = Some(target.to_string());

        // Run the backend's own pre-dispatch contract here, where nothing has
        // been sent yet, so a malformed authored request is a definite failure
        // rather than an uncertain one raised inside the provider call.
        llm_request
            .validate_provider_dispatch()
            .map_err(|error| TypedError {
                category: ErrorCategory::Validation,
                code: "model_request_not_dispatchable".into(),
                message: format!(
                    "the authored request for model target '{target}' cannot be dispatched: \
                     {error:#}"
                ),
            })?;
        Ok(llm_request)
    }

    /// Project one delivered response onto the port's attempt disposition.
    ///
    /// A response envelope came back, so the request was definitely sent and its
    /// terminal state is known. A finish reason that reports the model did not
    /// produce an answer is therefore a `DeliveredTypedFailure` — never an
    /// uncertain outcome, and never a success carrying empty content.
    fn model_attempt_disposition(
        target: &str,
        backend_name: &str,
        response: LLMResponse,
    ) -> AttemptDisposition {
        let (category, code) = match response.finish_reason {
            FinishReason::Error => (ErrorCategory::Internal, "model_reported_generation_error"),
            FinishReason::ContentFilter => {
                (ErrorCategory::Validation, "model_reported_content_filter")
            }
            FinishReason::Timeout => (ErrorCategory::Unavailable, "model_reported_timeout"),
            FinishReason::Stop
            | FinishReason::Length
            | FinishReason::ToolUse
            | FinishReason::Unknown => {
                return AttemptDisposition::Success {
                    usage: Usage {
                        input_tokens: response.usage.input_tokens as u64,
                        output_tokens: response.usage.output_tokens as u64,
                    },
                    output: json!({
                        "content": response.content,
                        "model": response.model,
                        "finish_reason": response.finish_reason.to_string(),
                        "tool_calls": response.tool_calls,
                    }),
                };
            }
        };
        AttemptDisposition::DeliveredTypedFailure(TypedError {
            category,
            code: code.into(),
            message: format!(
                "model target '{target}' on backend '{backend_name}' returned a response that \
                 finished as '{}' and so carries no admitted model output",
                response.finish_reason
            ),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use apxm_backends::llm::backends::mock::{MockLLMBackend, MockResponse};
        use apxm_inference::{
            InferenceTargetCommitment, ModelBindingAdmission, ModelTargetRef, ResolvedModelBinding,
        };

        const FIXTURE_MODEL: &str = "test.model.echo";

        fn digest(c: char) -> String {
            format!("sha256:{}", c.to_string().repeat(64))
        }

        fn model_request(target: &str, authored: Value) -> ModelCallRequest {
            let authored_target = ModelTargetRef(target.into());
            let admission = ModelBindingAdmission::for_invocation(vec![
                ResolvedModelBinding::from_target_commitment(
                    InferenceTargetCommitment::commit(
                        target,
                        digest('b'),
                        "test.model.deployment",
                        digest('a'),
                        digest('c'),
                        digest('d'),
                        1,
                    )
                    .expect("test target commitment"),
                ),
            ]);
            let preparation = ModelCallPreparation::authorize(
                "test.effect",
                "test.node-execution",
                digest('e'),
                digest('f'),
                authored,
                &authored_target,
                &admission,
            )
            .expect("test model preparation");
            let metadata = LocalModelRequestMetadata
                .materialize(&preparation)
                .expect("local metadata is always available");
            ModelCallRequest::prepare(preparation, metadata).expect("test model request")
        }

        fn echo_backend() -> Arc<dyn LLMBackend> {
            Arc::new(
                MockLLMBackend::new()
                    .named("test-echo")
                    .model_name(FIXTURE_MODEL)
                    .default(MockResponse::new("the model answered")),
            )
        }

        #[test]
        fn an_authored_request_reaches_a_real_backend_from_the_synchronous_seam() {
            let port = LocalModelInferencePort::for_bound_backend(
                "test-echo",
                FIXTURE_MODEL,
                echo_backend(),
            )
            .expect("bound inference port");

            let disposition = port.attempt(
                &model_request(FIXTURE_MODEL, json!({"prompt": "say something"})),
                0,
            );

            let AttemptDisposition::Success { usage, output } = disposition else {
                panic!("an admitted target must commit: {disposition:?}");
            };
            assert_eq!(output["content"], "the model answered");
            assert_eq!(output["finish_reason"], "stop");
            assert_eq!(usage.input_tokens, 10);
            assert_eq!(usage.output_tokens, 20);
        }

        /// The bridge must work from inside an async context, which is where the
        /// driver actually calls it. A `block_on` bridge panics here.
        #[tokio::test]
        async fn the_bridge_runs_inside_an_async_caller_without_panicking() {
            let port = LocalModelInferencePort::for_bound_backend(
                "test-echo",
                FIXTURE_MODEL,
                echo_backend(),
            )
            .expect("bound inference port");
            let disposition = port.attempt(
                &model_request(FIXTURE_MODEL, json!({"prompt": "inside a runtime"})),
                0,
            );
            assert!(matches!(disposition, AttemptDisposition::Success { .. }));
        }

        #[test]
        fn an_unconfigured_target_fails_before_send_with_a_configuration_error() {
            let port = LocalModelInferencePort::unconfigured(
                "backend 'gateway' is unusable: Environment variable 'LLM_GATEWAY_KEY' not set",
            )
            .expect("unconfigured inference port");

            let disposition =
                port.attempt(&model_request(FIXTURE_MODEL, json!({"prompt": "hello"})), 0);

            let AttemptDisposition::FailedBeforeSend(error) = disposition else {
                panic!("an unbound target is never sent: {disposition:?}");
            };
            assert_eq!(error.category, ErrorCategory::Configuration);
            assert_eq!(error.code, "model_target_not_registered");
            assert!(
                error.message.contains(FIXTURE_MODEL)
                    && error.message.contains("LLM_GATEWAY_KEY")
                    && error.message.contains("apxm backend add"),
                "the unconfigured error names the target, the reason, and the fix: {}",
                error.message
            );
        }

        #[test]
        fn an_authored_field_the_adapter_cannot_carry_is_rejected_not_dropped() {
            let port = LocalModelInferencePort::for_bound_backend(
                "test-echo",
                FIXTURE_MODEL,
                echo_backend(),
            )
            .expect("bound inference port");

            let disposition = port.attempt(
                &model_request(
                    FIXTURE_MODEL,
                    json!({"prompt": "hello", "logit_bias": {"5": 1}}),
                ),
                0,
            );

            let AttemptDisposition::FailedBeforeSend(error) = disposition else {
                panic!("an unmapped authored field is never sent: {disposition:?}");
            };
            assert_eq!(error.category, ErrorCategory::Validation);
            assert_eq!(error.code, "model_request_not_admitted");
            assert!(error.message.contains("logit_bias"), "{}", error.message);
        }

        #[test]
        fn a_backend_failure_is_uncertain_and_is_reported_as_a_diagnostic() {
            let port = LocalModelInferencePort::for_bound_backend(
                "test-echo",
                FIXTURE_MODEL,
                Arc::new(
                    MockLLMBackend::new()
                        .model_name(FIXTURE_MODEL)
                        .always_fail("connection reset by peer"),
                ),
            )
            .expect("bound inference port");

            let disposition =
                port.attempt(&model_request(FIXTURE_MODEL, json!({"prompt": "hello"})), 0);

            let AttemptDisposition::FailedAfterSend(error) = disposition else {
                panic!(
                    "an opaque provider failure never claims the request was not sent: \
                     {disposition:?}"
                );
            };
            assert_eq!(error.code, "model_attempt_failed");
            assert!(
                port.attempt_diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.contains("connection reset by peer")),
                "the text an uncertain outcome cannot carry is retained: {:?}",
                port.attempt_diagnostics()
            );
        }

        #[test]
        fn a_content_filtered_response_is_a_delivered_typed_failure() {
            let filtered = LLMResponse::new(
                String::new(),
                FIXTURE_MODEL,
                apxm_core::types::TokenUsage::new(4, 0),
                FinishReason::ContentFilter,
            );
            let disposition = model_attempt_disposition(FIXTURE_MODEL, "test-echo", filtered);
            let AttemptDisposition::DeliveredTypedFailure(error) = disposition else {
                panic!("a delivered refusal is terminal and known: {disposition:?}");
            };
            assert_eq!(error.code, "model_reported_content_filter");
        }

        #[test]
        fn local_request_metadata_carries_the_runtime_owned_identities() {
            let request = model_request(FIXTURE_MODEL, json!({"prompt": "hello"}));
            assert_eq!(
                request.model_context_envelope_ref().sealed_digest,
                request.context_digest(),
                "the sealed context digest is the digest of the Context this call used"
            );
            assert_eq!(
                request.idempotency().key_id,
                request.request_digest(),
                "the idempotency key is the stable request identity retries reuse"
            );
            assert_eq!(request.stream_mode(), ModelStreamMode::Buffered);
        }
    }
}

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use apxm_execution::{
    CapabilityInvocationAdmission, CapabilityOutcome, CompositionOutcome, CompositionPort,
    CompositionReceiver, CompositionRequest, EventAwait, EventOutcome, EventPort, ExecutionRequest,
    NodeOutcome, NoopStaticHookHandler, RuntimeProfile,
};
#[cfg(test)]
use apxm_execution::{ExecutionPortBundle, ExecutionPorts, RuntimeProfileError, execute};
use apxm_inference::{
    InferenceTargetCommitment, ModelBindingAdmission, ModelCallRequestMetadataPort, ModelOutcome,
    ResolvedModelBinding,
};
use apxm_kernel::{
    AcpPromptOutcome, AcpPromptRequest, AdmittedConfinement, AdmittedPortBinding, AtomicWriteSet,
    ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult,
    ExternalAgentCapabilityPort, InvocationAdmission, PortImplementation, PortSlot,
    ProgramInstanceRef, ProgramInvocationRef, PromptEffectState, ResourceCeilings,
    RuntimeAdmission, VerifiedInvocationAdmission, digest_serializable,
    verify_invocation_admission,
};
use apxm_kernel::{ConfinementAttestation, ConfinementError, ConfinementPort, ConfinementRequest};
#[cfg(test)]
use apxm_kernel::{ExactPortBinding, PortBundle, PortBundleSpec};
use apxm_program::CapabilityInvocationAuthority;
use apxm_program::air::{AirModule, SemanticOpKind};
#[cfg(test)]
use apxm_program::artifact::SchemaDigestRef;
use apxm_program::external_agent::{AttributedEvent, AttributedEventKind, PeerUsage};

use capability_port::LocalCapabilityPort;
use model_port::{LocalModelInferencePort, LocalModelRequestMetadata};

pub async fn execute_canonical_command(
    input: PathBuf,
    invocation_admission: PathBuf,
    release: PathBuf,
    provenance: PathBuf,
    json_output: bool,
) -> Result<()> {
    let (air, artifact_bytes) = load_canonical_air(&input)?;
    let admission_bytes = read_exact_bytes(&invocation_admission, "Invocation Admission")?;
    let admission: InvocationAdmission =
        serde_json::from_slice(&admission_bytes).with_context(|| {
            format!(
                "{} must contain exact {} JSON",
                invocation_admission.display(),
                apxm_kernel::INVOCATION_ADMISSION_SCHEMA
            )
        })?;
    let release_bytes = read_exact_bytes(&release, "release")?;
    let provenance_bytes = read_exact_bytes(&provenance, "provenance")?;
    let output = CanonicalRuntime::new()
        .execute(
            air,
            &artifact_bytes,
            &admission,
            &release_bytes,
            &provenance_bytes,
        )
        .await?;
    if json_output {
        println!("{}", serde_json::to_string(&output)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

/// One APXM-owned canonical runtime instance. The commit port is retained for
/// the instance lifetime so replay produces the existing idempotent/compare
/// conflict behavior instead of creating a second commit writer.
pub struct CanonicalRuntime {
    commit: Arc<DevCommit>,
}

impl CanonicalRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self {
            commit: Arc::new(DevCommit::default()),
        }
    }

    /// Execute one canonical AIR against the machine-local backend roster.
    pub async fn execute(
        &self,
        air: AirModule,
        artifact_bytes: &[u8],
        admission: &InvocationAdmission,
        release_bytes: &[u8],
        provenance_bytes: &[u8],
    ) -> Result<Value> {
        let model = Arc::new(LocalModelInferencePort::from_backend_roster()?);
        self.execute_with_model(
            air,
            artifact_bytes,
            admission,
            release_bytes,
            provenance_bytes,
            model,
        )
        .await
    }

    /// The one execution body, parameterized by the admitted inference port so a
    /// test can bind an exact backend instead of whatever the machine happens to
    /// have registered. Everything else — admission, ports, commit, reporting —
    /// is the shipped path.
    async fn execute_with_model(
        &self,
        air: AirModule,
        artifact_bytes: &[u8],
        admission: &InvocationAdmission,
        release_bytes: &[u8],
        provenance_bytes: &[u8],
        model: Arc<LocalModelInferencePort>,
    ) -> Result<Value> {
        let capability_invocations = local_capability_invocation_admissions(&air)?;
        let descriptor = canonical_runtime_descriptor();
        // Report both sides: this fails closed on any reference-profile change,
        // and without the expected digests the only way to re-mint a fixture is
        // to reimplement the derivation by hand.
        if admission.port_bindings_digest != canonical_port_bindings_digest()
            || admission.resource_ceiling_digest != canonical_resource_ceiling_digest()
        {
            anyhow::bail!(
                "Invocation Admission does not bind the exact reference runtime profile\n  \
                 port_bindings_digest:    admitted {} != expected {}\n  \
                 resource_ceiling_digest: admitted {} != expected {}",
                admission.port_bindings_digest,
                canonical_port_bindings_digest(),
                admission.resource_ceiling_digest,
                canonical_resource_ceiling_digest(),
            );
        }
        let verified = verify_invocation_admission(
            admission,
            artifact_bytes,
            release_bytes,
            provenance_bytes,
            &descriptor.port_bindings,
            descriptor.resource_ceilings.clone(),
            &descriptor.confinement,
        )
        .map_err(|error| anyhow::anyhow!(error))?;
        let model_binding_digest = descriptor
            .port_bindings
            .iter()
            .find(|binding| binding.slot == PortSlot::ModelInference.as_str())
            .map(|binding| binding.binding_digest.clone())
            .ok_or_else(|| anyhow::anyhow!("reference runtime model binding is absent"))?;
        let request = ExecutionRequest {
            model_admission: model_admission(&air, &model_binding_digest),
            initial_values: initial_model_request_values(&air),
            air,
            hook_bindings: Vec::new(),
            capability_invocations,
            program_instance_ref: ProgramInstanceRef::new("canonical.instance"),
            program_invocation_ref: ProgramInvocationRef::new(admission.invocation_id.clone()),
            commit_id: format!("canonical.commit.{}", admission.invocation_id),
            write_set: reference_write_set(&admission.invocation_id),
        };
        let profile = runtime_profile_from_invocation(
            self.commit.clone(),
            Arc::new(LocalCapabilityPort::new().map_err(|error| anyhow::anyhow!(error))?),
            model.clone(),
            Arc::new(LocalModelRequestMetadata),
            verified,
            "canonical.execution",
        )
        .await?;
        let report = profile
            .execute(request, Value::Null)
            .await
            .map_err(|err| anyhow::anyhow!(err))?;

        Ok(json!({
            "schema_version": "apxm.local-execute-result",
            "runtime": "apxm_execution",
            "status": "completed",
            "content": report.final_context,
            "results": {
                "node_outcomes": report.node_outcomes.iter().map(node_outcome_json).collect::<Vec<_>>(),
                "external_agent_evidence": report.external_agent_evidence,
                // A post-send model failure commits as `model_outcome_unknown`,
                // whose typed shape carries no message. The adapter reports what
                // it observed here instead, beside the outcome rather than
                // inside it, so an uncertain run is still diagnosable.
                "model_attempt_diagnostics": model.attempt_diagnostics(),
            },
            "stats": {
                "executed_nodes": report.node_outcomes.len(),
                "failed_nodes": failed_node_count(&report.node_outcomes),
                "duration_ms": 0,
            },
            "llm_usage": {
                "input_tokens": report.native_usage.input_tokens,
                "output_tokens": report.native_usage.output_tokens,
                "total_requests": report
                    .node_outcomes
                    .iter()
                    .filter(|outcome| matches!(outcome, NodeOutcome::Model { .. }))
                    .count(),
            },
            "commit": commit_result_json(&report.commit),
        }))
    }
}

impl Default for CanonicalRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn read_exact_bytes(path: &PathBuf, label: &str) -> Result<Vec<u8>> {
    std::fs::read(path)
        .with_context(|| format!("failed to read exact {label} bytes from {}", path.display()))
}

fn load_canonical_air(input: &PathBuf) -> Result<(AirModule, Vec<u8>)> {
    let bytes = read_exact_bytes(input, "canonical AIR")?;
    let text = std::str::from_utf8(&bytes)
        .with_context(|| format!("{} must contain UTF-8 canonical AIR JSON", input.display()))?;
    let air: AirModule = serde_json::from_str(text)
        .with_context(|| format!("{} must contain canonical apxm.air JSON", input.display()))?;
    let verdict = air.verify();
    if !verdict.is_accepted() {
        let diagnostics = verdict
            .into_diagnostics()
            .into_iter()
            .map(|diagnostic| {
                json!({
                    "code": diagnostic.code.slug(),
                    "location": diagnostic.location,
                    "message": diagnostic.message,
                })
            })
            .collect::<Vec<_>>();
        anyhow::bail!(
            "canonical AIR verification failed: {}",
            serde_json::to_string(&diagnostics)?
        );
    }
    Ok((air, bytes))
}

fn model_targets(air: &AirModule) -> Vec<String> {
    let mut targets = Vec::new();
    for op in &air.semantic_operations {
        if op.op != SemanticOpKind::ModelCall {
            continue;
        }
        let Some(target) = op
            .operands
            .iter()
            .find(|operand| operand.slot == "model_ref")
            .map(|operand| operand.value_id.clone())
        else {
            continue;
        };
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    targets
}

fn initial_model_request_values(air: &AirModule) -> BTreeMap<String, Value> {
    air.semantic_operations
        .iter()
        .filter(|operation| operation.op == apxm_program::SemanticOpKind::ModelCall)
        .filter_map(|operation| {
            let value_id = operation
                .operands
                .iter()
                .find(|operand| operand.slot == "request")?
                .value_id
                .clone();
            (!air
                .value_assemblies
                .iter()
                .any(|assembly| assembly.value_id == value_id))
            .then(|| (value_id.clone(), serde_json::json!({"value_id": value_id})))
        })
        .collect()
}

/// Acting principal for a Capability effect admitted by the local root.
const LOCAL_ACTING_PRINCIPAL_REF: &str = "apxm.canonical.local.acting-principal";
/// Agent identity a locally admitted Capability effect is attributed to.
const LOCAL_AGENT_IDENTITY_REF: &str = "apxm.canonical.local.agent-identity";
/// Prefix of the grant reference minted per authored capability reference.
const LOCAL_CAPABILITY_GRANT_PREFIX: &str = "apxm.canonical.local.grant.";

/// Mint one Capability invocation admission per authored `capability.invoke`
/// node.
///
/// `apxm execute-canonical` runs with no Auth or Server issuing Capability
/// grants, so the canonical composition root *is* the authority — and says so.
/// Every reference names the local root explicitly, so the evidence a run emits
/// can never be mistaken for a server-issued grant. Each admission is keyed by
/// AIR node id and carries the capability reference the node authored, which is
/// exactly what the driver re-checks before preparing the request; a mismatch
/// there is still a hard `CapabilityInvocationAdmissionMismatch`.
///
/// A node with no `capability_ref` operand is left unadmitted on purpose: the
/// driver raises the precise `MissingOperand` diagnostic for it, which is a
/// better failure than a fabricated admission for an unnamed capability.
fn local_capability_invocation_admissions(
    air: &AirModule,
) -> Result<BTreeMap<String, CapabilityInvocationAdmission>> {
    let mut admissions = BTreeMap::new();
    for operation in &air.semantic_operations {
        if operation.op != SemanticOpKind::CapabilityInvoke {
            continue;
        }
        let Some(capability_ref) = operation
            .operands
            .iter()
            .find(|operand| operand.slot == "capability_ref")
            .map(|operand| operand.value_id.clone())
        else {
            continue;
        };
        let authority = CapabilityInvocationAuthority::new(
            LOCAL_ACTING_PRINCIPAL_REF,
            LOCAL_AGENT_IDENTITY_REF,
            format!("{LOCAL_CAPABILITY_GRANT_PREFIX}{capability_ref}"),
            Vec::new(),
        )
        .map_err(|error| {
            anyhow::anyhow!(
                "canonical local execution cannot admit Capability {capability_ref} at node {}: {error}",
                operation.node_id
            )
        })?;
        admissions.insert(
            operation.node_id.clone(),
            CapabilityInvocationAdmission {
                capability_ref,
                authority,
                permission: None,
            },
        );
    }
    Ok(admissions)
}

fn model_admission(air: &AirModule, model_binding_digest: &str) -> ModelBindingAdmission {
    ModelBindingAdmission::for_invocation(
        model_targets(air)
            .into_iter()
            .map(|target| {
                ResolvedModelBinding::from_target_commitment(
                    InferenceTargetCommitment::commit(
                        target,
                        digest('b'),
                        "canonical.model",
                        model_binding_digest,
                        digest('c'),
                        digest('d'),
                        1,
                    )
                    .expect("development target commitment"),
                )
            })
            .collect(),
    )
}

#[cfg(test)]
fn dev_write_set() -> AtomicWriteSet {
    AtomicWriteSet {
        next_program_state_digest: digest('1'),
        continuation_digest: digest('2'),
        checkpoint_effect_outcomes_digest: digest('3'),
        runtime_evidence_batch_digest: digest('4'),
        usage_facts_digest: digest('5'),
        session_output_refs_digest: digest('6'),
    }
}

fn reference_write_set(invocation_id: &str) -> AtomicWriteSet {
    let digest_for = |member: &str| digest_text(&format!("{invocation_id}\0{member}"));
    AtomicWriteSet {
        next_program_state_digest: digest_for("state_continuation"),
        continuation_digest: digest_for("continuation"),
        checkpoint_effect_outcomes_digest: digest_for("checkpoint_effect_outcomes"),
        runtime_evidence_batch_digest: digest_for("runtime_evidence"),
        usage_facts_digest: digest_for("usage_facts"),
        session_output_refs_digest: digest_for("session_output_refs"),
    }
}

/// Exact APXM-owned descriptors used by the canonical composition root.
/// The transport digest must hash these descriptors; it is never copied onto
/// an unrelated implementation.
#[derive(Clone, Debug)]
pub struct CanonicalRuntimeDescriptor {
    pub port_bindings: Vec<AdmittedPortBinding>,
    pub resource_ceilings: ResourceCeilings,
    pub confinement: AdmittedConfinement,
}

pub fn canonical_runtime_descriptor() -> CanonicalRuntimeDescriptor {
    let port_bindings = [
        (
            PortSlot::ExecutionCommit,
            "apxm.execution-commit",
            "b64becdf1c246b6a05bf02206dcf2306171501079257f188a7d76d88af9f26f7",
        ),
        (
            PortSlot::Confinement,
            "apxm.confinement",
            "5d5e5c8e9e0a6d6e5f87eaed8e4c5ee3dfeef7fbd2501dc4f180785b9f5d7e0f",
        ),
        (
            PortSlot::ModelInference,
            "apxm.model-inference",
            "fdf6aea657550b87f8b66f63f8e50cf793cec427b320c5bce940ba24fbc97362",
        ),
        (
            PortSlot::Capability,
            "apxm.capability-invocation",
            "9369bd4c3506ba145d9424425efcb0493b288d418c582701d0856e8216fc8383",
        ),
        (
            PortSlot::ExternalAgentCapability,
            "apxm.external-agent",
            "d7f7a1319eeac3c69af8355dd58b60a8f86d4861e5d32e3b8cac85105bd5eb5d",
        ),
        (
            PortSlot::DurableEvent,
            "apxm.durable-event",
            "75d3c93c0c34f3c3d4dfeb89cec81cdbc208ac9106bba6955a923b0c566342d9",
        ),
        (
            PortSlot::ProgramComposition,
            "apxm.program-composition",
            "7cacd4d1d1a0f17f30e167907997fbaa86e3e84026c69eee11d0673ceb67351a",
        ),
    ]
    .into_iter()
    .map(|(slot, schema_id, contract_digest)| AdmittedPortBinding {
        slot: slot.as_str().into(),
        port_contract_schema_id: schema_id.into(),
        port_contract_digest: format!("sha256:{contract_digest}"),
        binding_digest: digest_text(&format!("apxm.canonical.binding.{}", slot.as_str())),
        proof_digest: digest_text(&format!("apxm.canonical.proof.{}", slot.as_str())),
    })
    .collect();
    CanonicalRuntimeDescriptor {
        port_bindings,
        resource_ceilings: ResourceCeilings {
            max_wall_ms: 60_000,
            max_memory_bytes: 64 * 1024 * 1024,
            max_effect_bytes: 1024 * 1024,
        },
        confinement: AdmittedConfinement {
            confinement_type: "NATIVE-SANDBOX".into(),
            sandbox_digest: digest_text("apxm.canonical.sandbox.v1"),
            policy_digest: digest_text("apxm.canonical.policy.v1"),
        },
    }
}

pub fn canonical_port_bindings_digest() -> String {
    digest_serializable(&canonical_runtime_descriptor().port_bindings)
        .expect("canonical bindings are serializable")
}

pub fn canonical_resource_ceiling_digest() -> String {
    digest_serializable(&canonical_runtime_descriptor().resource_ceilings)
        .expect("canonical ceilings are serializable")
}

fn digest_text(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

struct DevConfinement;

#[async_trait]
impl ConfinementPort for DevConfinement {
    async fn attest(
        &self,
        request: ConfinementRequest,
    ) -> Result<ConfinementAttestation, ConfinementError> {
        Ok(ConfinementAttestation {
            attestation_id: format!("dev-attestation.{}", request.execution_id),
            host_id: request.host_id,
            execution_id: request.execution_id,
            confinement_type: request.confinement_type,
            sandbox_digest: request.sandbox_digest,
            policy_digest: request.policy_digest,
            attested_at: "dev-profile".into(),
            signature: "dev-profile-attestation".into(),
        })
    }
}

struct DevExternalAgent;
#[async_trait]
impl ExternalAgentCapabilityPort for DevExternalAgent {
    async fn prompt(&self, request: AcpPromptRequest) -> AcpPromptOutcome {
        AcpPromptOutcome {
            session_ref: request.session_ref,
            state: PromptEffectState::Completed {
                stop_reason: Some("dev_profile_completed".into()),
            },
            nested_events: vec![AttributedEvent {
                event_sequence: 0,
                kind: AttributedEventKind::Message,
                detail: Some("dev-profile external agent completed".into()),
                reverse_operation: None,
                reverse_target: None,
                reverse_decision: None,
            }],
            peer_usage: vec![PeerUsage {
                reported_by: "dev-profile".into(),
                metric_scope: "peer.usage.unavailable".into(),
                reported_value: "unavailable_not_observed".into(),
                availability_partial: None,
            }],
        }
    }
}

struct DevEvents;
#[async_trait]
impl EventPort for DevEvents {
    async fn await_event(&self, request: EventAwait) -> EventOutcome {
        EventOutcome::Fulfilled {
            payload: format!("event:{}", request.event_ref),
            event_ref: request.event_ref,
        }
    }
}

struct DevComposition;
#[async_trait]
impl CompositionPort for DevComposition {
    async fn program_new(&self, request: CompositionRequest) -> CompositionOutcome {
        CompositionOutcome::Created {
            child_instance_ref: format!("dev.child.{}", request.receiver.reference()),
        }
    }

    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome {
        // A stateful instance receiver resolves to the already-created child;
        // a program receiver is a one-shot invocation.
        let child = match &request.receiver {
            CompositionReceiver::Instance {
                program_instance_ref,
            } => program_instance_ref.clone(),
            CompositionReceiver::Program { program_ref } => format!("dev.child.{program_ref}"),
        };
        CompositionOutcome::Invoked {
            child_instance_ref: child,
        }
    }
}

#[derive(Default)]
struct DevCommit {
    version: Mutex<u64>,
}

#[async_trait]
impl ExecutionCommitPort for DevCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        let mut version = self.version.lock().expect("dev commit mutex poisoned");
        if *version != request.expected_program_state_version {
            return ExecutionCommitResult::CompareConflict {
                current_program_state_version: *version,
            };
        }
        *version += 1;
        ExecutionCommitResult::Committed {
            new_program_state_version: *version,
            evidence_position_ref: "dev.evidence.1".into(),
        }
    }

    async fn current_version(&self, _program_instance_ref: &ProgramInstanceRef) -> u64 {
        *self.version.lock().expect("dev commit mutex poisoned")
    }

    /// The in-process development commit holds no durable record, so it never
    /// has a committed continuation to hand back. Resumption requires a real
    /// admitted commit binding.
    async fn load_continuation(
        &self,
        _program_instance_ref: &ProgramInstanceRef,
    ) -> Option<serde_json::Value> {
        None
    }
}

#[cfg(test)]
const DEV_BINDING_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[cfg(test)]
fn dev_ports(
    commit: Arc<DevCommit>,
    capability: Arc<LocalCapabilityPort>,
    model: Arc<LocalModelInferencePort>,
    model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
) -> Result<ExecutionPorts> {
    let contract = |schema_id: &str| SchemaDigestRef {
        schema_id: schema_id.into(),
        digest: DEV_BINDING_DIGEST.into(),
    };
    let binding = |slot, schema_id| ExactPortBinding {
        slot,
        port_contract: contract(schema_id),
        binding_digest: DEV_BINDING_DIGEST.into(),
        proof_digest: DEV_BINDING_DIGEST.into(),
    };
    let spec = PortBundleSpec::new(vec![
        (PortSlot::ExecutionCommit, contract("apxm.execution-commit")),
        (PortSlot::ModelInference, contract("apxm.model-inference")),
        (PortSlot::Capability, contract("apxm.capability-invocation")),
        (
            PortSlot::ExternalAgentCapability,
            contract("apxm.external-agent"),
        ),
    ]);
    let kernel_bundle = PortBundle::construct(
        &spec,
        vec![
            (
                binding(PortSlot::ExecutionCommit, "apxm.execution-commit"),
                PortImplementation::ExecutionCommit(commit),
            ),
            (
                binding(PortSlot::ModelInference, "apxm.model-inference"),
                PortImplementation::ModelInference(model),
            ),
            (
                binding(PortSlot::Capability, "apxm.capability-invocation"),
                PortImplementation::Capability(capability),
            ),
            (
                binding(PortSlot::ExternalAgentCapability, "apxm.external-agent"),
                PortImplementation::ExternalAgentCapability(Arc::new(DevExternalAgent)),
            ),
        ],
    )?;
    let bundle = ExecutionPortBundle::construct(
        Arc::new(kernel_bundle),
        contract("apxm.durable-event"),
        binding(PortSlot::DurableEvent, "apxm.durable-event"),
        Arc::new(DevEvents),
        contract("apxm.program-composition"),
        binding(PortSlot::ProgramComposition, "apxm.program-composition"),
        Arc::new(DevComposition),
    )?;
    Ok(ExecutionPorts::from_admitted_bundle(
        &bundle,
        model_call_request_metadata,
        Arc::new(NoopStaticHookHandler),
    )?)
}

async fn runtime_profile_from_invocation(
    commit: Arc<DevCommit>,
    capability: Arc<LocalCapabilityPort>,
    model: Arc<LocalModelInferencePort>,
    model_call_request_metadata: Arc<dyn ModelCallRequestMetadataPort>,
    verified: VerifiedInvocationAdmission,
    execution_id: &str,
) -> Result<RuntimeProfile> {
    let entries = verified
        .port_bindings
        .iter()
        .map(|binding| {
            let implementation = match binding.slot {
                PortSlot::ExecutionCommit => PortImplementation::ExecutionCommit(commit.clone()),
                PortSlot::Confinement => PortImplementation::Confinement(Arc::new(DevConfinement)),
                PortSlot::ModelInference => PortImplementation::ModelInference(model.clone()),
                PortSlot::Capability => PortImplementation::Capability(capability.clone()),
                PortSlot::ExternalAgentCapability => {
                    PortImplementation::ExternalAgentCapability(Arc::new(DevExternalAgent))
                }
                PortSlot::DurableEvent => PortImplementation::DurableEvent(Arc::new(DevEvents)),
                PortSlot::ProgramComposition => {
                    PortImplementation::ProgramComposition(Arc::new(DevComposition))
                }
            };
            (binding.clone(), implementation)
        })
        .collect();
    let runtime_admission =
        RuntimeAdmission::admit_invocation(verified, entries, "apxm-canonical", execution_id)
            .await
            .map_err(|error| anyhow::anyhow!(error))?;
    RuntimeProfile::from_fully_admitted(
        runtime_admission,
        model_call_request_metadata,
        Arc::new(NoopStaticHookHandler),
    )
    .map_err(|error| anyhow::anyhow!(error))
}

/// Count node outcomes that reported a definite failure.
///
/// Before the Capability port was wired, no node in a canonical local run could
/// fail and this count was a hardcoded zero; an authored `capability.invoke`
/// that a policy denies makes that literal wrong. `OutcomeUnknown` is
/// deliberately excluded: an unobserved outcome is not a known failure, and
/// counting it as one would erase exactly the uncertainty the ports preserve.
/// Cancellation and a parked event are likewise not failures.
fn failed_node_count(outcomes: &[NodeOutcome]) -> usize {
    outcomes
        .iter()
        .filter(|outcome| match outcome {
            NodeOutcome::Model { outcome, .. } => {
                matches!(outcome, ModelOutcome::TypedFailure { .. })
            }
            NodeOutcome::Capability { outcome, .. } => {
                matches!(outcome, CapabilityOutcome::Failed { .. })
            }
            NodeOutcome::ProgramNew { outcome, .. }
            | NodeOutcome::ProgramInvoke { outcome, .. } => {
                matches!(outcome, CompositionOutcome::Failed { .. })
            }
            NodeOutcome::AwaitEvent { outcome, .. } => matches!(
                outcome,
                EventOutcome::Expired | EventOutcome::Mismatched { .. }
            ),
            NodeOutcome::ExternalAgent { .. } => false,
        })
        .count()
}

fn node_outcome_json(outcome: &NodeOutcome) -> Value {
    match outcome {
        NodeOutcome::Model {
            node_id,
            outcome,
            result,
            replaced,
        } => json!({
            "node_id": node_id,
            "kind": "model.call",
            "outcome": model_outcome_json(outcome),
            "result": result,
            "replaced": replaced,
        }),
        NodeOutcome::Capability { node_id, outcome } => json!({
            "node_id": node_id,
            "kind": "capability.invoke",
            "outcome": capability_outcome_json(outcome),
        }),
        NodeOutcome::ExternalAgent { node_id, evidence } => json!({
            "node_id": node_id,
            "kind": "capability.invoke.external_agent",
            "outcome": "completed",
            "evidence": evidence,
        }),
        NodeOutcome::ProgramNew { node_id, outcome } => json!({
            "node_id": node_id,
            "kind": "program.new",
            "outcome": composition_outcome_json(outcome),
        }),
        NodeOutcome::ProgramInvoke { node_id, outcome } => json!({
            "node_id": node_id,
            "kind": "program.invoke",
            "outcome": composition_outcome_json(outcome),
        }),
        NodeOutcome::AwaitEvent { node_id, outcome } => json!({
            "node_id": node_id,
            "kind": "await.event",
            "outcome": event_outcome_json(outcome),
        }),
    }
}

fn model_outcome_json(outcome: &ModelOutcome) -> Value {
    match outcome {
        ModelOutcome::CommittedSuccess { usage } => json!({
            "status": "committed_success",
            "usage": usage,
        }),
        ModelOutcome::TypedFailure { error } => json!({
            "status": "typed_failure",
            "error": error,
        }),
        ModelOutcome::Cancelled => json!({"status": "cancelled"}),
        ModelOutcome::ModelOutcomeUnknown { uncertain_usage } => json!({
            "status": "model_outcome_unknown",
            "uncertain_usage": uncertain_usage,
        }),
    }
}

fn capability_outcome_json(outcome: &CapabilityOutcome) -> Value {
    match outcome {
        CapabilityOutcome::Completed { result } => {
            json!({"status": "completed", "result": result})
        }
        CapabilityOutcome::Failed { message } => json!({"status": "failed", "message": message}),
        CapabilityOutcome::OutcomeUnknown { message } => {
            json!({"status": "outcome_unknown", "message": message})
        }
    }
}

fn composition_outcome_json(outcome: &CompositionOutcome) -> Value {
    match outcome {
        CompositionOutcome::Created { child_instance_ref } => {
            json!({"status": "created", "child_instance_ref": child_instance_ref})
        }
        CompositionOutcome::Invoked { child_instance_ref } => {
            json!({"status": "invoked", "child_instance_ref": child_instance_ref})
        }
        CompositionOutcome::Failed { message } => json!({"status": "failed", "message": message}),
    }
}

fn event_outcome_json(outcome: &EventOutcome) -> Value {
    match outcome {
        EventOutcome::Fulfilled { event_ref, payload } => {
            json!({"status": "fulfilled", "event_ref": event_ref, "payload": payload})
        }
        EventOutcome::Parked => json!({"status": "parked"}),
        EventOutcome::Expired => json!({"status": "expired"}),
        EventOutcome::Cancelled => json!({"status": "cancelled"}),
        EventOutcome::Mismatched {
            delivered_event_ref,
        } => json!({
            "status": "mismatched",
            "delivered_event_ref": delivered_event_ref,
        }),
    }
}

fn commit_result_json(result: &ExecutionCommitResult) -> Value {
    match result {
        ExecutionCommitResult::Committed {
            new_program_state_version,
            evidence_position_ref,
        } => json!({
            "status": "committed",
            "new_program_state_version": new_program_state_version,
            "evidence_position_ref": evidence_position_ref,
        }),
        ExecutionCommitResult::CompareConflict {
            current_program_state_version,
        } => json!({
            "status": "compare_conflict",
            "current_program_state_version": current_program_state_version,
        }),
        ExecutionCommitResult::OutcomeUnknown { reconciliation_ref } => json!({
            "status": "outcome_unknown",
            "reconciliation_ref": reconciliation_ref,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_backends::llm::backends::mock::{MockLLMBackend, MockResponse};
    use apxm_inference::ModelTargetRef;

    const TEST_RELEASE_BYTES: &[u8] = b"reference-release-v1";
    const TEST_PROVENANCE_BYTES: &[u8] = b"reference-provenance-v1";

    fn bytes_digest(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    fn invocation_admission(air: &AirModule, invocation_id: &str) -> InvocationAdmission {
        let artifact_bytes = serde_json::to_vec(air).expect("test AIR serialization");
        InvocationAdmission {
            schema_version: apxm_kernel::INVOCATION_ADMISSION_SCHEMA.into(),
            invocation_id: invocation_id.into(),
            artifact_digest: bytes_digest(&artifact_bytes),
            release_digest: bytes_digest(TEST_RELEASE_BYTES),
            port_bindings_digest: canonical_port_bindings_digest(),
            resource_ceiling_digest: canonical_resource_ceiling_digest(),
            provenance_digest: bytes_digest(TEST_PROVENANCE_BYTES),
        }
    }

    async fn admitted_profile(
        air: &AirModule,
        invocation_id: &str,
        commit: Arc<DevCommit>,
    ) -> RuntimeProfile {
        let descriptor = canonical_runtime_descriptor();
        let admission = invocation_admission(air, invocation_id);
        let artifact_bytes = serde_json::to_vec(air).expect("test AIR serialization");
        let verified = verify_invocation_admission(
            &admission,
            &artifact_bytes,
            TEST_RELEASE_BYTES,
            TEST_PROVENANCE_BYTES,
            &descriptor.port_bindings,
            descriptor.resource_ceilings,
            &descriptor.confinement,
        )
        .expect("test invocation admission");
        runtime_profile_from_invocation(
            commit,
            Arc::new(LocalCapabilityPort::new().expect("local capability port")),
            Arc::new(LocalModelInferencePort::from_backend_roster().expect("local inference port")),
            Arc::new(LocalModelRequestMetadata),
            verified,
            "test.execution",
        )
        .await
        .expect("fully admitted profile")
    }

    fn empty_profile_air() -> AirModule {
        serde_json::from_value(json!({
            "schema_version": "apxm.air",
            "semantic_operations": [],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("empty profile AIR")
    }

    fn profile_request(air: AirModule, suffix: &str) -> ExecutionRequest {
        ExecutionRequest {
            model_admission: model_admission(&air, DEV_BINDING_DIGEST),
            initial_values: initial_model_request_values(&air),
            capability_invocations: local_capability_invocation_admissions(&air)
                .expect("local capability admissions"),
            air,
            hook_bindings: Vec::new(),
            program_instance_ref: ProgramInstanceRef::new(format!("profile.instance.{suffix}")),
            program_invocation_ref: ProgramInvocationRef::new(format!(
                "profile.invocation.{suffix}"
            )),
            commit_id: format!("profile.commit.{suffix}"),
            write_set: dev_write_set(),
        }
    }

    #[tokio::test]
    async fn canonical_runtime_requires_transport_invocation_admission() {
        let runtime = CanonicalRuntime::new();
        let air = empty_profile_air();
        let mut admission = invocation_admission(&air, "invocation.canonical.unauthorized");
        let artifact_bytes = serde_json::to_vec(&air).expect("test AIR serialization");
        admission.artifact_digest = digest('f');
        let output = runtime
            .execute(
                air,
                &artifact_bytes,
                &admission,
                TEST_RELEASE_BYTES,
                TEST_PROVENANCE_BYTES,
            )
            .await;
        assert!(output.is_err(), "tampered admission must fail closed");
    }

    #[tokio::test]
    async fn canonical_runtime_rejects_a_tampered_but_valid_air_module() {
        let runtime = CanonicalRuntime::new();
        let original = empty_profile_air();
        let admission = invocation_admission(&original, "invocation.canonical.air-drift");
        let mut tampered_json = serde_json::to_value(&original).expect("AIR value");
        tampered_json["source_map"]["source_language"] = Value::String("typescript".into());
        let tampered: AirModule =
            serde_json::from_value(tampered_json).expect("tampered AIR remains well-formed");
        let tampered_bytes = serde_json::to_vec(&tampered).expect("tampered AIR serialization");
        assert!(
            tampered.verify().is_accepted(),
            "tampered AIR remains valid"
        );

        let error = runtime
            .execute(
                tampered,
                &tampered_bytes,
                &admission,
                TEST_RELEASE_BYTES,
                TEST_PROVENANCE_BYTES,
            )
            .await
            .expect_err("valid AIR with different bytes must fail closed");
        assert!(error.to_string().contains("artifact digest mismatch"));
    }

    #[tokio::test]
    async fn canonical_execution_enters_through_the_verified_invocation_admission() {
        let runtime = CanonicalRuntime::new();
        let air = empty_profile_air();
        let admission = invocation_admission(&air, "invocation.canonical.1");
        let artifact_bytes = serde_json::to_vec(&air).expect("test AIR serialization");
        let output = runtime
            .execute(
                air,
                &artifact_bytes,
                &admission,
                TEST_RELEASE_BYTES,
                TEST_PROVENANCE_BYTES,
            )
            .await
            .expect("verified transport authority reaches canonical runtime");
        assert_eq!(output["runtime"], "apxm_execution");
        assert_eq!(output["status"], "completed");
    }

    #[tokio::test]
    async fn invocation_admission_is_carried_into_atomic_runtime_commit() {
        let runtime = CanonicalRuntime::new();
        let air = empty_profile_air();
        let admission = invocation_admission(&air, "invocation.canonical.commit");
        let artifact_bytes = serde_json::to_vec(&air).expect("test AIR serialization");
        let output = runtime
            .execute(
                air,
                &artifact_bytes,
                &admission,
                TEST_RELEASE_BYTES,
                TEST_PROVENANCE_BYTES,
            )
            .await
            .expect("exact host admission reaches canonical runtime");
        assert_eq!(output["status"], "completed");
        assert_eq!(output["commit"]["status"], "committed");
    }

    #[tokio::test]
    async fn invocation_admission_provenance_drift_fails_before_commit() {
        let runtime = CanonicalRuntime::new();
        let air = empty_profile_air();
        let admission = invocation_admission(&air, "invocation.canonical.negative");
        let artifact_bytes = serde_json::to_vec(&air).expect("test AIR serialization");
        let error = runtime
            .execute(
                air,
                &artifact_bytes,
                &admission,
                TEST_RELEASE_BYTES,
                b"tampered-provenance",
            )
            .await
            .expect_err("provenance drift must fail closed");
        assert!(error.to_string().contains("provenance digest mismatch"));
    }

    #[tokio::test]
    async fn profile_shutdown_rejects_new_work_fail_closed() {
        let air = empty_profile_air();
        let profile = admitted_profile(
            &air,
            "invocation.profile.closed",
            Arc::new(DevCommit::default()),
        )
        .await;
        profile.shutdown();

        let error = profile
            .execute(profile_request(empty_profile_air(), "closed"), Value::Null)
            .await
            .expect_err("closed profile must reject new work");
        assert!(matches!(error, RuntimeProfileError::Closed));
    }

    #[test]
    fn profile_admission_rejects_missing_required_slot() {
        let air = empty_profile_air();
        let descriptor = canonical_runtime_descriptor();
        let bindings = descriptor
            .port_bindings
            .into_iter()
            .filter(|binding| binding.slot != PortSlot::Confinement.as_str())
            .collect::<Vec<_>>();
        let admission = invocation_admission(&air, "invocation.profile.no-confinement");
        let artifact_bytes = serde_json::to_vec(&air).expect("test AIR serialization");
        let error = verify_invocation_admission(
            &admission,
            &artifact_bytes,
            TEST_RELEASE_BYTES,
            TEST_PROVENANCE_BYTES,
            &bindings,
            canonical_runtime_descriptor().resource_ceilings,
            &canonical_runtime_descriptor().confinement,
        )
        .expect_err("missing confinement binding must fail closed");
        assert!(error.to_string().contains("port binding digest mismatch"));
    }

    #[tokio::test]
    async fn profile_recovery_is_instance_local_and_new_profile_can_run() {
        let air = empty_profile_air();
        let stopped = admitted_profile(
            &air,
            "invocation.profile.stopped",
            Arc::new(DevCommit::default()),
        )
        .await;
        stopped.shutdown();

        let recovered = admitted_profile(
            &air,
            "invocation.profile.recovered",
            Arc::new(DevCommit::default()),
        )
        .await;
        let report = recovered
            .execute(
                profile_request(empty_profile_air(), "recovered"),
                Value::Null,
            )
            .await
            .expect("new profile accepts work after recovery");
        assert!(matches!(
            report.commit,
            ExecutionCommitResult::Committed { .. }
        ));
        assert!(!stopped.is_accepting());
        assert!(recovered.is_accepting());
    }

    /// The shipped metadata port. It replaced a test-only double that minted
    /// placeholder identities; the real one carries the runtime's own request
    /// and context digests, so these tests now exercise the production seam.
    fn local_model_request_metadata() -> Arc<dyn ModelCallRequestMetadataPort> {
        Arc::new(LocalModelRequestMetadata)
    }

    #[tokio::test]
    async fn executes_canonical_air_with_dev_ports() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air",
            "semantic_operations": [
                {"node_id": "n.model", "op": "model.call", "parent_region_id": "r.root", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.model.request", "type_ref": "ModelRequest"}], "result": {"value_id": "value.model.output", "type_ref": "ModelOutput"}},
                {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.root", "execution_order": 1, "operands": [{"slot": "program_ref", "value_id": "child", "type_ref": "ProgramRef"}], "result": {"value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}},
                {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.root", "execution_order": 2, "operands": [{"slot": "receiver", "value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}, {"slot": "input", "value_id": "value.program.input", "type_ref": "ProgramInput"}], "result": {"value_id": "value.program.output", "type_ref": "ProgramOutput"}},
                {"node_id": "n.await", "op": "await.event", "parent_region_id": "r.root", "execution_order": 3, "operands": [{"slot": "event_ref", "value_id": "ready", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}}
            ],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("canonical air");
        assert!(air.verify().is_accepted());

        let commit = Arc::new(DevCommit::default());
        let ports = dev_ports(
            commit,
            Arc::new(LocalCapabilityPort::new().expect("local capability port")),
            Arc::new(LocalModelInferencePort::from_backend_roster().expect("local inference port")),
            local_model_request_metadata(),
        )
        .expect("development ports form an admitted bundle");
        let model_admission = model_admission(&air, DEV_BINDING_DIGEST);
        let capability_invocations =
            local_capability_invocation_admissions(&air).expect("local capability admissions");
        let report = execute(
            &ports,
            ExecutionRequest {
                initial_values: initial_model_request_values(&air),
                air,
                hook_bindings: Vec::new(),
                model_admission,
                capability_invocations,
                program_instance_ref: ProgramInstanceRef::new("test.instance"),
                program_invocation_ref: ProgramInvocationRef::new("test.invocation"),
                commit_id: "test.commit".into(),
                write_set: dev_write_set(),
            },
            Value::Null,
        )
        .await
        .expect("canonical execution");

        assert_eq!(report.node_outcomes.len(), 4);
        assert!(matches!(
            report.commit,
            ExecutionCommitResult::Committed { .. }
        ));
    }

    #[tokio::test]
    async fn executes_canonical_air_without_a_model_binding() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air",
            "semantic_operations": [
                {"node_id": "n.new", "op": "program.new", "parent_region_id": "r.root", "execution_order": 0, "operands": [{"slot": "program_ref", "value_id": "child", "type_ref": "ProgramRef"}], "result": {"value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}},
                {"node_id": "n.invoke", "op": "program.invoke", "parent_region_id": "r.root", "execution_order": 1, "operands": [{"slot": "receiver", "value_id": "value.program.instance", "type_ref": "ProgramInstanceRef"}, {"slot": "input", "value_id": "value.program.input", "type_ref": "ProgramInput"}], "result": {"value_id": "value.program.output", "type_ref": "ProgramOutput"}},
                {"node_id": "n.await", "op": "await.event", "parent_region_id": "r.root", "execution_order": 2, "operands": [{"slot": "event_ref", "value_id": "ready", "type_ref": "EventRef"}], "result": {"value_id": "value.event.output", "type_ref": "EventOutput"}}
            ],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("canonical air without a model call");
        assert!(air.verify().is_accepted());

        let model_admission = model_admission(&air, DEV_BINDING_DIGEST);
        assert!(matches!(
            model_admission.validate(&ModelTargetRef("model.absent".into())),
            Err(apxm_inference::BindingError::MissingTarget(_))
        ));

        let commit = Arc::new(DevCommit::default());
        let ports = dev_ports(
            commit,
            Arc::new(LocalCapabilityPort::new().expect("local capability port")),
            Arc::new(LocalModelInferencePort::from_backend_roster().expect("local inference port")),
            local_model_request_metadata(),
        )
        .expect("development ports form an admitted bundle");
        let capability_invocations =
            local_capability_invocation_admissions(&air).expect("local capability admissions");
        let report = execute(
            &ports,
            ExecutionRequest {
                model_admission,
                initial_values: initial_model_request_values(&air),
                air,
                hook_bindings: Vec::new(),
                capability_invocations,
                program_instance_ref: ProgramInstanceRef::new("test.instance.no-model"),
                program_invocation_ref: ProgramInvocationRef::new("test.invocation.no-model"),
                commit_id: "test.commit.no-model".into(),
                write_set: dev_write_set(),
            },
            Value::Null,
        )
        .await
        .expect("canonical execution without a model binding");

        assert_eq!(report.node_outcomes.len(), 3);
        assert!(matches!(
            report.commit,
            ExecutionCommitResult::Committed { .. }
        ));
    }

    /// Build a one-node `capability.invoke` AIR whose arguments are an authored
    /// object literal, so the driver materializes them without any host input.
    fn capability_air(capability_ref: &str, arguments: Value) -> AirModule {
        let fields = arguments
            .as_object()
            .expect("authored capability arguments are an object")
            .iter()
            .map(|(name, value)| {
                json!({
                    "name": name,
                    "value": {"kind": "string", "value": value.as_str().expect("string field")},
                })
            })
            .collect::<Vec<_>>();
        serde_json::from_value(json!({
            "schema_version": "apxm.air",
            "value_assemblies": [
                {"value_id": "value.capability.arguments", "expression": {"kind": "object", "fields": fields}}
            ],
            "semantic_operations": [
                {"node_id": "n.cap", "op": "capability.invoke", "parent_region_id": "r.root", "execution_order": 0, "operands": [{"slot": "capability_ref", "value_id": capability_ref, "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.capability.arguments", "type_ref": "ArgumentValue"}], "result": {"value_id": "value.capability.output", "type_ref": "ToolOutput"}}
            ],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("canonical Capability AIR")
    }

    /// Replaces the retired `rejects_local_capability_without_invocation_admission_authority`
    /// guard test. That guard refused every non-`external-agent:` Capability
    /// outright; the composition root now mints the local authority instead, so
    /// the pinned behavior is that an admission exists, is keyed by node id, and
    /// binds exactly the authored capability reference.
    #[test]
    fn mints_a_local_invocation_admission_for_each_authored_capability_node() {
        let air = capability_air("read", json!({"file_path": "Cargo.toml"}));

        let admissions =
            local_capability_invocation_admissions(&air).expect("local capability admissions");

        let admission = admissions
            .get("n.cap")
            .expect("each authored capability node is admitted under its node id");
        assert_eq!(admission.capability_ref, "read");
        assert_eq!(
            admission.authority.acting_principal_ref().target,
            LOCAL_ACTING_PRINCIPAL_REF,
            "the admission names the local composition root, not a server-issued principal"
        );
        assert_eq!(
            admission.authority.capability_grant_ref().target,
            format!("{LOCAL_CAPABILITY_GRANT_PREFIX}read"),
            "the grant is minted per authored capability reference"
        );
        assert!(admission.authority.approval_refs().is_empty());
    }

    #[test]
    fn an_air_with_no_capability_node_admits_nothing() {
        assert!(
            local_capability_invocation_admissions(&empty_profile_air())
                .expect("local capability admissions")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn an_authored_capability_invoke_runs_a_real_capability_end_to_end() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let file = directory.path().join("payload.txt");
        std::fs::write(&file, "canonical end to end payload\n").expect("write payload");

        let runtime = CanonicalRuntime::new();
        let air = capability_air(
            "read",
            json!({"file_path": file.to_str().expect("utf-8 path")}),
        );
        assert!(air.verify().is_accepted());
        let admission = invocation_admission(&air, "invocation.canonical.capability");
        let artifact_bytes = serde_json::to_vec(&air).expect("test AIR serialization");

        let output = runtime
            .execute(
                air,
                &artifact_bytes,
                &admission,
                TEST_RELEASE_BYTES,
                TEST_PROVENANCE_BYTES,
            )
            .await
            .expect("an admitted Capability reaches the local capability surface");

        let outcome = &output["results"]["node_outcomes"][0];
        assert_eq!(outcome["kind"], "capability.invoke");
        assert_eq!(
            outcome["outcome"]["status"], "completed",
            "the shipped path now runs a tool: {output}"
        );
        assert!(
            outcome["outcome"]["result"]
                .as_str()
                .expect("capability result text")
                .contains("canonical end to end payload"),
            "the read capability returns file contents: {output}"
        );
    }

    #[tokio::test]
    async fn an_unadmitted_capability_fails_closed_without_applying_its_effect() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let target = directory.path().join("must-not-exist.txt");

        let runtime = CanonicalRuntime::new();
        let air = capability_air(
            "write",
            json!({
                "file_path": target.to_str().expect("utf-8 path"),
                "content": "this effect must never be applied",
            }),
        );
        let admission = invocation_admission(&air, "invocation.canonical.capability-denied");
        let artifact_bytes = serde_json::to_vec(&air).expect("test AIR serialization");

        let output = runtime
            .execute(
                air,
                &artifact_bytes,
                &admission,
                TEST_RELEASE_BYTES,
                TEST_PROVENANCE_BYTES,
            )
            .await
            .expect("a denied Capability is a typed node outcome, not a runtime abort");

        assert_eq!(
            output["stats"]["failed_nodes"], 1,
            "a denied Capability is reported as a failed node: {output}"
        );
        let outcome = &output["results"]["node_outcomes"][0];
        assert_eq!(outcome["outcome"]["status"], "failed");
        assert!(
            outcome["outcome"]["message"]
                .as_str()
                .expect("failure message")
                .contains("not admitted by canonical local execution"),
            "the denial names the local admission policy: {output}"
        );
        assert!(
            !target.exists(),
            "the denial happens before the implementation receives its arguments"
        );
    }

    /// The exact model reference the checked-in canonical fixture authors.
    const FIXTURE_MODEL_TARGET: &str = "apxm.canonical.fixture.model";

    /// Read one checked-in fixture, returning both the parsed value and the
    /// exact bytes the Invocation Admission is a digest over.
    fn checked_in_fixture(name: &str) -> (AirModule, Vec<u8>) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tools/tests/fixtures")
            .join(name);
        let bytes = std::fs::read(&path).expect("checked-in fixture bytes");
        let air = serde_json::from_slice(&bytes).expect("checked-in fixture AIR");
        (air, bytes)
    }

    fn checked_in_admission(name: &str) -> InvocationAdmission {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tools/tests/fixtures")
            .join(name);
        serde_json::from_slice(&std::fs::read(&path).expect("checked-in admission bytes"))
            .expect("checked-in Invocation Admission")
    }

    /// Drive the checked-in canonical fixture — which authors `model.call` and
    /// `capability.invoke` in one module — through the shipped execution body.
    ///
    /// The fixture's capability arguments are repository-relative, so only the
    /// model node is asserted here; the capability nodes are covered from the
    /// repository root by the CLI integration test.
    async fn execute_canonical_fixture(model: Arc<LocalModelInferencePort>) -> Value {
        let (air, artifact_bytes) = checked_in_fixture("canonical-capability-execute.air.json");
        assert!(
            air.verify().is_accepted(),
            "the checked-in fixture is valid AIR"
        );
        let admission =
            checked_in_admission("canonical-capability-execute.invocation-admission.json");
        let release = std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../tools/tests/fixtures/canonical-execute.release.json"),
        )
        .expect("release fixture bytes");
        let provenance = std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../tools/tests/fixtures/canonical-execute.provenance.json"),
        )
        .expect("provenance fixture bytes");

        CanonicalRuntime::new()
            .execute_with_model(
                air,
                &artifact_bytes,
                &admission,
                &release,
                &provenance,
                model,
            )
            .await
            .expect("the checked-in fixture is admitted")
    }

    fn fixture_node_outcome<'a>(output: &'a Value, node_id: &str) -> &'a Value {
        output["results"]["node_outcomes"]
            .as_array()
            .expect("node outcomes")
            .iter()
            .find(|outcome| outcome["node_id"] == node_id)
            .unwrap_or_else(|| panic!("no node outcome for {node_id}: {output}"))
    }

    #[tokio::test]
    async fn an_authored_model_call_commits_a_real_backend_response() {
        let backend = MockLLMBackend::new()
            .named("fixture-inference")
            .model_name(FIXTURE_MODEL_TARGET)
            .default(MockResponse::new("the fixture model answered"));
        let port = LocalModelInferencePort::for_bound_backend(
            "fixture-inference",
            FIXTURE_MODEL_TARGET,
            Arc::new(backend),
        )
        .expect("bound inference port");

        let output = execute_canonical_fixture(Arc::new(port)).await;

        let model = fixture_node_outcome(&output, "fixture.model.call");
        assert_eq!(model["kind"], "model.call");
        assert_eq!(
            model["outcome"]["status"], "committed_success",
            "an authored model.call now commits a real backend response: {output}"
        );
        assert_eq!(
            model["result"]["content"], "the fixture model answered",
            "the committed output is the backend's answer, not a sentinel: {output}"
        );
        assert_eq!(
            output["llm_usage"]["input_tokens"], 10,
            "usage is the backend's reported usage: {output}"
        );
        assert_eq!(output["llm_usage"]["output_tokens"], 20);
        assert_eq!(output["llm_usage"]["total_requests"], 1);
    }

    #[tokio::test]
    async fn an_unregistered_model_target_degrades_to_a_typed_configuration_failure() {
        let port = LocalModelInferencePort::unconfigured(
            "backend 'gateway' is unusable: Environment variable 'LLM_GATEWAY_KEY' not set",
        )
        .expect("unconfigured inference port");

        let output = execute_canonical_fixture(Arc::new(port)).await;

        let model = fixture_node_outcome(&output, "fixture.model.call");
        assert_eq!(
            model["outcome"]["status"], "typed_failure",
            "an unconfigured model target fails typed, never silently: {output}"
        );
        assert_eq!(
            model["outcome"]["error"]["category"], "configuration",
            "{output}"
        );
        assert_eq!(
            model["outcome"]["error"]["code"], "model_target_not_registered",
            "{output}"
        );
        assert!(
            model["outcome"]["error"]["message"]
                .as_str()
                .expect("typed error message")
                .contains("LLM_GATEWAY_KEY"),
            "the typed error names why no backend was admitted: {output}"
        );
        assert!(
            model["result"].is_null(),
            "a failed model effect never manufactures an output value: {output}"
        );
        assert!(
            !output["results"]["model_attempt_diagnostics"]
                .as_array()
                .expect("model attempt diagnostics")
                .is_empty(),
            "the adapter reports what it observed: {output}"
        );
    }

    #[test]
    fn admits_each_distinct_authored_model_target_once() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air",
            "semantic_operations": [
                {"node_id": "n.model.first", "op": "model.call", "parent_region_id": "r.root", "execution_order": 0, "operands": [{"slot": "model_ref", "value_id": "model.target.first", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.request.first", "type_ref": "ModelRequest"}], "result": {"value_id": "value.output.first", "type_ref": "ModelOutput"}},
                {"node_id": "n.model.second", "op": "model.call", "parent_region_id": "r.root", "execution_order": 1, "operands": [{"slot": "model_ref", "value_id": "model.target.second", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.request.second", "type_ref": "ModelRequest"}], "result": {"value_id": "value.output.second", "type_ref": "ModelOutput"}},
                {"node_id": "n.model.first.again", "op": "model.call", "parent_region_id": "r.root", "execution_order": 2, "operands": [{"slot": "model_ref", "value_id": "model.target.first", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.request.third", "type_ref": "ModelRequest"}], "result": {"value_id": "value.output.third", "type_ref": "ModelOutput"}}
            ],
            "structural_ir": [{"region_id": "r.root", "kind": "function", "execution_order": 0}],
            "context_flow": [],
            "source_map": {"schema_version": "apxm.source-map", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("canonical air with distinct model targets");
        assert!(air.verify().is_accepted());

        let admission = model_admission(&air, DEV_BINDING_DIGEST);
        for target in ["model.target.first", "model.target.second"] {
            let resolved = admission
                .validate(&ModelTargetRef(target.into()))
                .expect("authored target has exactly one local binding");
            assert_eq!(resolved.model_target.reference.0, target);
        }
    }
}
