use dashmap::DashMap;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;

use apxm_runtime::sandbox::SandboxBackend;

use crate::AcpError;
use crate::constants::terminal as term_consts;

pub struct TerminalManager {
    terminals: DashMap<String, Terminal>,
    /// When set, terminals the agent opens are confined under this backend.
    /// `terminal/create` spawns on the host via the reverse handler, so an
    /// otherwise-confined agent could escape through it without this.
    sandbox: Option<Arc<dyn SandboxBackend>>,
}

struct Terminal {
    child: Option<apxm_runtime::sandbox::WrappedChild>,
    output_buffer: Vec<u8>,
    exit_status: Option<ExitStatus>,
    truncated: bool,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            terminals: DashMap::new(),
            sandbox: None,
        }
    }

    /// Create a manager whose terminals are confined under `sandbox`.
    pub fn with_sandbox(sandbox: Option<Arc<dyn SandboxBackend>>) -> Self {
        Self {
            terminals: DashMap::new(),
            sandbox,
        }
    }

    /// Spawn a new terminal process, returning its terminal ID.
    pub async fn create(
        &self,
        command: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &[(String, String)],
    ) -> Result<String, AcpError> {
        let terminal_id = uuid::Uuid::new_v4().to_string();

        // Confine the spawned terminal when a backend is configured. Bind the
        // requested cwd (falling back to the process cwd) so the command can't
        // reach outside it for writes or escape network isolation policy.
        let child_env = apxm_runtime::sandbox::constants::env::child_environment(
            env.iter().map(|(key, value)| (key, value)),
        );
        let sandbox_command = match &self.sandbox {
            Some(backend) => {
                let wrap_cwd = match cwd {
                    Some(path) => PathBuf::from(path),
                    None => std::env::current_dir().map_err(|error| AcpError::Spawn {
                        agent: command.to_string(),
                        reason: format!("failed to resolve terminal working directory: {error}"),
                    })?,
                };
                backend
                    .wrap_command(command, args, &wrap_cwd, true, &child_env)
                    .map_err(|error| AcpError::Spawn {
                        agent: command.to_string(),
                        reason: error.to_string(),
                    })?
            }
            None => {
                apxm_runtime::sandbox::WrappedCommand::direct(command, args.to_vec(), child_env)
                    .map_err(|error| AcpError::Spawn {
                        agent: command.to_string(),
                        reason: error.to_string(),
                    })?
            }
        };

        let child = sandbox_command
            .spawn(|process| {
                process
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                if let Some(dir) = cwd {
                    process.current_dir(dir);
                }
            })
            .map_err(|e| AcpError::Spawn {
                agent: command.to_string(),
                reason: e.to_string(),
            })?;

        self.terminals.insert(
            terminal_id.clone(),
            Terminal {
                child: Some(child),
                output_buffer: Vec::new(),
                exit_status: None,
                truncated: false,
            },
        );

        Ok(terminal_id)
    }

    /// Collect available output from a terminal.
    pub async fn output(&self, terminal_id: &str) -> Result<(String, bool, Option<i32>), AcpError> {
        let mut entry = self
            .terminals
            .get_mut(terminal_id)
            .ok_or_else(|| AcpError::Protocol(format!("Unknown terminal: {terminal_id}")))?;
        let term = entry.value_mut();

        // Try to read any available output from the child
        if let Some(child) = &mut term.child {
            if let Some(stdout) = child.stdout.as_mut() {
                let mut buf = vec![0u8; 4096];
                // Non-blocking read attempt
                match tokio::time::timeout(
                    std::time::Duration::from_millis(term_consts::READ_POLL_TIMEOUT_MS),
                    stdout.read(&mut buf),
                )
                .await
                {
                    Ok(Ok(n)) if n > 0 => {
                        if term.output_buffer.len() + n > term_consts::DEFAULT_OUTPUT_LIMIT {
                            term.truncated = true;
                        } else {
                            term.output_buffer.extend_from_slice(&buf[..n]);
                        }
                    }
                    _ => {}
                }
            }
            // Check if child has exited
            if let Ok(Some(status)) = child.try_wait() {
                term.exit_status = Some(status);
            }
        }

        let output = String::from_utf8_lossy(&term.output_buffer).to_string();
        let exit_code = term.exit_status.and_then(|s| s.code());
        Ok((output, term.truncated, exit_code))
    }

    /// Wait for a terminal process to exit.
    pub async fn wait_for_exit(&self, terminal_id: &str) -> Result<(i32, Option<i32>), AcpError> {
        let mut child = {
            let mut entry = self
                .terminals
                .get_mut(terminal_id)
                .ok_or_else(|| AcpError::Protocol(format!("Unknown terminal: {terminal_id}")))?;
            entry.value_mut().child.take()
        };

        if let Some(ref mut child) = child {
            let status = child
                .wait()
                .await
                .map_err(|e| AcpError::Protocol(format!("wait failed: {e}")))?;

            // Drain remaining output
            if let Some(mut stdout) = child.stdout.take() {
                let mut buf = Vec::new();
                let _ = stdout.read_to_end(&mut buf).await;
                if let Some(mut entry) = self.terminals.get_mut(terminal_id) {
                    let term = entry.value_mut();
                    if term.output_buffer.len() + buf.len() <= term_consts::DEFAULT_OUTPUT_LIMIT {
                        term.output_buffer.extend_from_slice(&buf);
                    } else {
                        term.truncated = true;
                    }
                    term.exit_status = Some(status);
                }
            }

            let exit_code = status.code().unwrap_or(-1);
            // signal = None for normal exit
            #[cfg(unix)]
            let signal = {
                use std::os::unix::process::ExitStatusExt;
                status.signal()
            };
            #[cfg(not(unix))]
            let signal = None;

            Ok((exit_code, signal))
        } else {
            // Child already taken (already waited)
            let entry = self
                .terminals
                .get(terminal_id)
                .ok_or_else(|| AcpError::Protocol(format!("Unknown terminal: {terminal_id}")))?;
            let code = entry.exit_status.and_then(|s| s.code()).unwrap_or(-1);
            Ok((code, None))
        }
    }

    /// Kill a terminal process.
    pub async fn kill(&self, terminal_id: &str) -> Result<(), AcpError> {
        if let Some(mut entry) = self.terminals.get_mut(terminal_id)
            && let Some(ref mut child) = entry.value_mut().child
        {
            let _ = child.kill().await;
        }
        Ok(())
    }

    /// Release a terminal, cleaning up resources.
    pub fn release(&self, terminal_id: &str) {
        self.terminals.remove(terminal_id);
    }
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}
