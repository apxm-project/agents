use super::policy::SandboxPolicy;
#[cfg(test)]
use crate::sandbox::constants::session_prefixes;
use crate::sandbox::constants::{env as sandbox_env, messages};
use tokio::process::Command;

/// Result of a sandboxed execution.
#[derive(Debug, Clone)]
pub struct SandboxResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub timed_out: bool,
}

/// Process-based sandbox for code execution.
pub struct ProcessSandbox {
    policy: SandboxPolicy,
}

impl ProcessSandbox {
    pub fn new(policy: SandboxPolicy) -> Self {
        Self { policy }
    }

    pub fn with_default_policy() -> Self {
        Self::new(SandboxPolicy::default())
    }

    /// Execute a command in the sandbox.
    pub async fn execute(
        &self,
        command: &str,
        args: &[&str],
        stdin_data: Option<&str>,
    ) -> std::io::Result<SandboxResult> {
        if let Some(allowed_commands) = &self.policy.allowed_commands
            && !allowed_commands.iter().any(|allowed| allowed == command)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                messages::COMMAND_BLOCKED_BY_POLICY,
            ));
        }

        // Create temp working directory if not specified
        let temp_dir;
        let work_dir = if let Some(dir) = &self.policy.working_dir {
            dir.clone()
        } else {
            temp_dir = tempfile::tempdir()?;
            temp_dir.path().to_path_buf()
        };

        // Ensure work_dir exists
        tokio::fs::create_dir_all(&work_dir).await?;

        let mut cmd = Command::new(command);
        cmd.args(args)
            .current_dir(&work_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Set up restricted environment: clear everything, then whitelist safe vars
        cmd.env_clear();
        for key in sandbox_env::CHILD_PASSTHROUGH {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        for (key, value) in &self.policy.env_overrides {
            if !self.policy.blocks_env_var(key) {
                cmd.env(key, value);
            }
        }

        if stdin_data.is_some() {
            cmd.stdin(std::process::Stdio::piped());
        }

        let mut child = cmd.spawn()?;

        // Write stdin if provided
        if let Some(data) = stdin_data
            && let Some(mut stdin) = child.stdin.take()
        {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(data.as_bytes()).await;
            drop(stdin);
        }

        // Take stdout/stderr handles before waiting so we can read them
        // while retaining ownership of child for kill-on-timeout.
        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        let max = self.policy.max_output_bytes;

        // Wait for the child with a timeout
        match tokio::time::timeout(self.policy.timeout, child.wait()).await {
            Ok(Ok(status)) => {
                // Process exited within timeout. Read captured output.
                let stdout = if let Some(mut out) = stdout_handle {
                    use tokio::io::AsyncReadExt;
                    let mut buf = Vec::new();
                    let _ = out.read_to_end(&mut buf).await;
                    String::from_utf8_lossy(&buf).into_owned()
                } else {
                    String::new()
                };
                let stderr = if let Some(mut err) = stderr_handle {
                    use tokio::io::AsyncReadExt;
                    let mut buf = Vec::new();
                    let _ = err.read_to_end(&mut buf).await;
                    String::from_utf8_lossy(&buf).into_owned()
                } else {
                    String::new()
                };

                let stdout = truncate_output(stdout, max);
                let stderr = truncate_output(stderr, max);

                Ok(SandboxResult {
                    stdout,
                    stderr,
                    exit_code: status.code().unwrap_or(-1),
                    timed_out: false,
                })
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // Timeout - kill the process
                let _ = child.kill().await;
                Ok(SandboxResult {
                    stdout: String::new(),
                    stderr: messages::PROCESS_TIMED_OUT_AND_KILLED.to_string(),
                    exit_code: -1,
                    timed_out: true,
                })
            }
        }
    }

    /// Execute a script (write to temp file, then run with interpreter).
    ///
    /// Test-only: production code reaches `ProcessSandbox` through
    /// `ProcessSandboxBackend::execute`, never this helper.
    #[cfg(test)]
    pub async fn execute_script(
        &self,
        interpreter: &str,
        code: &str,
    ) -> std::io::Result<SandboxResult> {
        let script_dir = tempfile::tempdir()?;
        let script_path = script_dir.path().join(session_prefixes::SCRIPT);
        tokio::fs::write(&script_path, code).await?;

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&script_path, perms)?;
        }

        self.execute(
            interpreter,
            &[script_path.to_str().unwrap_or(session_prefixes::SCRIPT)],
            None,
        )
        .await
    }
}

/// Truncate captured process output without splitting a UTF-8 code point.
fn truncate_output(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let boundary = text
        .char_indices()
        .take_while(|(index, _)| *index < max)
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0);
    text[..boundary].to_string()
}
