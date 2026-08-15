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
//!
//! A package-shipped Capability crosses exactly the same three boundaries.
//! It is registered into the same [`CapabilitySystem`] as a builtin, so its
//! permission decision, its admission, its schema validation, and its
//! evidence are the builtin path rather than a parallel one; the only thing
//! that differs is which implementation the registry hands back.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use apxm_capability::CapabilitySystem;
use apxm_capability::builtins::{
    BashConfig, SkillRootConfig, SkillsConfig, ToolsConfig, register_standard_tools,
};
use apxm_capability::executor::CapabilityExecutor;
use apxm_capability::interceptor::{CapabilityInterceptor, InterceptDecision};
use apxm_capability::metadata::RuntimeCapability;
use apxm_core::error::RuntimeError;
use apxm_core::types::values::{Value as CapabilityValue, ValueError};
use apxm_core::types::{HandlerDescriptor, HandlerManifest};
use apxm_execution::{CapabilityOutcome, CapabilityPort, CapabilityRequest};
use apxm_program::skill::RootTier;
use async_trait::async_trait;

/// Interceptor name reported by the local admission gate.
const LOCAL_ADMISSION_INTERCEPTOR: &str = "canonical-local-admission";

/// Discovery-root id for the `skills/` directory of the package under
/// execution. Distinct from the project and local roots so an operator
/// reading evidence can tell which root served an instruction.
const PACKAGE_SKILL_ROOT_ID: &str = "package";

/// Standard-tool configuration for a composition root with no admitted sandbox.
///
/// `bash` declares an `ExecRequest` through
/// `CapabilityExecutor::to_exec_request`, so `CapabilitySystem` *always* routes
/// it through a sandbox backend and `BashCapability::execute` refuses direct
/// execution outright. With no sandbox registry bound, registering `bash` would
/// publish a capability that fails 100% of the time, so the local root leaves it
/// unregistered instead of advertising it. A sandbox-backed local profile is the
/// thing that turns it back on, not a config toggle here.
fn local_tools_config(package_root: Option<&Path>) -> ToolsConfig {
    ToolsConfig {
        bash: BashConfig {
            enabled: false,
            ..BashConfig::default()
        },
        skills: local_skills_config(package_root),
        ..ToolsConfig::default()
    }
}

/// The discovery roots canonical local execution publishes.
///
/// `SkillsConfig` defaults to no roots on purpose — a capability that went
/// looking for skills on its own initiative would be reading whatever
/// happened to be near the process. Naming them is the host's job, and this
/// is the host: canonical local execution runs inside a project, so it
/// publishes that project's conventional roots, both relative to the
/// working directory the CLI was invoked from. A root that does not exist
/// contributes nothing rather than failing.
///
/// A package under execution publishes its own `skills/` as a third root.
/// Without it a program could declare a Skill, ship the `SKILL.md` the
/// package format recognises, pass every gate, and then fail at
/// `read_skill` because no root ever named the directory sitting inside the
/// package being run. The package path arrives already integrity-verified,
/// so the bytes served are the bytes the chain covers.
fn local_skills_config(package_root: Option<&Path>) -> SkillsConfig {
    let mut roots = vec![
        SkillRootConfig {
            root_id: "project".to_string(),
            tier: RootTier::Project,
            path: PathBuf::from(".agents/skills"),
        },
        SkillRootConfig {
            root_id: "local".to_string(),
            tier: RootTier::Local,
            path: PathBuf::from(".apxm/skills"),
        },
    ];
    if let Some(package_root) = package_root {
        roots.push(SkillRootConfig {
            root_id: PACKAGE_SKILL_ROOT_ID.to_string(),
            tier: RootTier::Local,
            path: package_root.join("skills"),
        });
    }
    SkillsConfig {
        enabled: true,
        roots,
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

/// The private handler-worker adapter for package-shipped Capabilities.
///
/// ADR-0016 keeps the execution boundary in Rust and calls a language
/// helper process a *private* adapter the Composition Root selects. This is
/// that adapter, and its privacy is structural: the worker command and the
/// manifest are both handed in, the process is spawned lazily on the first
/// invocation so a run that never invokes a package Capability never starts
/// one, and nothing outside this module can address it.
///
/// Every language's worker is this same type. The frame protocol is the
/// contract, not the interpreter, so a Python worker and a Node worker are
/// two commands rather than two adapters.
///
/// Requests are serialized behind one lock. The frame protocol correlates
/// by `req_id` and could pipeline, but a single in-flight request is what
/// makes "the reply I read is the reply to the frame I wrote" a property of
/// the code rather than of the worker's scheduling.
struct PackageHandlerWorker {
    command: crate::PackageHandlerWorkerCommand,
    /// The validated manifest, materialized so the worker evaluates exactly
    /// the bytes this root admitted rather than re-reading a package path
    /// that may have changed since verification.
    manifest_file: tempfile::TempPath,
    process: tokio::sync::Mutex<Option<WorkerProcess>>,
    next_request: AtomicU64,
}

/// One live worker process and the two halves of its frame transport.
struct WorkerProcess {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
}

impl PackageHandlerWorker {
    /// Materialize the admitted manifest beside the selected worker entry.
    fn new(
        command: &crate::PackageHandlerWorkerCommand,
        manifest: &HandlerManifest,
    ) -> Result<Self, RuntimeError> {
        let mut file = tempfile::NamedTempFile::new().map_err(|error| {
            RuntimeError::Executor(format!(
                "the package handler worker manifest could not be materialized: {error}"
            ))
        })?;
        serde_json::to_writer(&mut file, manifest).map_err(|error| {
            RuntimeError::Executor(format!(
                "the admitted handler manifest is not serializable: {error}"
            ))
        })?;
        Ok(Self {
            command: command.clone(),
            manifest_file: file.into_temp_path(),
            process: tokio::sync::Mutex::new(None),
            next_request: AtomicU64::new(1),
        })
    }

    /// Send one frame and read its reply, starting the worker if needed.
    async fn invoke(
        &self,
        capability: &str,
        handler_id: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        use tokio::io::AsyncWriteExt;

        let failure = |message: String| RuntimeError::Capability {
            capability: capability.to_string(),
            message,
        };
        let request_id = self
            .next_request
            .fetch_add(1, Ordering::Relaxed)
            .to_string();
        let frame = serde_json::json!({
            "v": 1,
            "req_id": request_id,
            "tool_id": handler_id,
            "args": args,
        });
        let mut line = serde_json::to_string(&frame).map_err(|error| {
            failure(format!("the invocation frame is not encodable: {error}"))
        })?;
        line.push('\n');

        let mut guard = self.process.lock().await;
        if guard.is_none() {
            *guard = Some(self.spawn().map_err(failure)?);
        }
        let worker = guard.as_mut().expect("the worker was just started");

        // Anything that goes wrong from here leaves the worker's state
        // unknown, so it is torn down and the next invocation starts a
        // clean one rather than reusing a stream mid-frame.
        let exchange = async {
            worker
                .stdin
                .write_all(line.as_bytes())
                .await
                .map_err(|error| format!("the worker did not accept the request: {error}"))?;
            worker
                .stdin
                .flush()
                .await
                .map_err(|error| format!("the worker did not accept the request: {error}"))?;
            match worker.stdout.next_line().await {
                Ok(Some(reply)) => Ok(reply),
                Ok(None) => Err("the worker closed its output before replying".to_string()),
                Err(error) => Err(format!("the worker reply could not be read: {error}")),
            }
        }
        .await;
        let reply = match exchange {
            Ok(reply) => reply,
            Err(message) => {
                if let Some(mut worker) = guard.take() {
                    let _ = worker.child.start_kill();
                }
                // Only a `read_only` handler is ever admitted by this root
                // (see `admitted_capability_names`), so a lost reply leaves
                // no external effect in doubt: reporting a definite failure
                // is accurate here rather than optimistic.
                return Err(failure(message));
            }
        };
        drop(guard);

        let reply: WorkerReply = serde_json::from_str(&reply).map_err(|error| {
            failure(format!("the worker reply is not a result frame: {error}"))
        })?;
        if reply.req_id != request_id {
            return Err(failure(format!(
                "the worker replied to request '{}' while '{request_id}' was outstanding",
                reply.req_id
            )));
        }
        match (reply.ok, reply.value, reply.error) {
            (true, Some(value), _) => Ok(value),
            (true, None, _) => Err(failure(
                "the worker reported success without a result value".to_string(),
            )),
            (false, _, Some(error)) => Err(failure(error)),
            (false, _, None) => Err(failure(
                "the worker reported failure without a reason".to_string(),
            )),
        }
    }

    /// Start the selected worker over the admitted manifest.
    fn spawn(&self) -> Result<WorkerProcess, String> {
        let mut child = tokio::process::Command::new(&self.command.interpreter)
            .arg(&self.command.entry)
            .arg(&self.manifest_file)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                format!(
                    "the package handler worker {} could not be started with {}: {error}",
                    self.command.entry.display(),
                    self.command.interpreter
                )
            })?;
        let stdin = child.stdin.take().ok_or("the worker has no input stream")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("the worker has no output stream")?;
        Ok(WorkerProcess {
            child,
            stdin,
            stdout: tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(stdout)),
        })
    }
}

/// The worker's result frame.
#[derive(serde::Deserialize)]
struct WorkerReply {
    req_id: String,
    ok: bool,
    #[serde(default)]
    value: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<String>,
}

/// One package-shipped Capability, registered like any other implementation.
struct PackageHandlerCapability {
    metadata: RuntimeCapability,
    handler_id: String,
    worker: Arc<PackageHandlerWorker>,
}

impl PackageHandlerCapability {
    /// Project one manifest descriptor onto runtime capability metadata.
    ///
    /// Both policy fields are read off the descriptor and neither is
    /// invented here. `read_only` is what the handler declared about
    /// itself; `requires_approval` is the permission decision `apxm agent
    /// sync` resolved and wrote into the manifest. An absent decision is
    /// not an allow — a manifest that never had one carried through is
    /// gated, which fails closed at the approval interceptor rather than
    /// running unapproved.
    fn new(
        descriptor: &HandlerDescriptor,
        worker: Arc<PackageHandlerWorker>,
    ) -> Result<Self, RuntimeError> {
        let schema = descriptor
            .schema
            .clone()
            .ok_or_else(|| RuntimeError::Capability {
                capability: descriptor.name.clone(),
                message: "a package handler is addressable only through its argument schema, \
                          and this descriptor carries none"
                    .to_string(),
            })?;
        let mut metadata = RuntimeCapability::new(
            descriptor.name.clone(),
            descriptor.description.clone().unwrap_or_else(|| {
                format!("Capability '{}' shipped by its package.", descriptor.name)
            }),
            schema,
        );
        if descriptor.read_only == Some(true) {
            metadata = metadata.with_read_only();
        }
        if descriptor.requires_approval != Some(false) {
            metadata = metadata.with_requires_approval();
        }
        Ok(Self {
            metadata,
            handler_id: descriptor.handler_id.clone(),
            worker,
        })
    }
}

#[async_trait]
impl CapabilityExecutor for PackageHandlerCapability {
    async fn execute(
        &self,
        args: HashMap<String, CapabilityValue>,
    ) -> Result<CapabilityValue, RuntimeError> {
        let mut encoded = serde_json::Map::with_capacity(args.len());
        for (name, value) in args {
            let json = value.to_json().map_err(|error| RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!("argument '{name}' has no canonical JSON form: {error}"),
            })?;
            encoded.insert(name, json);
        }
        let value = self
            .worker
            .invoke(
                &self.metadata.name,
                &self.handler_id,
                &serde_json::Value::Object(encoded),
            )
            .await?;
        CapabilityValue::try_from(value).map_err(|error| RuntimeError::Capability {
            capability: self.metadata.name.clone(),
            message: format!("the handler result has no runtime value mapping: {error}"),
        })
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

/// Register every Capability the supplied package ships.
///
/// Registration happens before the admission policy is captured, so a
/// package Capability is in the admitted set on exactly the same terms a
/// builtin is: because its own metadata declares it read-only.
///
/// A package handler may not take a builtin's name. `register` refuses a
/// duplicate, and that refusal is kept rather than softened into a
/// replacement: a package that could shadow `read` could change what every
/// program in it means by reading.
///
/// One worker is started per language, shared by every descriptor in it, so
/// a package shipping both languages runs two processes rather than one per
/// capability. A descriptor whose language this root was supplied no worker
/// for is a registration failure, not a capability that registers and then
/// cannot be dispatched.
fn register_package_handlers(
    system: &CapabilitySystem,
    handlers: &crate::AdmittedPackageHandlers,
) -> Result<(), RuntimeError> {
    let mut workers: BTreeMap<_, Arc<PackageHandlerWorker>> = BTreeMap::new();
    for (language, command) in &handlers.workers {
        // A worker is given only the descriptors it can evaluate, so
        // "exactly the bytes this root admitted" is also exactly the bytes
        // this interpreter can load.
        let evaluable = HandlerManifest::new(
            handlers
                .manifest
                .handlers
                .iter()
                .filter(|descriptor| descriptor.language == *language)
                .cloned()
                .collect(),
        );
        workers.insert(
            *language,
            Arc::new(PackageHandlerWorker::new(command, &evaluable)?),
        );
    }
    for descriptor in &handlers.manifest.handlers {
        let worker =
            workers
                .get(&descriptor.language)
                .ok_or_else(|| RuntimeError::Capability {
                    capability: descriptor.name.clone(),
                    message: format!(
                        "this composition root was supplied no {:?} handler worker",
                        descriptor.language
                    ),
                })?;
        system.register(Arc::new(PackageHandlerCapability::new(
            descriptor,
            Arc::clone(worker),
        )?))?;
    }
    Ok(())
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
    /// run without a sandbox and whatever Capabilities the supplied package
    /// ships, then gate them all behind the read-only admission policy.
    ///
    /// `handlers` is the exact package-handler implementation this root was
    /// supplied with, or `None` when it was supplied with none. There is no
    /// discovery step: an AIR naming a package Capability that no supplied
    /// implementation covers is refused at admission.
    ///
    /// `package_root` publishes that package's own `skills/` directory as a
    /// discovery root, so a Skill the package ships is readable by the
    /// program that declared it. `None` leaves only the project and local
    /// roots, which is what a run with no package on disk should see.
    ///
    /// # Errors
    ///
    /// Returns the registration error if a standard tool or a package
    /// Capability cannot be registered.
    pub fn with_package_root(
        handlers: Option<&crate::AdmittedPackageHandlers>,
        package_root: Option<&Path>,
    ) -> Result<Self, RuntimeError> {
        let system = CapabilitySystem::new();
        register_standard_tools(&system, &local_tools_config(package_root))?;
        if let Some(handlers) = handlers {
            register_package_handlers(&system, handlers)?;
        }
        system.register_interceptor(Arc::new(LocalAdmissionPolicy {
            admitted: admitted_capability_names(&system),
        }));
        Ok(Self { system })
    }

    /// Capability names this port admits, in canonical order.
    ///
    /// The same set the interceptor gates on, published so the composition
    /// root can state it as a permission decision *before* an invocation is
    /// admitted. Two enforcement points, one set: the permission layer
    /// refuses at admission, this port refuses at the chokepoint, and
    /// neither restates the other's policy.
    #[must_use]
    pub fn admitted_names(&self) -> BTreeSet<String> {
        admitted_capability_names(&self.system)
    }

    /// Every capability registered on the local surface, admitted or not.
    ///
    /// This is the resolvable set, not the permitted one: a name in here
    /// has an implementation behind it, whether or not policy lets this
    /// root call it. The composition root grants against this so a
    /// reference nothing can dispatch fails at admission, while a
    /// registered-but-refused reference stays a permission decision.
    #[must_use]
    pub fn registered_names(&self) -> BTreeSet<String> {
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
        let port =
            LocalCapabilityPort::with_package_root(None, None).expect("local capability port");
        let registered = port.registered_names();
        assert!(
            !registered.contains("bash"),
            "bash always routes through a sandbox backend, so an unsandboxed local \
             surface must not publish it: {registered:?}"
        );
    }

    #[test]
    fn only_the_read_only_surface_is_admitted() {
        let port =
            LocalCapabilityPort::with_package_root(None, None).expect("local capability port");
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

        let port =
            LocalCapabilityPort::with_package_root(None, None).expect("local capability port");
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

        let port =
            LocalCapabilityPort::with_package_root(None, None).expect("local capability port");
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
        let port =
            LocalCapabilityPort::with_package_root(None, None).expect("local capability port");
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
        let port =
            LocalCapabilityPort::with_package_root(None, None).expect("local capability port");
        let outcome = port
            .invoke(request("cap.absent", serde_json::json!({})))
            .await;
        assert!(
            matches!(outcome, CapabilityOutcome::Failed { .. }),
            "an unregistered capability never completes: {outcome:?}"
        );
    }

    /// One manifest descriptor, with the two policy fields under test left
    /// to the caller and everything else fixed at a valid shape.
    fn descriptor(
        name: &str,
        read_only: Option<bool>,
        requires_approval: Option<bool>,
    ) -> HandlerDescriptor {
        HandlerDescriptor {
            kind: apxm_core::types::HandlerKind::Tool,
            language: apxm_core::types::HandlerLanguage::TypeScript,
            handler_id: format!("sha256:{}", "a".repeat(64)),
            module: format!("capabilities/{name}/handler"),
            qualname: name.to_string(),
            name: name.to_string(),
            source: apxm_core::types::HandlerSource {
                artifact_path: format!("handlers/{name}.mjs"),
                content: "export const handler = {};\n".to_string(),
            },
            description: Some(format!("package handler {name}")),
            schema: Some(serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false,
            })),
            read_only,
            requires_approval,
        }
    }

        fn supplied(descriptors: Vec<HandlerDescriptor>) -> crate::AdmittedPackageHandlers {
            crate::AdmittedPackageHandlers {
            // Never started: every test here stops at admission, which is
            // the point — a refused package Capability must not reach a
            // worker any more than a refused builtin reaches its
            // implementation.
            workers: [(
                apxm_core::types::HandlerLanguage::TypeScript,
                crate::PackageHandlerWorkerCommand {
                    interpreter: "node".to_string(),
                    entry: PathBuf::from("tool-worker.mjs"),
                },
            )]
            .into_iter()
            .collect(),
            manifest: HandlerManifest::new(descriptors),
        }
    }

    #[test]
    fn a_package_capability_is_admitted_on_the_same_read_only_terms_as_a_builtin() {
        let handlers = supplied(vec![
            descriptor("proposal", Some(true), Some(false)),
            descriptor("apply", Some(false), Some(false)),
        ]);
        let port = LocalCapabilityPort::with_package_root(Some(&handlers), None)
            .expect("local capability port");

        assert!(port.registered_names().contains("proposal"));
        assert!(
            port.registered_names().contains("apply"),
            "a package Capability is registered so its denial is a decision, not a miss"
        );
        assert!(port.admitted_names().contains("proposal"));
        assert!(
            !port.admitted_names().contains("apply"),
            "a package handler that does not declare itself read-only is refused by the same \
             rule that refuses the write builtin"
        );
    }

    /// A Skill the package ships has to be readable by the program that
    /// declared it. Before the package root was published, `Skill(...)`
    /// compiled, passed the folder contract, rode the integrity chain, and
    /// then failed at `read_skill` because no configured root ever named
    /// the directory inside the package being executed.
    #[tokio::test]
    async fn a_skill_the_package_ships_resolves_through_the_package_root() {
        let package = tempfile::tempdir().expect("temp package root");
        let skill_dir = package.path().join("skills").join("review");
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: review\ndescription: How to review a change.\n---\n\n# Review\n\nRead the diff first.\n",
        )
        .expect("write SKILL.md");

        let port = LocalCapabilityPort::with_package_root(None, Some(package.path()))
            .expect("local capability port");

        let outcome = port
            .invoke(request(
                "read_skill",
                serde_json::json!({ "skill_id": "review" }),
            ))
            .await;
        let CapabilityOutcome::Completed { result } = outcome else {
            panic!("a package-shipped skill must be readable: {outcome:?}");
        };
        let rendered = serde_json::to_string(&result).expect("serialize skill body");
        assert!(
            rendered.contains("Read the diff first."),
            "read_skill must serve the package's own instructions: {rendered}"
        );
    }

    /// Without a package the root is absent, so the same id resolves to
    /// nothing rather than to whatever happens to sit near the process.
    #[tokio::test]
    async fn a_package_skill_is_not_reachable_without_the_package_root() {
        let port =
            LocalCapabilityPort::with_package_root(None, None).expect("local capability port");

        let outcome = port
            .invoke(request(
                "read_skill",
                serde_json::json!({ "skill_id": "a-skill-no-configured-root-publishes" }),
            ))
            .await;
        let CapabilityOutcome::Failed { message } = outcome else {
            panic!("an unpublished skill must fail: {outcome:?}");
        };
        assert!(
            message.contains("discovery roots"),
            "the refusal names the root set that was searched: {message}"
        );
    }

    #[tokio::test]
    async fn a_package_capability_the_root_does_not_admit_never_reaches_its_worker() {
        let handlers = supplied(vec![descriptor("apply", Some(false), Some(false))]);
        let port = LocalCapabilityPort::with_package_root(Some(&handlers), None)
            .expect("local capability port");

        let outcome = port.invoke(request("apply", serde_json::json!({}))).await;
        let CapabilityOutcome::Failed { message } = outcome else {
            panic!("an unadmitted package Capability must fail: {outcome:?}");
        };
        assert!(
            message.contains("not admitted by canonical local execution"),
            "the denial is the local admission policy, not a worker error: {message}"
        );
    }

    #[tokio::test]
    async fn a_package_capability_awaiting_approval_fails_before_its_worker() {
        // `requires_approval` absent is not an allow: a manifest that never
        // had a resolved decision carried into it is gated, so an unsynced
        // package cannot run unapproved.
        for undecided in [None, Some(true)] {
            let handlers = supplied(vec![descriptor("gated", Some(true), undecided)]);
            let port = LocalCapabilityPort::with_package_root(Some(&handlers), None)
                .expect("local capability port");
            assert!(
                port.admitted_names().contains("gated"),
                "the read-only surface still admits it; approval is the separate gate"
            );

            let outcome = port.invoke(request("gated", serde_json::json!({}))).await;
            let CapabilityOutcome::Failed { message } = outcome else {
                panic!(
                    "an approval-gated Capability with no consent context must fail closed: {outcome:?}"
                );
            };
            assert!(
                message.contains("requires approval"),
                "the refusal is the approval gate every builtin passes through: {message}"
            );
        }
    }

    #[test]
    fn a_package_handler_may_not_take_a_builtin_name() {
        let handlers = supplied(vec![descriptor("read", Some(true), Some(false))]);
        let Err(error) = LocalCapabilityPort::with_package_root(Some(&handlers), None) else {
            panic!("a package handler must not shadow a builtin");
        };
        assert!(
            error.to_string().contains("already registered"),
            "shadowing is refused rather than silently replacing the builtin: {error}"
        );
    }
}
