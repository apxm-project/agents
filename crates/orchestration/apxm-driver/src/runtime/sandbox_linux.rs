use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio as StdStdio};
use std::time::Instant;

use apxm_runtime::sandbox::constants::{
    backend_names, bubblewrap, env as sandbox_env, executables, session_prefixes,
};
use apxm_runtime::sandbox::policy::SandboxPolicy;
use apxm_runtime::sandbox::{
    ExecRequest, ExecResult, IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxContext,
    SandboxError, ValidationResult,
};
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub struct BubblewrapSandboxBackend {
    policy: SandboxPolicy,
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
    scratch_dir: PathBuf,
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

#[async_trait]
impl SandboxBackend for BubblewrapSandboxBackend {
    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            isolation_level: IsolationLevel::Container,
            supports_filesystem_restriction: true,
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

    fn validate(&self, request: &ExecRequest) -> ValidationResult {
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

        let mut warnings = Vec::new();
        if !request.read_paths.is_empty() {
            warnings.push(bubblewrap::WARN_READ_ALLOWLISTS.to_string());
        }

        if warnings.is_empty() {
            ValidationResult::Ok
        } else {
            ValidationResult::Degraded { warnings }
        }
    }

    async fn create_session(&self) -> Result<SandboxContext, SandboxError> {
        let root =
            tempfile::tempdir().map_err(|error| SandboxError::SessionError(error.to_string()))?;
        let scratch_dir = root.path().join(session_prefixes::SCRATCH);
        let fallback_workdir = root.path().join(session_prefixes::WORKDIR);

        std::fs::create_dir_all(&scratch_dir)
            .map_err(|error| SandboxError::SessionError(error.to_string()))?;
        std::fs::create_dir_all(&fallback_workdir)
            .map_err(|error| SandboxError::SessionError(error.to_string()))?;

        Ok(SandboxContext::new(
            session_id(session_prefixes::BUBBLEWRAP),
            backend_names::BUBBLEWRAP,
            IsolationLevel::Container,
            BubblewrapSession {
                root,
                scratch_dir,
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
        let command_args = build_bwrap_command_args(
            &request,
            &working_directory,
            &writable_mounts,
            session.scratch_dir.as_path(),
        )?;

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

        for key in sandbox_env::SAFE_PASSTHROUGH {
            if let Ok(value) = std::env::var(key) {
                command.env(key, value);
            }
        }
        command.env(sandbox_env::TMPDIR, bubblewrap::TMP_DIR);
        command.env(sandbox_env::TEMP, bubblewrap::TMP_DIR);
        command.env(sandbox_env::TMP, bubblewrap::TMP_DIR);
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
}

fn bubblewrap_available() -> bool {
    StdCommand::new(executables::BUBBLEWRAP)
        .arg(bubblewrap::VERSION_ARG)
        .stdout(StdStdio::null())
        .stderr(StdStdio::null())
        .status()
        .is_ok()
}

fn resolve_working_directory(
    requested_working_dir: Option<&PathBuf>,
    session: &BubblewrapSession,
) -> Result<WorkingDirectory, SandboxError> {
    match requested_working_dir {
        Some(path) => {
            let host = absolute_path(path)?;
            std::fs::create_dir_all(&host)
                .map_err(|error| SandboxError::ExecutionFailed(error.to_string()))?;
            if !host.is_dir() {
                return Err(SandboxError::ExecutionFailed(
                    bubblewrap::ERR_WORKDIR_NOT_DIRECTORY.to_string(),
                ));
            }
            Ok(WorkingDirectory {
                host,
                sandbox: absolute_path(path)?,
            })
        }
        None => Ok(WorkingDirectory {
            host: session.fallback_workdir.clone(),
            sandbox: PathBuf::from(bubblewrap::WORKDIR),
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
    let host_candidate_exists = host_candidate.exists();
    let host_candidate_is_dir = host_candidate.is_dir();
    let host_candidate_is_file = host_candidate.is_file();

    let host = if host_candidate_exists {
        if host_candidate_is_dir {
            host_candidate
        } else {
            host_candidate
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| working_directory.host.clone())
        }
    } else {
        std::fs::create_dir_all(&host_candidate)
            .map_err(|error| SandboxError::ExecutionFailed(error.to_string()))?;
        host_candidate
    };

    let sandbox = if host_candidate_exists && host_candidate_is_file {
        sandbox_candidate
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| working_directory.sandbox.clone())
    } else {
        sandbox_candidate
    };

    Ok(WritableMount {
        host: absolute_path(&host)?,
        sandbox,
    })
}

fn build_bwrap_command_args(
    request: &ExecRequest,
    working_directory: &WorkingDirectory,
    writable_mounts: &[WritableMount],
    scratch_dir: &Path,
) -> Result<Vec<String>, SandboxError> {
    let mut args = vec![
        bubblewrap::FLAG_NEW_SESSION.to_string(),
        bubblewrap::FLAG_DIE_WITH_PARENT.to_string(),
        bubblewrap::FLAG_RO_BIND.to_string(),
        bubblewrap::FILESYSTEM_ROOT.to_string(),
        bubblewrap::FILESYSTEM_ROOT.to_string(),
        bubblewrap::FLAG_DEV.to_string(),
        bubblewrap::FILESYSTEM_DEV.to_string(),
        bubblewrap::FLAG_UNSHARE_USER.to_string(),
        bubblewrap::FLAG_UNSHARE_PID.to_string(),
    ];

    if !request.needs_network {
        args.push(bubblewrap::FLAG_UNSHARE_NET.to_string());
    }

    args.push(bubblewrap::FLAG_PROC.to_string());
    args.push(bubblewrap::FILESYSTEM_PROC.to_string());

    let mut created_dirs = HashSet::new();
    ensure_sandbox_dir(&mut args, &mut created_dirs, Path::new(bubblewrap::TMP_DIR));
    args.push(bubblewrap::FLAG_BIND.to_string());
    args.push(path_string(scratch_dir));
    args.push(bubblewrap::TMP_DIR.to_string());

    for mount in writable_mounts {
        ensure_mount_target(&mut args, &mut created_dirs, mount.sandbox.as_path());
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

fn ensure_mount_target(args: &mut Vec<String>, created_dirs: &mut HashSet<PathBuf>, path: &Path) {
    if path.starts_with(Path::new(bubblewrap::WORKDIR)) {
        ensure_sandbox_dir(args, created_dirs, Path::new(bubblewrap::WORKDIR));

        let mut current = PathBuf::from(bubblewrap::WORKDIR);
        if let Ok(relative) = path.strip_prefix(Path::new(bubblewrap::WORKDIR)) {
            for component in relative.components() {
                current.push(component.as_os_str());
                ensure_sandbox_dir(args, created_dirs, current.as_path());
            }
        }
    } else if path.starts_with(Path::new(bubblewrap::TMP_DIR)) {
        ensure_sandbox_dir(args, created_dirs, Path::new(bubblewrap::TMP_DIR));
    }
}

fn ensure_sandbox_dir(args: &mut Vec<String>, created_dirs: &mut HashSet<PathBuf>, path: &Path) {
    let path = path.to_path_buf();
    if created_dirs.insert(path.clone()) {
        args.push(bubblewrap::FLAG_DIR.to_string());
        args.push(path_string(path.as_path()));
    }
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

    #[test]
    fn validate_reports_unavailable_without_bwrap() {
        let backend = BubblewrapSandboxBackend::with_default_policy();

        if !backend.is_available() {
            assert!(matches!(
                backend.validate(&ExecRequest {
                    min_isolation: IsolationLevel::OsLevel,
                    ..ExecRequest::default()
                }),
                ValidationResult::Unsupported { .. }
            ));
        }
    }

    #[test]
    fn validate_reports_degraded_read_allowlists() {
        let backend = BubblewrapSandboxBackend::with_default_policy();

        if backend.is_available() {
            let result = backend.validate(&ExecRequest {
                read_paths: vec![PathBuf::from("/etc/hosts")],
                min_isolation: IsolationLevel::OsLevel,
                ..ExecRequest::default()
            });
            assert!(matches!(result, ValidationResult::Degraded { .. }));
        }
    }

    #[test]
    fn build_bwrap_args_use_read_only_root_and_network_isolation() {
        let request = ExecRequest {
            min_isolation: IsolationLevel::OsLevel,
            program: "echo".to_string(),
            args: vec!["hello".to_string()],
            needs_network: false,
            ..ExecRequest::default()
        };
        let working_directory = WorkingDirectory {
            host: PathBuf::from("/tmp/apxm-test-workdir"),
            sandbox: PathBuf::from(bubblewrap::WORKDIR),
        };
        let writable_mounts = vec![WritableMount {
            host: working_directory.host.clone(),
            sandbox: working_directory.sandbox.clone(),
        }];

        let args = build_bwrap_command_args(
            &request,
            &working_directory,
            &writable_mounts,
            Path::new("/tmp/apxm-test-scratch"),
        )
        .unwrap();

        assert!(args.windows(3).any(|window| {
            window
                == [
                    bubblewrap::FLAG_RO_BIND,
                    bubblewrap::FILESYSTEM_ROOT,
                    bubblewrap::FILESYSTEM_ROOT,
                ]
        }));
        assert!(args.iter().any(|arg| arg == bubblewrap::FLAG_UNSHARE_NET));
        assert!(args.iter().any(|arg| arg == bubblewrap::WORKDIR));
    }

    #[test]
    fn build_bwrap_args_preserve_network_when_requested() {
        let request = ExecRequest {
            min_isolation: IsolationLevel::OsLevel,
            program: "echo".to_string(),
            args: vec!["hello".to_string()],
            needs_network: true,
            ..ExecRequest::default()
        };
        let working_directory = WorkingDirectory {
            host: PathBuf::from("/tmp/apxm-test-workdir"),
            sandbox: PathBuf::from(bubblewrap::WORKDIR),
        };
        let writable_mounts = vec![WritableMount {
            host: working_directory.host.clone(),
            sandbox: working_directory.sandbox.clone(),
        }];

        let args = build_bwrap_command_args(
            &request,
            &working_directory,
            &writable_mounts,
            Path::new("/tmp/apxm-test-scratch"),
        )
        .unwrap();

        assert!(!args.iter().any(|arg| arg == bubblewrap::FLAG_UNSHARE_NET));
    }
}
