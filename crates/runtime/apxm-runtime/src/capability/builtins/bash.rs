use super::require_string_arg;
use crate::capability::{
    executor::{CapabilityExecutor, CapabilityResult, exec_result_to_value},
    metadata::CapabilityMetadata,
};
use crate::sandbox::constants::{executables, shell_args};
use crate::sandbox::{ExecRequest, ExecResult, IsolationLevel};
use apxm_core::{
    error::RuntimeError,
    types::{AISOperationType, Value},
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, process::Stdio, time::Instant};
use tokio::{process::Command, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BashConfig {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub blocked_commands: Vec<String>,
    #[serde(default)]
    pub allowed_commands: Option<Vec<String>>,
    #[serde(default)]
    pub working_directory: Option<PathBuf>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_output")]
    pub max_output_bytes: usize,
}

fn default_timeout() -> u64 {
    120
}

fn default_max_output() -> usize {
    100_000
}

impl Default for BashConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            blocked_commands: Vec::new(),
            allowed_commands: None,
            working_directory: None,
            timeout_secs: default_timeout(),
            max_output_bytes: default_max_output(),
        }
    }
}

pub struct BashCapability {
    metadata: CapabilityMetadata,
    config: BashConfig,
}

impl BashCapability {
    pub fn new() -> Self {
        Self::with_config(BashConfig::default())
    }

    pub fn with_config(config: BashConfig) -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                apxm_core::constants::capabilities::BASH,
                "Execute shell commands with policy enforcement",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Shell command to run"
                        },
                        "timeout": {
                            "type": "integer",
                            "description": "Optional timeout in seconds"
                        }
                    },
                    "required": ["command"],
                    "additionalProperties": false
                }),
            )
            .with_returns("string")
            .with_groups(vec!["shell".to_string(), "exec".to_string()])
            .with_latency(250),
            config,
        }
    }

    pub fn safe() -> Self {
        Self::with_config(BashConfig {
            blocked_commands: vec![
                "rm -rf".to_string(),
                "rm -r".to_string(),
                "sudo".to_string(),
                "su ".to_string(),
                "mkfs".to_string(),
                "fdisk".to_string(),
                "dd if=".to_string(),
            ],
            ..Default::default()
        })
    }

    pub fn build() -> Self {
        Self::with_config(BashConfig {
            blocked_commands: vec!["sudo".to_string(), "su ".to_string(), "rm -rf".to_string()],
            timeout_secs: 600,
            ..Default::default()
        })
    }

    pub fn git() -> Self {
        Self::with_config(BashConfig {
            allowed_commands: Some(vec![
                "git status".to_string(),
                "git diff".to_string(),
                "git log".to_string(),
                "git show".to_string(),
                "git add".to_string(),
                "git reset".to_string(),
                "git commit".to_string(),
                "git push".to_string(),
                "git pull".to_string(),
                "git branch".to_string(),
                "git checkout".to_string(),
                "git switch".to_string(),
                "git merge".to_string(),
                "git rebase".to_string(),
                "git stash".to_string(),
                "git fetch".to_string(),
                "git remote".to_string(),
                "git clone".to_string(),
                "git rev-parse".to_string(),
                "git config --get".to_string(),
                "git config --list".to_string(),
            ]),
            ..Default::default()
        })
    }

    fn validate_command(&self, command: &str) -> CapabilityResult<()> {
        if let Some(allowed_commands) = &self.config.allowed_commands
            && !allowed_commands.is_empty()
        {
            let is_allowed = allowed_commands
                .iter()
                .any(|allowed| command_matches_allowed(command, allowed));
            if !is_allowed {
                return Err(RuntimeError::Capability {
                    capability: self.metadata.name.clone(),
                    message: format!(
                        "Command not allowed. Allowed commands: {}",
                        allowed_commands.join(", ")
                    ),
                });
            }
            return Ok(());
        }

        if let Some(blocked_pattern) = self
            .config
            .blocked_commands
            .iter()
            .find(|pattern| command.contains(pattern.as_str()))
        {
            return Err(RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!("Command blocked by policy: {blocked_pattern}"),
            });
        }

        Ok(())
    }

    fn timeout_secs(&self, args: &HashMap<String, Value>) -> u64 {
        args.get("timeout")
            .or_else(|| args.get("timeout_secs"))
            .and_then(|value| value.as_u64())
            .unwrap_or(self.config.timeout_secs)
    }

    fn working_directory(&self) -> Option<PathBuf> {
        self.config
            .working_directory
            .clone()
            .or_else(|| std::env::current_dir().ok())
    }

    fn build_exec_request(&self, args: &HashMap<String, Value>) -> CapabilityResult<ExecRequest> {
        let command = require_string_arg(args, "command", &self.metadata.name)?.to_string();
        self.validate_command(&command)?;

        let working_dir = self.working_directory();
        let read_paths: Vec<PathBuf> = working_dir.iter().cloned().collect();
        let write_paths: Vec<PathBuf> = working_dir.iter().cloned().collect();

        Ok(ExecRequest {
            min_isolation: IsolationLevel::OsLevel,
            program: executables::SHELL.to_string(),
            args: vec![shell_args::LOGIN_COMMAND.to_string(), command],
            working_dir,
            timeout: Duration::from_secs(self.timeout_secs(args)),
            max_output_bytes: self.config.max_output_bytes,
            read_paths,
            write_paths,
            needs_network: false,
            needs_process_spawn: true,
            origin_op: Some(AISOperationType::InvTool.to_string()),
            ..ExecRequest::default()
        })
    }

    fn truncate_output(stdout: &mut String, stderr: &mut String, max_output_bytes: usize) {
        let mut remaining = max_output_bytes;
        truncate_string_in_place(stdout, &mut remaining);
        truncate_string_in_place(stderr, &mut remaining);
    }
}

impl Default for BashCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityExecutor for BashCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let exec_request = self.build_exec_request(&args)?;
        let mut command_process = Command::new(&exec_request.program);
        command_process
            .args(&exec_request.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(working_directory) = &exec_request.working_dir {
            command_process.current_dir(working_directory);
        }

        let start = Instant::now();
        let output = tokio::time::timeout(exec_request.timeout, command_process.output())
            .await
            .map_err(|_| RuntimeError::Timeout {
                op_id: 0,
                timeout: exec_request.timeout,
            })?
            .map_err(|error| RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!("Failed to execute command: {error}"),
            })?;

        let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        Self::truncate_output(&mut stdout, &mut stderr, exec_request.max_output_bytes);

        Ok(exec_result_to_value(ExecResult {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout,
            stderr,
            duration: start.elapsed(),
            timed_out: false,
        }))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }

    fn to_exec_request(&self, args: &HashMap<String, Value>) -> Option<ExecRequest> {
        self.build_exec_request(args).ok()
    }
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

fn command_matches_allowed(command: &str, allowed: &str) -> bool {
    if command == allowed {
        return true;
    }
    let Some(suffix) = command.strip_prefix(allowed) else {
        return false;
    };
    if !suffix
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, ' ' | '\t'))
    {
        return false;
    }
    !has_disallowed_shell_syntax(suffix)
}

fn has_disallowed_shell_syntax(value: &str) -> bool {
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for ch in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single_quote => escaped = true,
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            '`' | '$' => return true,
            ';' | '|' | '&' | '<' | '>' | '(' | ')' | '\n' | '\r'
                if !in_single_quote && !in_double_quote =>
            {
                return true;
            }
            _ => {}
        }
    }

    in_single_quote || in_double_quote || escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_commands_accept_arguments_after_whitespace() {
        let capability = BashCapability::git();

        for command in [
            "git status --short",
            "git commit -m 'half;half'",
            "git commit -m \"docs: mention foo|bar\"",
        ] {
            capability
                .validate_command(command)
                .expect("git command arguments should be allowed");
        }
    }

    #[test]
    fn allowed_commands_reject_shell_control_suffixes() {
        let capability = BashCapability::git();

        for command in [
            "git status; echo bypass",
            "git status ; echo bypass",
            "git status && echo bypass",
            "git status | cat",
        ] {
            let error = capability
                .validate_command(command)
                .expect_err("shell control suffix should not match allowed command prefix");

            assert!(
                error.to_string().contains("Command not allowed"),
                "expected allowlist rejection for {command:?}: {error}"
            );
        }
    }

    #[test]
    fn command_args_cannot_request_network_access() {
        let capability = BashCapability::safe();
        let args = HashMap::from([
            (
                "command".to_string(),
                Value::String("echo hello".to_string()),
            ),
            ("needs_network".to_string(), Value::Bool(true)),
        ]);

        let request = capability
            .build_exec_request(&args)
            .expect("exec request should be built");

        assert!(
            !request.needs_network,
            "network access must be host policy, not tool-argument controlled"
        );
    }
}
