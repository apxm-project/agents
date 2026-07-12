use std::ffi::OsString;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Component, Path, PathBuf};
use std::process::{Command as StdCommand, Stdio as StdStdio};
use std::sync::OnceLock;
use std::time::Instant;

use apxm_runtime::sandbox::constants::{
    backend_names, bubblewrap, env as sandbox_env, executables, session_prefixes, shell_args,
    systemctl, systemd_run,
};
use apxm_runtime::sandbox::policy::SandboxPolicy;
use apxm_runtime::sandbox::{
    ExecRequest, ExecResult, IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxContext,
    SandboxError, ValidationResult, WrappedCommand,
};
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub struct BubblewrapSandboxBackend {
    policy: SandboxPolicy,
}

pub struct SystemdSandboxBackend {
    policy: SandboxPolicy,
}

impl SystemdSandboxBackend {
    pub fn new(policy: SandboxPolicy) -> Self {
        Self { policy }
    }

    pub fn with_default_policy() -> Self {
        Self::new(SandboxPolicy::default())
    }
}

impl BubblewrapSandboxBackend {
    pub fn new(policy: SandboxPolicy) -> Self {
        Self { policy }
    }

    pub fn with_default_policy() -> Self {
        Self::new(SandboxPolicy::default())
    }
}

struct BubblewrapSession {
    root: tempfile::TempDir,
    fallback_workdir: PathBuf,
}

struct WorkingDirectory {
    host: PathBuf,
    sandbox: PathBuf,
}

struct WritableMount {
    host: PathBuf,
    sandbox: PathBuf,
}

struct SystemdUnitGuard {
    unit: String,
    systemctl: String,
}

impl SystemdUnitGuard {
    fn new(unit: String) -> Self {
        Self {
            unit,
            systemctl: executables::SYSTEMCTL.to_string(),
        }
    }

    #[cfg(test)]
    fn with_systemctl(unit: String, systemctl: String) -> Self {
        Self { unit, systemctl }
    }
}

impl Drop for SystemdUnitGuard {
    fn drop(&mut self) {
        let mut command = StdCommand::new(&self.systemctl);
        command
            .args([systemctl::FLAG_USER, systemctl::STOP, self.unit.as_str()])
            .stdout(StdStdio::null())
            .stderr(StdStdio::null())
            .env_clear();
        apply_systemd_launcher_environment_std(&mut command);
        let _ = command.status();
    }
}

#[async_trait]
impl SandboxBackend for BubblewrapSandboxBackend {
    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            isolation_level: IsolationLevel::Container,
            supports_filesystem_restriction: false,
            supports_network_restriction: true,
            supports_syscall_filtering: false,
            supports_resource_limits: false,
            name: backend_names::BUBBLEWRAP.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && bubblewrap_available()
    }

    fn validate(&self, _request: &ExecRequest) -> ValidationResult {
        if !cfg!(target_os = "linux") {
            return ValidationResult::Unsupported {
                reason: bubblewrap::ERR_ONLY_LINUX.to_string(),
            };
        }
        if !bubblewrap_available() {
            return ValidationResult::Unsupported {
                reason: bubblewrap::ERR_NOT_AVAILABLE.to_string(),
            };
        }

        // Read grants are informational, while writes are confined to explicit
        // mounts. The backend does not advertise a readable-path allowlist.
        ValidationResult::Ok
    }

    async fn create_session(&self) -> Result<SandboxContext, SandboxError> {
        let root =
            tempfile::tempdir().map_err(|error| SandboxError::SessionError(error.to_string()))?;
        let fallback_workdir = root.path().join(session_prefixes::WORKDIR);

        std::fs::create_dir_all(&fallback_workdir)
            .map_err(|error| SandboxError::SessionError(error.to_string()))?;

        Ok(SandboxContext::new(
            session_id(session_prefixes::BUBBLEWRAP),
            backend_names::BUBBLEWRAP,
            IsolationLevel::Container,
            BubblewrapSession {
                root,
                fallback_workdir,
            },
        ))
    }

    async fn execute(
        &self,
        ctx: &SandboxContext,
        request: ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        let session = ctx
            .downcast_ref::<BubblewrapSession>()
            .ok_or_else(|| SandboxError::SessionError(bubblewrap::ERR_SESSION_STATE.to_string()))?;

        let working_directory = resolve_working_directory(request.working_dir.as_ref(), session)?;
        let writable_mounts = collect_writable_mounts(&request, &working_directory)?;
        let command_args =
            build_bwrap_command_args(&request, &working_directory, &writable_mounts)?;

        let mut command = Command::new(executables::BUBBLEWRAP);
        command
            .args(&command_args)
            .current_dir(session.root.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .env_clear();

        if request.stdin_data.is_some() {
            command.stdin(std::process::Stdio::piped());
        } else {
            command.stdin(std::process::Stdio::null());
        }

        for key in sandbox_env::CHILD_PASSTHROUGH {
            if let Ok(value) = std::env::var(key) {
                command.env(key, value);
            }
        }
        command.env(sandbox_env::TMPDIR, bubblewrap::FILESYSTEM_TMP);
        command.env(sandbox_env::TEMP, bubblewrap::FILESYSTEM_TMP);
        command.env(sandbox_env::TMP, bubblewrap::FILESYSTEM_TMP);
        for (key, value) in &request.env {
            if !self.policy.blocks_env_var(key) {
                command.env(key, value);
            }
        }

        let start = Instant::now();
        let mut child = command.spawn().map_err(map_bwrap_spawn_error)?;

        if let Some(stdin_data) = &request.stdin_data
            && let Some(mut stdin) = child.stdin.take()
        {
            stdin
                .write_all(stdin_data.as_bytes())
                .await
                .map_err(|error| {
                    SandboxError::ExecutionFailed(format!(
                        "{}: {error}",
                        bubblewrap::ERR_EXECUTION_PREFIX
                    ))
                })?;
        }

        let output = match tokio::time::timeout(request.timeout, child.wait_with_output()).await {
            Ok(result) => result.map_err(|error| {
                SandboxError::ExecutionFailed(format!(
                    "{}: {error}",
                    bubblewrap::ERR_EXECUTION_PREFIX
                ))
            })?,
            Err(_) => {
                return Ok(ExecResult {
                    success: false,
                    exit_code: None,
                    stdout: String::new(),
                    stderr: bubblewrap::ERR_TIMED_OUT.to_string(),
                    duration: start.elapsed(),
                    timed_out: true,
                });
            }
        };

        let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        truncate_output(&mut stdout, &mut stderr, request.max_output_bytes);

        Ok(ExecResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout,
            stderr,
            duration: start.elapsed(),
            timed_out: false,
        })
    }

    async fn destroy_session(&self, _ctx: SandboxContext) -> Result<(), SandboxError> {
        Ok(())
    }

    fn wrap_command(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        needs_network: bool,
        env: &[(String, String)],
    ) -> Result<WrappedCommand, SandboxError> {
        validate_child_environment(env)?;
        let cwd_mount = writable_cwd_mount(cwd)?;
        let bwrap_args = build_bwrap_wrap_args(program, args, &cwd_mount, needs_network, env);
        WrappedCommand::direct(executables::BUBBLEWRAP, bwrap_args, env.to_vec())
    }
}

#[async_trait]
impl SandboxBackend for SystemdSandboxBackend {
    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            isolation_level: IsolationLevel::OsLevel,
            supports_filesystem_restriction: false,
            supports_network_restriction: true,
            supports_syscall_filtering: true,
            supports_resource_limits: false,
            name: backend_names::SYSTEMD.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn is_available(&self) -> bool {
        cfg!(target_os = "linux") && systemd_run_available()
    }

    fn validate(&self, request: &ExecRequest) -> ValidationResult {
        if !cfg!(target_os = "linux") {
            return ValidationResult::Unsupported {
                reason: systemd_run::ERR_ONLY_LINUX.to_string(),
            };
        }
        if !systemd_run_available() {
            return ValidationResult::Unsupported {
                reason: systemd_run::ERR_NOT_AVAILABLE.to_string(),
            };
        }
        if request.needs_network {
            return ValidationResult::Unsupported {
                reason: "systemd user-service sandbox is reserved for no-network workers"
                    .to_string(),
            };
        }
        ValidationResult::Degraded {
            warnings: vec![
                "systemd user-service sandbox enforces syscall and network isolation but cannot restrict filesystem paths on this host"
                    .to_string(),
            ],
        }
    }

    async fn create_session(&self) -> Result<SandboxContext, SandboxError> {
        Ok(SandboxContext::new(
            session_id(session_prefixes::SYSTEMD),
            backend_names::SYSTEMD,
            IsolationLevel::OsLevel,
            (),
        ))
    }

    async fn execute(
        &self,
        ctx: &SandboxContext,
        request: ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        ctx.downcast_ref::<()>().ok_or_else(|| {
            SandboxError::SessionError(systemd_run::ERR_SESSION_STATE.to_string())
        })?;
        if request.needs_network {
            return Err(SandboxError::RequirementsNotMet(
                "systemd user-service sandbox does not admit networked commands".to_string(),
            ));
        }

        let cwd = request
            .working_dir
            .clone()
            .map_or_else(std::env::current_dir, Ok)
            .map_err(|error| SandboxError::ExecutionFailed(error.to_string()))?;
        let env = systemd_environment(&self.policy, &request.env);
        validate_child_environment(&env)?;
        let unit = systemd_unit_name();
        let _unit_guard = SystemdUnitGuard::new(unit.clone());
        let command_args = build_systemd_run_args(
            &request.program,
            &request.args,
            &cwd,
            request.needs_network,
            &env,
            Some(&unit),
        );

        let mut command = Command::new(executables::SYSTEMD_RUN);
        command
            .args(&command_args)
            .stdin(if request.stdin_data.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .env_clear();
        apply_outer_systemd_environment(&mut command);

        let start = Instant::now();
        let mut child = command.spawn().map_err(map_systemd_spawn_error)?;
        if let Some(stdin_data) = &request.stdin_data
            && let Some(mut stdin) = child.stdin.take()
        {
            stdin
                .write_all(stdin_data.as_bytes())
                .await
                .map_err(|error| SandboxError::ExecutionFailed(error.to_string()))?;
        }

        let output = match tokio::time::timeout(request.timeout, child.wait_with_output()).await {
            Ok(result) => result.map_err(|error| {
                SandboxError::ExecutionFailed(format!(
                    "{}: {error}",
                    systemd_run::ERR_EXECUTION_PREFIX
                ))
            })?,
            Err(_) => {
                return Ok(ExecResult {
                    success: false,
                    exit_code: None,
                    stdout: String::new(),
                    stderr: systemd_run::ERR_TIMED_OUT.to_string(),
                    duration: start.elapsed(),
                    timed_out: true,
                });
            }
        };

        let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        truncate_output(&mut stdout, &mut stderr, request.max_output_bytes);
        Ok(ExecResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout,
            stderr,
            duration: start.elapsed(),
            timed_out: false,
        })
    }

    async fn destroy_session(&self, _ctx: SandboxContext) -> Result<(), SandboxError> {
        Ok(())
    }

    fn wrap_command(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        needs_network: bool,
        env: &[(String, String)],
    ) -> Result<WrappedCommand, SandboxError> {
        if needs_network {
            return Err(SandboxError::RequirementsNotMet(
                "systemd user-service sandbox does not admit networked commands".to_string(),
            ));
        }
        validate_child_environment(env)?;
        let unit = systemd_unit_name();
        let args = build_systemd_run_args(program, args, cwd, false, env, Some(&unit));
        WrappedCommand::guarded(
            executables::SYSTEMD_RUN,
            args,
            systemd_launcher_environment(),
            SystemdUnitGuard::new(unit),
        )
    }
}

/// Build the `bwrap` argv that wraps a long-running, stdio-attached child
/// (ACP coding agent, interactive terminal) under bubblewrap confinement.
///
/// Unlike [`build_bwrap_command_args`] this is session-free: the working
/// directory is bound writable in place, `/tmp` is an ephemeral tmpfs, the
/// rest of the root is read-only, and the network namespace is kept only when
/// the child needs it (coding agents reach the model gateway, so callers pass
/// `needs_network = true`). bubblewrap forwards stdin/stdout/stderr to the
/// inner process, so the caller's pipe wiring is unchanged.
fn build_bwrap_wrap_args(
    program: &str,
    args: &[String],
    cwd: &WritableMount,
    needs_network: bool,
    env: &[(String, String)],
) -> Vec<String> {
    let host_cwd = path_string(&cwd.host);
    let sandbox_cwd = path_string(&cwd.sandbox);
    let mut wrapped = bwrap_isolation_preamble(needs_network);
    wrapped.push(bubblewrap::FLAG_BIND.to_string());
    wrapped.push(host_cwd);
    wrapped.push(sandbox_cwd.clone());
    wrapped.push(bubblewrap::FLAG_CHDIR.to_string());
    wrapped.push(sandbox_cwd);
    wrapped.push(bubblewrap::FLAG_SEPARATOR.to_string());
    append_child_command(&mut wrapped, program, args, env);

    wrapped
}

/// Shared bubblewrap isolation flags for one-shot and long-running commands.
///
/// The host root remains readable but not writable, `/tmp` and `/run/user` are
/// replaced, and network isolation is enabled for no-network requests.
fn bwrap_isolation_preamble(needs_network: bool) -> Vec<String> {
    let mut args = vec![
        bubblewrap::FLAG_NEW_SESSION.to_string(),
        bubblewrap::FLAG_DIE_WITH_PARENT.to_string(),
        bubblewrap::FLAG_RO_BIND.to_string(),
        bubblewrap::FILESYSTEM_ROOT.to_string(),
        bubblewrap::FILESYSTEM_ROOT.to_string(),
        bubblewrap::FLAG_DEV.to_string(),
        bubblewrap::FILESYSTEM_DEV.to_string(),
        bubblewrap::FLAG_PROC.to_string(),
        bubblewrap::FILESYSTEM_PROC.to_string(),
        bubblewrap::FLAG_TMPFS.to_string(),
        bubblewrap::FILESYSTEM_TMP.to_string(),
        bubblewrap::FLAG_TMPFS.to_string(),
        bubblewrap::FILESYSTEM_RUN_USER.to_string(),
        bubblewrap::FLAG_UNSHARE_USER.to_string(),
        bubblewrap::FLAG_UNSHARE_PID.to_string(),
    ];
    if !needs_network {
        args.push(bubblewrap::FLAG_UNSHARE_NET.to_string());
    }
    args
}

fn build_systemd_run_args(
    program: &str,
    args: &[String],
    cwd: &Path,
    needs_network: bool,
    env: &[(String, String)],
    unit: Option<&str>,
) -> Vec<String> {
    let mut wrapped = vec![
        systemd_run::FLAG_USER.to_string(),
        systemd_run::FLAG_WAIT.to_string(),
        systemd_run::FLAG_PIPE.to_string(),
        systemd_run::FLAG_QUIET.to_string(),
        systemd_run::FLAG_COLLECT.to_string(),
    ];
    if let Some(unit) = unit {
        wrapped.push(format!("{}={unit}", systemd_run::FLAG_UNIT));
    }
    for property in [
        systemd_run::PROPERTY_NO_NEW_PRIVILEGES,
        systemd_run::PROPERTY_RESTRICT_SUID_SGID,
        systemd_run::PROPERTY_LOCK_PERSONALITY,
        systemd_run::PROPERTY_REMOVE_IPC,
        systemd_run::PROPERTY_UMASK,
        systemd_run::PROPERTY_KILL_MODE,
        systemd_run::PROPERTY_TIMEOUT_STOP,
        systemd_run::PROPERTY_SYSTEM_CALL_ARCHITECTURES,
        systemd_run::PROPERTY_SYSTEM_CALL_ERROR_NUMBER,
    ] {
        wrapped.push(systemd_run::FLAG_PROPERTY.to_string());
        wrapped.push(property.to_string());
    }
    wrapped.push(systemd_run::FLAG_PROPERTY.to_string());
    wrapped.push(
        if needs_network {
            systemd_run::PROPERTY_SYSTEM_CALL_FILTER
        } else {
            systemd_run::PROPERTY_SYSTEM_CALL_FILTER_NO_NETWORK
        }
        .to_string(),
    );
    wrapped.push(format!(
        "{}={}",
        systemd_run::FLAG_WORKING_DIRECTORY,
        path_string(cwd)
    ));
    append_child_command(&mut wrapped, program, args, env);
    wrapped
}

fn append_child_command(
    wrapped: &mut Vec<String>,
    program: &str,
    args: &[String],
    env: &[(String, String)],
) {
    wrapped.push(executables::ENV.to_string());
    wrapped.push(systemd_run::ENV_CLEAR.to_string());
    wrapped.push(systemd_run::ENV_OPTION_TERMINATOR.to_string());
    wrapped.extend(env.iter().map(|(key, value)| format!("{key}={value}")));
    wrapped.push(program.to_string());
    wrapped.extend(args.iter().cloned());
}

fn systemd_environment(
    policy: &SandboxPolicy,
    request_env: &std::collections::HashMap<String, String>,
) -> Vec<(String, String)> {
    sandbox_env::child_environment(
        request_env
            .iter()
            .filter(|(key, _)| !policy.blocks_env_var(key))
            .map(|(key, value)| (key, value)),
    )
}

fn apply_outer_systemd_environment(command: &mut Command) {
    for (key, value) in systemd_launcher_environment() {
        command.env(key, value);
    }
}

fn bubblewrap_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| probe_bubblewrap(executables::BUBBLEWRAP))
}

fn systemd_run_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| probe_systemd_run(executables::SYSTEMD_RUN))
}

fn probe_bubblewrap(executable: &str) -> bool {
    let args = bwrap_isolation_preamble(false);
    StdCommand::new(executable)
        .args(args)
        .arg(bubblewrap::FLAG_SEPARATOR)
        .arg(executables::TRUE)
        .stdout(StdStdio::null())
        .stderr(StdStdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn probe_systemd_run(executable: &str) -> bool {
    let env =
        sandbox_env::child_environment([(sandbox_env::LANG, "C"), (sandbox_env::LC_ALL, "C")]);
    if sandbox_env::validate_child_environment(&env).is_err() {
        return false;
    }
    let sanity_args = vec![
        shell_args::COMMAND.to_string(),
        format!("printf {}", systemd_run::PROBE_RESTRICTED_OK),
    ];
    let restricted_sanity = build_systemd_run_args(
        executables::BASH,
        &sanity_args,
        Path::new("/"),
        false,
        &env,
        None,
    );
    if !run_systemd_probe(executable, &restricted_sanity)
        .is_some_and(|output| probe_output_matches(&output, systemd_run::PROBE_RESTRICTED_OK))
    {
        return false;
    }
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else {
        return false;
    };
    let Ok(address) = listener.local_addr() else {
        return false;
    };
    let socket_probe = format!(
        "exec 3<>/dev/tcp/127.0.0.1/{} && printf {}",
        address.port(),
        systemd_run::PROBE_NETWORK_OK
    );
    let network_args = vec![shell_args::COMMAND.to_string(), socket_probe];
    let unrestricted_probe = build_systemd_run_args(
        executables::BASH,
        &network_args,
        Path::new("/"),
        true,
        &env,
        None,
    );
    if !run_systemd_probe(executable, &unrestricted_probe)
        .is_some_and(|output| probe_output_matches(&output, systemd_run::PROBE_NETWORK_OK))
    {
        return false;
    }
    let restricted_probe = build_systemd_run_args(
        executables::BASH,
        &network_args,
        Path::new("/"),
        false,
        &env,
        None,
    );
    run_systemd_probe(executable, &restricted_probe).is_some_and(|output| {
        !output.status.success()
            && String::from_utf8_lossy(&output.stderr).contains(systemd_run::PROBE_NETWORK_DENIED)
    })
}

fn probe_output_matches(output: &std::process::Output, marker: &str) -> bool {
    output.status.success() && String::from_utf8_lossy(&output.stdout) == marker
}

fn run_systemd_probe(executable: &str, args: &[String]) -> Option<std::process::Output> {
    let mut command = StdCommand::new(executable);
    command
        .args(args)
        .stdout(StdStdio::piped())
        .stderr(StdStdio::piped())
        .env_clear();
    apply_systemd_launcher_environment_std(&mut command);
    command.output().ok()
}

fn apply_systemd_launcher_environment_std(command: &mut StdCommand) {
    for (key, value) in systemd_launcher_environment() {
        command.env(key, value);
    }
}

fn systemd_launcher_environment() -> Vec<(String, String)> {
    let mut env = Vec::new();
    for key in sandbox_env::SYSTEMD_LAUNCHER_PASSTHROUGH {
        if let Ok(value) = std::env::var(key) {
            env.push(((*key).to_string(), value));
        }
    }
    env
}

fn validate_child_environment(env: &[(String, String)]) -> Result<(), SandboxError> {
    sandbox_env::validate_child_environment(env)
        .map_err(|error| SandboxError::ValidationFailed(error.to_string()))
}

fn systemd_unit_name() -> String {
    format!("apxm-sandbox-{}.service", monotonic_nanos())
}

fn resolve_working_directory(
    requested_working_dir: Option<&PathBuf>,
    session: &BubblewrapSession,
) -> Result<WorkingDirectory, SandboxError> {
    match requested_working_dir {
        Some(path) => {
            let host = projected_canonical_path(path)?;
            validate_bwrap_writable_mount(&host)?;
            std::fs::create_dir_all(&host)
                .map_err(|error| SandboxError::ExecutionFailed(error.to_string()))?;
            if !host.is_dir() {
                return Err(SandboxError::ExecutionFailed(
                    bubblewrap::ERR_WORKDIR_NOT_DIRECTORY.to_string(),
                ));
            }
            Ok(WorkingDirectory {
                host: std::fs::canonicalize(&host)
                    .map_err(|error| SandboxError::ExecutionFailed(error.to_string()))?,
                sandbox: host,
            })
        }
        None => Ok(WorkingDirectory {
            // Use the real host path: it exists under the read-only root, so
            // it can be re-bound writable. A synthetic mountpoint can't be
            // created on the read-only root.
            host: session.fallback_workdir.clone(),
            sandbox: session.fallback_workdir.clone(),
        }),
    }
}

fn collect_writable_mounts(
    request: &ExecRequest,
    working_directory: &WorkingDirectory,
) -> Result<Vec<WritableMount>, SandboxError> {
    let mut mounts = vec![WritableMount {
        host: working_directory.host.clone(),
        sandbox: working_directory.sandbox.clone(),
    }];

    for raw_path in &request.write_paths {
        let mount = normalize_writable_mount(raw_path, working_directory)?;
        if !mounts
            .iter()
            .any(|existing| existing.host == mount.host && existing.sandbox == mount.sandbox)
        {
            mounts.push(mount);
        }
    }

    Ok(mounts)
}

fn writable_cwd_mount(cwd: &Path) -> Result<WritableMount, SandboxError> {
    let cwd = projected_canonical_path(cwd)?;
    validate_bwrap_writable_mount(&cwd)?;
    Ok(WritableMount {
        host: cwd.clone(),
        sandbox: cwd,
    })
}

fn normalize_writable_mount(
    raw_path: &Path,
    working_directory: &WorkingDirectory,
) -> Result<WritableMount, SandboxError> {
    let (host_candidate, sandbox_candidate) = if raw_path.is_absolute() {
        (raw_path.to_path_buf(), raw_path.to_path_buf())
    } else {
        (
            working_directory.host.join(raw_path),
            working_directory.sandbox.join(raw_path),
        )
    };
    let host_candidate = projected_canonical_path(&host_candidate)?;
    let sandbox_candidate = projected_canonical_path(&sandbox_candidate)?;
    validate_bwrap_writable_mount(&host_candidate)?;
    validate_bwrap_writable_mount(&sandbox_candidate)?;

    let host_candidate_exists = host_candidate.exists();
    let host_candidate_is_dir = host_candidate.is_dir();
    let host_candidate_is_file = host_candidate.is_file();

    let host = if host_candidate_exists {
        if host_candidate_is_dir {
            host_candidate
        } else {
            host_candidate
                .parent()
                .map_or_else(|| working_directory.host.clone(), Path::to_path_buf)
        }
    } else {
        std::fs::create_dir_all(&host_candidate)
            .map_err(|error| SandboxError::ExecutionFailed(error.to_string()))?;
        host_candidate
    };

    let sandbox = if host_candidate_exists && host_candidate_is_file {
        sandbox_candidate
            .parent()
            .map_or_else(|| working_directory.sandbox.clone(), Path::to_path_buf)
    } else {
        sandbox_candidate
    };

    validate_bwrap_writable_mount(&host)?;
    validate_bwrap_writable_mount(&sandbox)?;

    Ok(WritableMount {
        host: std::fs::canonicalize(&host)
            .map_err(|error| SandboxError::ExecutionFailed(error.to_string()))?,
        sandbox,
    })
}

fn validate_bwrap_writable_mount(path: &Path) -> Result<(), SandboxError> {
    let absolute = projected_canonical_path(path)?;
    let isolation_roots = [
        Path::new(bubblewrap::FILESYSTEM_ROOT),
        Path::new(bubblewrap::FILESYSTEM_TMP),
        Path::new(bubblewrap::FILESYSTEM_RUN_USER),
        Path::new(bubblewrap::FILESYSTEM_DEV),
        Path::new(bubblewrap::FILESYSTEM_PROC),
        Path::new(bubblewrap::FILESYSTEM_SYS),
    ];
    let protected_subtrees = [
        Path::new(bubblewrap::FILESYSTEM_RUN_USER),
        Path::new(bubblewrap::FILESYSTEM_DEV),
        Path::new(bubblewrap::FILESYSTEM_PROC),
        Path::new(bubblewrap::FILESYSTEM_SYS),
    ];
    let protected = isolation_roots
        .iter()
        .any(|root| root.starts_with(&absolute))
        || protected_subtrees
            .iter()
            .any(|root| absolute.starts_with(root));
    if protected {
        return Err(SandboxError::ValidationFailed(format!(
            "bubblewrap writable mount overlaps protected path: {}",
            absolute.display()
        )));
    }
    Ok(())
}

fn build_bwrap_command_args(
    request: &ExecRequest,
    working_directory: &WorkingDirectory,
    writable_mounts: &[WritableMount],
) -> Result<Vec<String>, SandboxError> {
    // The preamble mounts the tmpfs before these binds, so a carve-out under
    // /tmp lands on the fresh tmpfs rather than being masked by it.
    let mut args = bwrap_isolation_preamble(request.needs_network);

    for mount in writable_mounts {
        args.push(bubblewrap::FLAG_BIND.to_string());
        args.push(path_string(mount.host.as_path()));
        args.push(path_string(mount.sandbox.as_path()));
    }

    args.push(bubblewrap::FLAG_CHDIR.to_string());
    args.push(path_string(working_directory.sandbox.as_path()));
    args.push(bubblewrap::FLAG_SEPARATOR.to_string());
    args.push(request.program.clone());
    args.extend(request.args.clone());

    Ok(args)
}

fn absolute_path(path: &Path) -> Result<PathBuf, SandboxError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|error| SandboxError::ExecutionFailed(error.to_string()))
    }
}

fn normalized_absolute_path(path: &Path) -> Result<PathBuf, SandboxError> {
    let absolute = absolute_path(path)?;
    let mut normalized = PathBuf::new();

    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    Ok(normalized)
}

/// Resolve symlinks in the nearest existing ancestor without creating the
/// requested path. This keeps validation side-effect free for missing mounts.
fn projected_canonical_path(path: &Path) -> Result<PathBuf, SandboxError> {
    let absolute = normalized_absolute_path(path)?;
    let mut ancestor = absolute.as_path();
    let mut missing = Vec::<OsString>::new();

    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let part = ancestor.file_name().ok_or_else(|| {
                    SandboxError::ExecutionFailed(format!(
                        "cannot resolve writable mount path: {}",
                        absolute.display()
                    ))
                })?;
                missing.push(part.to_os_string());
                ancestor = ancestor.parent().ok_or_else(|| {
                    SandboxError::ExecutionFailed(format!(
                        "cannot resolve writable mount ancestor: {}",
                        absolute.display()
                    ))
                })?;
            }
            Err(error) => return Err(SandboxError::ExecutionFailed(error.to_string())),
        }
    }

    let mut resolved = std::fs::canonicalize(ancestor)
        .map_err(|error| SandboxError::ExecutionFailed(error.to_string()))?;
    for part in missing.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn map_bwrap_spawn_error(error: std::io::Error) -> SandboxError {
    match error.kind() {
        std::io::ErrorKind::NotFound => {
            SandboxError::NotAvailable(bubblewrap::ERR_NOT_AVAILABLE.into())
        }
        std::io::ErrorKind::PermissionDenied => SandboxError::PermissionDenied(error.to_string()),
        _ => {
            SandboxError::ExecutionFailed(format!("{}: {error}", bubblewrap::ERR_EXECUTION_PREFIX))
        }
    }
}

fn map_systemd_spawn_error(error: std::io::Error) -> SandboxError {
    match error.kind() {
        std::io::ErrorKind::NotFound => {
            SandboxError::NotAvailable(systemd_run::ERR_NOT_AVAILABLE.into())
        }
        std::io::ErrorKind::PermissionDenied => SandboxError::PermissionDenied(error.to_string()),
        _ => {
            SandboxError::ExecutionFailed(format!("{}: {error}", systemd_run::ERR_EXECUTION_PREFIX))
        }
    }
}

fn session_id(prefix: &str) -> String {
    format!("{}-{}", prefix, monotonic_nanos())
}

fn monotonic_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn truncate_output(stdout: &mut String, stderr: &mut String, max_output_bytes: usize) {
    let mut remaining = max_output_bytes;
    truncate_string_in_place(stdout, &mut remaining);
    truncate_string_in_place(stderr, &mut remaining);
}

fn truncate_string_in_place(value: &mut String, remaining: &mut usize) {
    if *remaining == 0 {
        value.clear();
        return;
    }

    if value.len() <= *remaining {
        *remaining -= value.len();
        return;
    }

    let boundary = value.floor_char_boundary(*remaining);
    value.truncate(boundary);
    *remaining = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn write_executable(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write probe executable");
        let mut permissions = std::fs::metadata(&path)
            .expect("probe executable metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&path, permissions).expect("mark probe executable");
        path
    }

    #[test]
    fn bubblewrap_wrapper_carries_the_complete_child_environment() {
        let env = vec![
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("APXM_TEST".to_string(), "present".to_string()),
        ];
        let cwd = writable_cwd_mount(Path::new("/tmp/work")).expect("validated cwd mount");
        let wrapped = build_bwrap_wrap_args("node", &["worker.js".to_string()], &cwd, false, &env);

        assert!(
            wrapped
                .iter()
                .any(|arg| arg == bubblewrap::FLAG_UNSHARE_NET)
        );
        assert_eq!(
            &wrapped[wrapped.len() - 7..],
            [
                executables::ENV,
                systemd_run::ENV_CLEAR,
                systemd_run::ENV_OPTION_TERMINATOR,
                "PATH=/usr/bin",
                "APXM_TEST=present",
                "node",
                "worker.js",
            ]
        );
    }

    #[test]
    fn systemd_wrapper_declares_only_the_supported_isolation_properties() {
        let env = vec![("PATH".to_string(), "/usr/bin".to_string())];
        let wrapped = build_systemd_run_args(
            "node",
            &["worker.js".to_string()],
            Path::new("/tmp/work"),
            false,
            &env,
            None,
        );

        assert!(
            wrapped
                .iter()
                .any(|arg| arg == systemd_run::PROPERTY_SYSTEM_CALL_FILTER_NO_NETWORK)
        );
        assert!(!wrapped.iter().any(|arg| arg.contains("ProtectSystem")));
        assert!(!wrapped.iter().any(|arg| arg.contains("PrivateNetwork")));
        assert_eq!(
            &wrapped[wrapped.len() - 6..],
            [
                executables::ENV,
                systemd_run::ENV_CLEAR,
                systemd_run::ENV_OPTION_TERMINATOR,
                "PATH=/usr/bin",
                "node",
                "worker.js",
            ]
        );
    }

    #[test]
    fn systemd_backend_reports_bounded_host_guarantees() {
        let capabilities = SystemdSandboxBackend::with_default_policy().capabilities();

        assert_eq!(capabilities.isolation_level, IsolationLevel::OsLevel);
        assert!(!capabilities.supports_filesystem_restriction);
        assert!(capabilities.supports_network_restriction);
        assert!(capabilities.supports_syscall_filtering);
        assert!(!capabilities.supports_resource_limits);
    }

    #[test]
    fn bubblewrap_does_not_claim_readable_path_restriction() {
        let capabilities = BubblewrapSandboxBackend::with_default_policy().capabilities();

        assert_eq!(capabilities.isolation_level, IsolationLevel::Container);
        assert!(!capabilities.supports_filesystem_restriction);
        assert!(capabilities.supports_network_restriction);
    }

    #[test]
    fn systemd_launcher_environment_is_limited_to_user_manager_connection_state() {
        assert_eq!(
            sandbox_env::SYSTEMD_LAUNCHER_PASSTHROUGH,
            &[
                sandbox_env::DBUS_SESSION_BUS_ADDRESS,
                sandbox_env::XDG_RUNTIME_DIR,
            ]
        );
        assert!(
            sandbox_env::CHILD_PASSTHROUGH
                .iter()
                .all(|key| !sandbox_env::SYSTEMD_LAUNCHER_PASSTHROUGH.contains(key))
        );
    }

    #[test]
    fn bubblewrap_probe_requires_the_isolation_command_to_succeed() {
        let dir = tempfile::tempdir().expect("probe tempdir");
        let succeeds = write_executable(dir.path(), "succeeds", "#!/bin/sh\nexit 0\n");
        let fails = write_executable(dir.path(), "fails", "#!/bin/sh\nexit 1\n");

        assert!(probe_bubblewrap(succeeds.to_str().expect("utf-8 path")));
        assert!(!probe_bubblewrap(fails.to_str().expect("utf-8 path")));
    }

    #[test]
    fn systemd_probe_requires_a_working_launcher_and_enforced_network_block() {
        let dir = tempfile::tempdir().expect("probe tempdir");
        let always_succeeds =
            write_executable(dir.path(), "always-succeeds", "#!/bin/sh\nexit 0\n");
        let always_fails = write_executable(dir.path(), "always-fails", "#!/bin/sh\nexit 1\n");
        let enforces = write_executable(
            dir.path(),
            "enforces",
            &format!(
                "#!/bin/sh\ncase \"$*\" in\n  *{restricted}*) printf {restricted}; exit 0 ;;\n  *'@network-io'*) printf '%s\\n' 'bash: connect: {denied}' >&2; exit 1 ;;\n  *) printf {network}; exit 0 ;;\nesac\n",
                restricted = systemd_run::PROBE_RESTRICTED_OK,
                denied = systemd_run::PROBE_NETWORK_DENIED,
                network = systemd_run::PROBE_NETWORK_OK,
            ),
        );
        let wrong_denial = write_executable(
            dir.path(),
            "wrong-denial",
            &format!(
                "#!/bin/sh\ncase \"$*\" in\n  *{restricted}*) printf {restricted}; exit 0 ;;\n  *'@network-io'*) printf '%s\\n' 'bash: connect: Connection refused' >&2; exit 1 ;;\n  *) printf {network}; exit 0 ;;\nesac\n",
                restricted = systemd_run::PROBE_RESTRICTED_OK,
                network = systemd_run::PROBE_NETWORK_OK,
            ),
        );

        assert!(!probe_systemd_run(
            always_succeeds.to_str().expect("utf-8 path")
        ));
        assert!(!probe_systemd_run(
            always_fails.to_str().expect("utf-8 path")
        ));
        assert!(!probe_systemd_run(
            wrong_denial.to_str().expect("utf-8 path")
        ));
        assert!(probe_systemd_run(enforces.to_str().expect("utf-8 path")));
    }

    #[tokio::test]
    async fn functional_systemd_wrapper_separates_launcher_and_child_environments() {
        if !systemd_run_available() {
            return;
        }
        let backend = SystemdSandboxBackend::with_default_policy();
        let env = sandbox_env::child_environment([(sandbox_env::PATH, "/usr/bin:/bin")]);
        let wrapped = backend
            .wrap_command(executables::ENV, &[], Path::new("/tmp"), false, &env)
            .expect("wrap no-network command");
        let child = wrapped
            .spawn(|command| {
                command.stdout(StdStdio::piped()).stderr(StdStdio::piped());
            })
            .expect("launch wrapped command");
        let output = child
            .wait_with_output()
            .await
            .expect("wait for wrapped command");
        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        let child_environment = String::from_utf8_lossy(&output.stdout);
        let dbus_prefix = format!("{}=", sandbox_env::DBUS_SESSION_BUS_ADDRESS);
        let runtime_dir_prefix = format!("{}=", sandbox_env::XDG_RUNTIME_DIR);
        assert!(
            child_environment
                .lines()
                .any(|entry| entry == "PATH=/usr/bin:/bin")
        );
        assert!(
            child_environment.lines().all(|entry| {
                !entry.starts_with(&dbus_prefix) && !entry.starts_with(&runtime_dir_prefix)
            }),
            "launcher environment leaked into child: {child_environment}"
        );
    }

    #[test]
    fn systemd_guard_stops_the_owned_transient_unit() {
        let dir = tempfile::tempdir().expect("guard tempdir");
        let log = dir.path().join("systemctl.log");
        let systemctl = write_executable(
            dir.path(),
            "systemctl",
            &format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", log.display()),
        );

        drop(SystemdUnitGuard::with_systemctl(
            "apxm-test.service".to_string(),
            systemctl.to_string_lossy().into_owned(),
        ));

        let args = std::fs::read_to_string(log).expect("systemctl invocation log");
        assert_eq!(args, "--user\nstop\napxm-test.service\n");
    }

    #[test]
    fn systemd_wrap_rejects_network_and_invalid_environment_requests() {
        let backend = SystemdSandboxBackend::with_default_policy();
        let networked = backend.wrap_command("node", &[], Path::new("/tmp"), true, &[]);
        assert!(matches!(
            networked,
            Err(SandboxError::RequirementsNotMet(_))
        ));

        let invalid_env = vec![("--chdir".to_string(), "/tmp".to_string())];
        let invalid = backend.wrap_command("node", &[], Path::new("/tmp"), false, &invalid_env);
        assert!(matches!(invalid, Err(SandboxError::ValidationFailed(_))));
    }

    #[test]
    fn bubblewrap_rejects_writable_mounts_that_override_isolation_roots() {
        let backend = BubblewrapSandboxBackend::with_default_policy();
        for path in [
            "/",
            "/tmp",
            "/run",
            "/run/user",
            "/run/user/1000",
            "/tmp/apxm/../../run/user/1000",
            "/run/user/../user/1000",
            "/proc/self",
            "/dev",
        ] {
            let result = backend.wrap_command("true", &[], Path::new(path), false, &[]);
            assert!(
                matches!(result, Err(SandboxError::ValidationFailed(_))),
                "protected cwd was admitted: {path}"
            );
        }
        assert!(
            backend
                .wrap_command("true", &[], Path::new("/tmp/apxm-work"), false, &[])
                .is_ok()
        );
    }

    #[test]
    fn rejected_symlinked_writable_mount_is_not_created() {
        let dir = tempfile::tempdir().expect("mount tempdir");
        symlink(
            bubblewrap::FILESYSTEM_RUN_USER,
            dir.path().join("runtime-user"),
        )
        .expect("runtime-user symlink");
        let child = format!("apxm-rejected-{}", monotonic_nanos());
        let protected_target = Path::new(bubblewrap::FILESYSTEM_RUN_USER).join(&child);
        assert!(!protected_target.exists());

        let working_directory = WorkingDirectory {
            host: dir.path().to_path_buf(),
            sandbox: dir.path().to_path_buf(),
        };
        let symlinked_cwd = dir.path().join("runtime-user").join(&child);
        let backend = BubblewrapSandboxBackend::with_default_policy();
        assert!(matches!(
            backend.wrap_command("true", &[], &symlinked_cwd, false, &[]),
            Err(SandboxError::ValidationFailed(_))
        ));
        let result =
            normalize_writable_mount(&Path::new("runtime-user").join(&child), &working_directory);

        assert!(matches!(result, Err(SandboxError::ValidationFailed(_))));
        assert!(
            !protected_target.exists(),
            "rejected mount validation created a protected host path"
        );
    }

    #[test]
    fn functional_systemd_backend_blocks_nested_user_service_escape() {
        if !systemd_run_available() {
            return;
        }
        let (Ok(dbus), Ok(runtime_dir)) = (
            std::env::var(sandbox_env::DBUS_SESSION_BUS_ADDRESS),
            std::env::var(sandbox_env::XDG_RUNTIME_DIR),
        ) else {
            return;
        };
        let env = sandbox_env::child_environment([
            (sandbox_env::DBUS_SESSION_BUS_ADDRESS, dbus.as_str()),
            (sandbox_env::XDG_RUNTIME_DIR, runtime_dir.as_str()),
        ]);
        let nested_args = vec![
            systemd_run::FLAG_USER.to_string(),
            systemd_run::FLAG_WAIT.to_string(),
            systemd_run::FLAG_PIPE.to_string(),
            systemd_run::FLAG_QUIET.to_string(),
            executables::TRUE.to_string(),
        ];
        let args = build_systemd_run_args(
            executables::SYSTEMD_RUN,
            &nested_args,
            Path::new("/tmp"),
            false,
            &env,
            None,
        );

        let output = run_systemd_probe(executables::SYSTEMD_RUN, &args)
            .expect("functional systemd launcher");
        assert!(
            !output.status.success(),
            "nested systemd-run escaped network filtering"
        );
    }
}
