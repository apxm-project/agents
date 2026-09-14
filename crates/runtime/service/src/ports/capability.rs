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
//! 2. **Three outcomes from two.** [`apxm_capability::CapabilitySystem::invoke_request`]
//!    returns `Result<Value, RuntimeError>`. A timeout is the one error that
//!    leaves the effect *unobserved*, so it maps to
//!    [`CapabilityOutcome::OutcomeUnknown`]; collapsing it into `Failed` would
//!    assert the effect did not happen when the runtime does not know that.
//! 3. **What local execution may do at all.** The canonical composition root
//!    has no admitted sandbox backend and no Auth/Server-issued grants, so it
//!    admits only builtin capability surfaces with host-owned read-only
//!    metadata. Package metadata is descriptive and cannot grant read-only
//!    authority. Everything else is denied at the interceptor chokepoint,
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
    BashConfig, HttpConfig, InlineSkill, ReadConfig, SkillRootConfig, SkillsConfig, ToolsConfig,
    register_standard_tools, register_standard_tools_with_inline_skills, untrusted_content_value,
};
use apxm_capability::executor::CapabilityExecutor;
use apxm_capability::interceptor::{
    CapabilityInterceptor, InterceptDecision, PermissionInterceptor, requires_auth_names,
};
use apxm_capability::metadata::RuntimeCapability;
use apxm_capability_iface::sandbox::{
    ExecRequest, IsolationLevel, SandboxRegistry, ValidationResult, WrappedChild,
};
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
    // The local composition root must not turn an authored read into an
    // ambient host-file primitive. Restrict it to the caller's project root;
    // HTTP is disabled because SSRF filtering does not prevent exfiltration to
    // an arbitrary public endpoint.
    let project_root = package_root.map_or_else(
        || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        Path::to_path_buf,
    );
    ToolsConfig {
        bash: BashConfig {
            enabled: false,
            ..BashConfig::default()
        },
        read: ReadConfig {
            base_directory: Some(project_root.clone()),
            allowed_paths: Some(vec![project_root]),
            ..ReadConfig::default()
        },
        http: HttpConfig {
            enabled: false,
            ..HttpConfig::default()
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
/// invocation through the host's injected confinement backend, and nothing
/// outside this module can address it. No backend means no worker.
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
    package_root: Option<PathBuf>,
    sandbox_registry: Option<Arc<SandboxRegistry>>,
    process: tokio::sync::Mutex<Option<WorkerProcess>>,
    next_request: AtomicU64,
}

/// One live worker process and the two halves of its frame transport.
struct WorkerProcess {
    child: WrappedChild,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
}

const MAX_WORKER_FRAME_BYTES: usize = 8 * 1024 * 1024;
// Keep a worker's own I/O deadline below the CapabilitySystem's default
// invocation deadline. This ensures a cancelled outer invocation cannot leave
// a wedged worker cached for the next call.
const WORKER_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const WORKER_PASSTHROUGH_ENV: &[&str] = &["PATH", "LANG", "LC_ALL", "TERM"];

impl PackageHandlerWorker {
    /// Materialize the admitted manifest beside the selected worker entry.
    fn new(
        command: &crate::PackageHandlerWorkerCommand,
        manifest: &HandlerManifest,
        package_root: Option<&Path>,
        sandbox_registry: Option<Arc<SandboxRegistry>>,
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
            package_root: package_root.map(Path::to_path_buf),
            sandbox_registry,
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
        let mut line = serde_json::to_string(&frame)
            .map_err(|error| failure(format!("the invocation frame is not encodable: {error}")))?;
        line.push('\n');
        if line.len() > MAX_WORKER_FRAME_BYTES {
            return Err(failure(format!(
                "the worker request frame exceeds {MAX_WORKER_FRAME_BYTES} bytes"
            )));
        }

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
            match read_worker_line(&mut worker.stdout).await {
                Ok(Some(reply)) => Ok(reply),
                Ok(None) => Err("the worker closed its output before replying".to_string()),
                Err(error) => Err(format!("the worker reply could not be read: {error}")),
            }
        };
        let Ok(reply) = tokio::time::timeout(WORKER_IO_TIMEOUT, exchange).await else {
            if let Some(mut worker) = guard.take() {
                let _ = worker.child.start_kill();
            }
            return Err(failure(format!(
                "the package handler worker did not reply within {WORKER_IO_TIMEOUT:?}"
            )));
        };
        let reply = match reply {
            Ok(reply) => reply,
            Err(message) => {
                if let Some(mut worker) = guard.take() {
                    let _ = worker.child.start_kill();
                }
                // Only a host-authorized read-only handler is admitted by this
                // root (see `admitted_capability_names`), so a lost reply leaves
                // no external effect in doubt: reporting a definite failure
                // is accurate here rather than optimistic.
                return Err(failure(message));
            }
        };
        let reply: WorkerReply = match serde_json::from_str(&reply) {
            Ok(reply) => reply,
            Err(error) => {
                if let Some(mut worker) = guard.take() {
                    let _ = worker.child.start_kill();
                }
                return Err(failure(format!(
                    "the worker reply is not a result frame: {error}"
                )));
            }
        };
        if reply.req_id != request_id {
            if let Some(mut worker) = guard.take() {
                let _ = worker.child.start_kill();
            }
            return Err(failure(format!(
                "the worker replied to request '{}' while '{request_id}' was outstanding",
                reply.req_id
            )));
        }
        drop(guard);
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

    /// Start the selected worker through the host-supplied confinement port.
    fn spawn(&self) -> Result<WorkerProcess, String> {
        let package_root = self.package_root.as_deref().ok_or_else(|| {
            "package handler execution requires an explicit package root for filesystem policy"
                .to_string()
        })?;
        let registry = self.sandbox_registry.as_ref().ok_or_else(|| {
            "package handler execution requires a trusted sandbox registry".to_string()
        })?;
        if registry.is_empty() {
            return Err(
                "package handler execution requires an available sandbox backend".to_string(),
            );
        }

        let mut env = HashMap::new();
        for key in WORKER_PASSTHROUGH_ENV {
            if let Ok(value) = std::env::var(key) {
                env.insert((*key).to_string(), value);
            }
        }
        let request = ExecRequest {
            min_isolation: IsolationLevel::OsLevel,
            program: self.command.interpreter.clone(),
            args: vec![
                self.command.entry.to_string_lossy().into_owned(),
                self.manifest_file.to_string_lossy().into_owned(),
            ],
            working_dir: Some(package_root.to_path_buf()),
            env,
            stdin_data: None,
            timeout: WORKER_IO_TIMEOUT,
            max_output_bytes: MAX_WORKER_FRAME_BYTES,
            read_paths: vec![
                package_root.to_path_buf(),
                self.command.entry.clone(),
                self.manifest_file.to_path_buf(),
            ],
            write_paths: Vec::new(),
            needs_network: false,
            needs_process_spawn: false,
            origin_op: Some("capability.invoke".to_string()),
            origin_node_id: None,
        };
        let selection = registry
            .select_for_request(&request)
            .map_err(|error| format!("package worker sandbox selection failed: {error}"))?;
        if !matches!(selection.validation, ValidationResult::Ok) {
            return Err(
                "package worker sandbox validation did not provide full confinement guarantees"
                    .to_string(),
            );
        }
        let capabilities = selection.backend.capabilities();
        if capabilities.isolation_level < IsolationLevel::OsLevel
            || !capabilities.supports_filesystem_restriction
            || !capabilities.supports_network_restriction
            || !capabilities.supports_process_restriction
            || !capabilities.supports_syscall_filtering
            || !capabilities.supports_resource_limits
        {
            return Err(format!(
                "sandbox backend '{}' cannot enforce the package worker filesystem, network, process, syscall, and resource policy",
                capabilities.name
            ));
        }
        let wrapped = selection
            .backend
            .wrap_command_for_request(&request)
            .map_err(|error| format!("package worker sandbox wrapping failed: {error}"))?;
        if !wrapped.is_confined() {
            return Err(
                "package worker sandbox returned an unconfined command wrapper".to_string(),
            );
        }
        let mut child = wrapped
            .spawn(|command| {
                command
                    .current_dir(package_root)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true);
            })
            .map_err(|error| {
                format!(
                    "the package handler worker {} could not be started with {}: {error}",
                    self.command.entry.display(),
                    self.command.interpreter
                )
            })?;
        let stdin = child.take_stdin().ok_or("the worker has no input stream")?;
        let stdout = child
            .take_stdout()
            .ok_or("the worker has no output stream")?;
        Ok(WorkerProcess {
            child,
            stdin,
            stdout: tokio::io::BufReader::new(stdout),
        })
    }
}

async fn read_worker_line(
    reader: &mut tokio::io::BufReader<tokio::process::ChildStdout>,
) -> Result<Option<String>, String> {
    use tokio::io::AsyncBufReadExt;

    let mut bytes = Vec::new();
    loop {
        let (take, newline) = {
            let available = reader.fill_buf().await.map_err(|error| error.to_string())?;
            if available.is_empty() {
                if bytes.is_empty() {
                    return Ok(None);
                }
                return String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|error| format!("worker frame is not UTF-8: {error}"));
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let take = newline.map_or(available.len(), |index| index + 1);
            if bytes.len().saturating_add(take) > MAX_WORKER_FRAME_BYTES {
                return Err(format!(
                    "worker frame exceeds {MAX_WORKER_FRAME_BYTES} bytes"
                ));
            }
            (take, newline.is_some())
        };
        let available = reader.fill_buf().await.map_err(|error| error.to_string())?;
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if newline {
            return String::from_utf8(bytes)
                .map(Some)
                .map_err(|error| format!("worker frame is not UTF-8: {error}"));
        }
    }
}

/// The worker's result frame.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// Read-only authority is supplied separately by the trusted composition
    /// root. The descriptor's `read_only` bit is descriptive only.
    /// `requires_approval` remains gated by the permission decision carried in
    /// the manifest; an absent decision is not an allow.
    fn new(
        descriptor: &HandlerDescriptor,
        worker: Arc<PackageHandlerWorker>,
        trusted_read_only: bool,
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
        if trusted_read_only {
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
        // Package handlers are untrusted extension code. Their result is
        // quoted as data with a stable handler-origin URI before it can enter
        // the Agent Program value stream; a handler cannot smuggle a role,
        // policy, or instruction field into the trusted result channel.
        let result = untrusted_content_value(
            &self.metadata.name,
            format!("extension://{}", self.handler_id),
            serde_json::to_string(&value).map_err(|error| RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!("the handler result could not be serialized: {error}"),
            })?,
        )?;
        Ok(result)
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

/// Register every Capability the supplied package ships.
///
/// Registration happens before the admission policy is captured. A package
/// Capability enters the admitted set only when the composition root supplies
/// a host-issued read-only decision for its name; the package manifest's
/// `read_only` field is never used as authority.
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
    package_root: Option<&Path>,
    sandbox_registry: Option<Arc<SandboxRegistry>>,
) -> Result<(), RuntimeError> {
    validate_package_handler_binding(handlers)?;
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
            Arc::new(PackageHandlerWorker::new(
                command,
                &evaluable,
                package_root,
                sandbox_registry.clone(),
            )?),
        );
    }
    for descriptor in &handlers.manifest.handlers {
        let worker = workers
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
            handlers.trusted_read_only.contains(&descriptor.name),
        )?))?;
    }
    Ok(())
}

/// Validate the complete host-supplied package-handler binding before
/// registering even one descriptor. The manifest is untrusted input and the
/// worker map is host-selected implementation data; accepting either one
/// partially would let a malformed package become visible as a capability or
/// make the worker policy depend on map iteration order.
fn validate_package_handler_binding(
    handlers: &crate::AdmittedPackageHandlers,
) -> Result<(), RuntimeError> {
    handlers
        .manifest
        .validate()
        .map_err(|error| RuntimeError::Capability {
            capability: "package.handlers".to_owned(),
            message: format!("the package handler manifest is invalid: {error}"),
        })?;

    let manifest_languages: BTreeSet<_> = handlers
        .manifest
        .handlers
        .iter()
        .map(|descriptor| descriptor.language)
        .collect();
    let worker_languages: BTreeSet<_> = handlers.workers.keys().copied().collect();
    if manifest_languages != worker_languages {
        return Err(RuntimeError::Capability {
            capability: "package.handlers".to_owned(),
            message: format!(
                "package handler workers must exactly cover manifest languages (manifest: {:?}, workers: {:?})",
                manifest_languages, worker_languages
            ),
        });
    }

    let manifest_names: BTreeSet<_> = handlers
        .manifest
        .handlers
        .iter()
        .map(|descriptor| descriptor.name.as_str())
        .collect();
    if let Some(name) = handlers
        .trusted_read_only
        .iter()
        .find(|name| !manifest_names.contains(name.as_str()))
    {
        return Err(RuntimeError::Capability {
            capability: name.clone(),
            message: "host-issued read-only decision names no package handler".to_owned(),
        });
    }

    for (language, command) in &handlers.workers {
        if command.interpreter.trim().is_empty() {
            return Err(RuntimeError::Capability {
                capability: format!("package.handlers.{language:?}"),
                message: "package handler worker interpreter must not be empty".to_owned(),
            });
        }
        if command.entry.as_os_str().is_empty()
            || command
                .entry
                .components()
                .any(|component| component == std::path::Component::ParentDir)
        {
            return Err(RuntimeError::Capability {
                capability: format!("package.handlers.{language:?}"),
                message: format!(
                    "package handler worker entry '{}' is not a safe path",
                    command.entry.display()
                ),
            });
        }
    }
    Ok(())
}

/// The locally admitted capability names: builtin read-only metadata plus
/// package metadata marked by a host-issued read-only decision.
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
    /// ships, then gate them behind host-owned read-only admission policy.
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
    #[allow(dead_code)]
    pub fn with_package_root(
        handlers: Option<&crate::AdmittedPackageHandlers>,
        package_root: Option<&Path>,
    ) -> Result<Self, RuntimeError> {
        Self::with_package_root_and_sandbox_and_inline_skills(
            handlers,
            package_root,
            None,
            Vec::new(),
        )
    }

    /// Build the local capability surface with inline Skills carried by the
    /// admitted Agent Program artifact.
    pub fn with_package_root_and_inline_skills(
        handlers: Option<&crate::AdmittedPackageHandlers>,
        package_root: Option<&Path>,
        inline_skills: Vec<InlineSkill>,
    ) -> Result<Self, RuntimeError> {
        Self::with_package_root_and_sandbox_and_inline_skills(
            handlers,
            package_root,
            None,
            inline_skills,
        )
    }

    /// Build the local capability surface with an explicitly injected sandbox
    /// registry for package workers. The registry is host authority; package
    /// metadata cannot supply or select it.
    #[allow(dead_code)]
    pub fn with_package_root_and_sandbox(
        handlers: Option<&crate::AdmittedPackageHandlers>,
        package_root: Option<&Path>,
        sandbox_registry: Option<Arc<SandboxRegistry>>,
    ) -> Result<Self, RuntimeError> {
        Self::with_package_root_and_sandbox_and_inline_skills(
            handlers,
            package_root,
            sandbox_registry,
            Vec::new(),
        )
    }

    /// Build the local capability surface with inline Skills and an optional
    /// host-owned sandbox registry.
    pub fn with_package_root_and_sandbox_and_inline_skills(
        handlers: Option<&crate::AdmittedPackageHandlers>,
        package_root: Option<&Path>,
        sandbox_registry: Option<Arc<SandboxRegistry>>,
        inline_skills: Vec<InlineSkill>,
    ) -> Result<Self, RuntimeError> {
        let system = CapabilitySystem::new();
        let tools = local_tools_config(package_root);
        if inline_skills.is_empty() {
            register_standard_tools(&system, &tools)?;
        } else {
            register_standard_tools_with_inline_skills(&system, &tools, inline_skills)?;
        }
        if let Some(handlers) = handlers {
            register_package_handlers(&system, handlers, package_root, sandbox_registry)?;
        }
        // Install auth policy after the complete registry is assembled. A
        // requires_auth capability must never become callable merely because
        // its registration happened after policy construction.
        let requires_auth = requires_auth_names(&system.list_capabilities());
        system.register_interceptor(Arc::new(PermissionInterceptor::new(requires_auth, true)));
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

impl LocalCapabilityPort {
    /// Execute the immutable canonical request without projecting away its
    /// authority. `CapabilitySystem::invoke_request` validates the request's
    /// authority and digest binding before converting arguments for the
    /// implementation.
    async fn invoke_canonical(&self, request: CapabilityRequest) -> CapabilityOutcome {
        let capability_ref = request.capability_ref().to_string();
        match self.system.invoke_request(request).await {
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

#[async_trait]
impl CapabilityPort for LocalCapabilityPort {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityOutcome {
        self.invoke_canonical(request).await
    }

    async fn invoke_authorized(&self, request: CapabilityRequest) -> CapabilityOutcome {
        self.invoke_canonical(request).await
    }
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
    use sha2::{Digest, Sha256};

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
            !port.admitted_names().contains("http_get"),
            "canonical local execution must not expose arbitrary outbound HTTP"
        );
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

        let port = LocalCapabilityPort::with_package_root(None, Some(directory.path()))
            .expect("local capability port");
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
        let module = format!("capabilities/{name}/handler");
        let qualname = name.to_string();
        let handler_id = format!(
            "sha256:{:x}",
            Sha256::digest(format!("{module}:{qualname}"))
        );
        HandlerDescriptor {
            kind: apxm_core::types::HandlerKind::Tool,
            language: apxm_core::types::HandlerLanguage::TypeScript,
            handler_id,
            module,
            qualname,
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
            trusted_read_only: BTreeSet::new(),
        }
    }

    #[test]
    fn malformed_package_manifest_fails_before_registration() {
        let mut handlers = supplied(vec![descriptor("proposal", Some(true), Some(false))]);
        handlers.manifest.version = "unknown.handler-manifest".to_owned();
        let error = match LocalCapabilityPort::with_package_root(Some(&handlers), None) {
            Ok(_) => panic!("an unsupported package manifest must not be registered"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("handler manifest is invalid"),
            "the fail-closed error identifies manifest validation: {error}"
        );
    }

    #[test]
    fn unknown_host_read_only_decisions_fail_before_registration() {
        let mut handlers = supplied(vec![descriptor("proposal", Some(true), Some(false))]);
        handlers.trusted_read_only.insert("not-shipped".to_owned());
        let error = match LocalCapabilityPort::with_package_root(Some(&handlers), None) {
            Ok(_) => panic!("a decision for an unknown package handler must not be registered"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("names no package handler"),
            "the fail-closed error identifies the unknown decision: {error}"
        );
    }

    #[test]
    fn worker_parent_traversal_fails_before_registration() {
        let mut handlers = supplied(vec![descriptor("proposal", Some(true), Some(false))]);
        handlers
            .workers
            .get_mut(&apxm_core::types::HandlerLanguage::TypeScript)
            .expect("the fixture supplies a TypeScript worker")
            .entry = PathBuf::from("../worker.mjs");
        let error = match LocalCapabilityPort::with_package_root(Some(&handlers), None) {
            Ok(_) => panic!("a worker outside its package root must not be registered"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("not a safe path"),
            "the fail-closed error identifies worker path traversal: {error}"
        );
    }

    #[test]
    fn package_metadata_cannot_self_authorize_read_only_execution() {
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
        assert!(
            !port.admitted_names().contains("proposal"),
            "the package manifest's read_only bit is not trusted authority"
        );
        assert!(
            !port.admitted_names().contains("apply"),
            "a package handler without a host-issued read-only decision is refused"
        );
    }

    #[tokio::test]
    async fn a_trusted_read_only_package_still_requires_confinement() {
        let mut handlers = supplied(vec![descriptor("proposal", Some(true), Some(false))]);
        handlers.trusted_read_only.insert("proposal".to_string());
        let package = tempfile::tempdir().expect("package root");
        let port = LocalCapabilityPort::with_package_root(Some(&handlers), Some(package.path()))
            .expect("local capability port");

        let outcome = port
            .invoke(request("proposal", serde_json::json!({})))
            .await;
        let CapabilityOutcome::Failed { message } = outcome else {
            panic!("a package worker without a sandbox must fail: {outcome:?}");
        };
        assert!(
            message.contains("trusted sandbox registry"),
            "the failure names the missing confinement boundary: {message}"
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
            let mut handlers = supplied(vec![descriptor("gated", Some(true), undecided)]);
            handlers.trusted_read_only.insert("gated".to_string());
            let package = tempfile::tempdir().expect("package root");
            let port =
                LocalCapabilityPort::with_package_root(Some(&handlers), Some(package.path()))
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

    #[test]
    fn package_workers_do_not_inherit_authority_bearing_environment() {
        for forbidden in [
            "APXM_AUTH_BEARER",
            "TAVILY_API_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "DATABASE_URL",
            "LD_LIBRARY_PATH",
            "HOME",
        ] {
            assert!(
                !WORKER_PASSTHROUGH_ENV.contains(&forbidden),
                "worker environment must not pass through {forbidden}"
            );
        }
    }
}
