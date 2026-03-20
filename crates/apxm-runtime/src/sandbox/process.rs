use super::policy::SandboxPolicy;
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
        for key in &["PATH", "HOME", "LANG", "LC_ALL", "TERM"] {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
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

                // Truncate (floor to char boundary to avoid panic on multi-byte UTF-8)
                let stdout = if stdout.len() > max {
                    stdout[..stdout.floor_char_boundary(max)].to_string()
                } else {
                    stdout
                };
                let stderr = if stderr.len() > max {
                    stderr[..stderr.floor_char_boundary(max)].to_string()
                } else {
                    stderr
                };

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
                    stderr: "Process timed out and was killed".to_string(),
                    exit_code: -1,
                    timed_out: true,
                })
            }
        }
    }

    /// Execute a script (write to temp file, then run with interpreter).
    pub async fn execute_script(
        &self,
        interpreter: &str,
        code: &str,
    ) -> std::io::Result<SandboxResult> {
        let script_dir = tempfile::tempdir()?;
        let script_path = script_dir.path().join("script");
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
            &[script_path.to_str().unwrap_or("script")],
            None,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_execute_simple_command() {
        let sandbox = ProcessSandbox::with_default_policy();
        let result = sandbox
            .execute("echo", &["hello world"], None)
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "hello world");
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn test_execute_script() {
        let sandbox = ProcessSandbox::with_default_policy();
        let result = sandbox
            .execute_script("bash", "echo 'from script'")
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("from script"));
    }

    #[tokio::test]
    async fn test_timeout_kills_process() {
        let policy = SandboxPolicy {
            timeout: std::time::Duration::from_millis(500),
            ..SandboxPolicy::default()
        };
        let sandbox = ProcessSandbox::new(policy);
        let result = sandbox.execute("sleep", &["10"], None).await.unwrap();
        assert!(result.timed_out);
    }

    #[tokio::test]
    async fn test_env_vars_restricted() {
        // We use env_clear() so no env vars from the parent leak through
        // except the whitelisted ones (PATH, HOME, LANG, LC_ALL, TERM).
        let sandbox = ProcessSandbox::with_default_policy();
        let result = sandbox
            .execute_script("bash", "echo $OPENAI_API_KEY")
            .await
            .unwrap();
        assert!(
            result.stdout.trim().is_empty(),
            "Sensitive env var should not be visible"
        );
    }
}
