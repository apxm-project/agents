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

const BLOCKED_COMMAND_RM_RECURSIVE: &str = "rm recursive";
const BLOCKED_COMMAND_RM_RECURSIVE_FORCE: &str = "rm recursive force";
const BLOCKED_COMMAND_SUDO: &str = "sudo";
const BLOCKED_COMMAND_SU: &str = "su ";
const BLOCKED_COMMAND_MKFS: &str = "mkfs";
const BLOCKED_COMMAND_FDISK: &str = "fdisk";
const BLOCKED_COMMAND_DD_INPUT: &str = "dd if=";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BashConfig {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default = "default_blocked_commands")]
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

fn default_blocked_commands() -> Vec<String> {
    safe_blocked_commands()
}

fn base_config() -> BashConfig {
    BashConfig {
        enabled: true,
        blocked_commands: Vec::new(),
        allowed_commands: None,
        working_directory: None,
        timeout_secs: default_timeout(),
        max_output_bytes: default_max_output(),
    }
}

fn safe_blocked_commands() -> Vec<String> {
    vec![
        BLOCKED_COMMAND_RM_RECURSIVE.to_string(),
        BLOCKED_COMMAND_SUDO.to_string(),
        BLOCKED_COMMAND_SU.to_string(),
        BLOCKED_COMMAND_MKFS.to_string(),
        BLOCKED_COMMAND_FDISK.to_string(),
        BLOCKED_COMMAND_DD_INPUT.to_string(),
    ]
}

impl Default for BashConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            blocked_commands: default_blocked_commands(),
            allowed_commands: None,
            working_directory: None,
            timeout_secs: default_timeout(),
            max_output_bytes: default_max_output(),
        }
    }
}

impl BashConfig {
    pub fn safe_preset() -> Self {
        Self {
            blocked_commands: safe_blocked_commands(),
            ..base_config()
        }
    }

    pub fn build_preset() -> Self {
        Self {
            blocked_commands: vec![
                BLOCKED_COMMAND_SUDO.to_string(),
                BLOCKED_COMMAND_SU.to_string(),
                BLOCKED_COMMAND_RM_RECURSIVE_FORCE.to_string(),
            ],
            timeout_secs: 600,
            ..base_config()
        }
    }

    pub fn git_preset() -> Self {
        Self {
            allowed_commands: Some(git_allowed_commands()),
            ..base_config()
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
        Self::with_config(BashConfig::safe_preset())
    }

    pub fn build() -> Self {
        Self::with_config(BashConfig::build_preset())
    }

    pub fn git() -> Self {
        Self::with_config(BashConfig::git_preset())
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
            .find(|pattern| command_matches_blocked(command, pattern))
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

fn git_allowed_commands() -> Vec<String> {
    [
        "git status",
        "git diff",
        "git log",
        "git show",
        "git add",
        "git reset",
        "git commit",
        "git push",
        "git pull",
        "git branch",
        "git checkout",
        "git switch",
        "git merge",
        "git rebase",
        "git stash",
        "git fetch",
        "git remote",
        "git clone",
        "git rev-parse",
        "git config --get",
        "git config --list",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
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

fn command_matches_blocked(command: &str, blocked: &str) -> bool {
    if blocked == BLOCKED_COMMAND_RM_RECURSIVE {
        return contains_recursive_rm(command);
    }
    if blocked == BLOCKED_COMMAND_RM_RECURSIVE_FORCE {
        return contains_recursive_force_rm(command);
    }
    command.contains(blocked)
}

fn contains_recursive_force_rm(command: &str) -> bool {
    contains_rm_matching(command, rm_args_have_recursive_force)
}

fn contains_recursive_rm(command: &str) -> bool {
    contains_rm_matching(command, rm_args_have_recursive)
}

fn contains_rm_matching(command: &str, predicate: impl Fn(&[String]) -> bool) -> bool {
    shell_word_segments_for_policy(command)
        .iter()
        .any(|tokens| {
            for (index, token) in tokens.iter().enumerate() {
                if !token_is_rm_command(token) {
                    continue;
                }
                if predicate(&tokens[index + 1..]) {
                    return true;
                }
            }
            false
        })
}

fn token_is_rm_command(token: &str) -> bool {
    token.rsplit('/').next() == Some("rm")
}

fn shell_word_segments_for_policy(command: &str) -> Vec<Vec<String>> {
    let mut segments = Vec::new();
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single_quote => escaped = true,
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            ch if ch.is_whitespace() && !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            ';' | '|' | '&' | '<' | '>' | '(' | ')' | '\n' | '\r'
                if !in_single_quote && !in_double_quote =>
            {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                if !words.is_empty() {
                    segments.push(std::mem::take(&mut words));
                }
            }
            _ => current.push(ch),
        }
    }

    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        words.push(current);
    }
    if !words.is_empty() {
        segments.push(words);
    }

    segments
}

fn rm_args_have_recursive(tokens: &[String]) -> bool {
    for token in tokens {
        if token == "--" {
            break;
        }
        if token == "--recursive" {
            return true;
        }
        if token.starts_with("--") || !token.starts_with('-') {
            continue;
        }
        if token.chars().skip(1).any(|ch| ch == 'r' || ch == 'R') {
            return true;
        }
    }
    false
}

fn rm_args_have_recursive_force(tokens: &[String]) -> bool {
    let mut recursive = false;
    let mut force = false;
    for token in tokens {
        if token == "--" {
            break;
        }
        if !token.starts_with('-') {
            continue;
        }
        if token == "--recursive" {
            recursive = true;
            continue;
        }
        if token == "--force" {
            force = true;
            continue;
        }
        if token.starts_with("--") {
            continue;
        }
        recursive |= token.chars().skip(1).any(|ch| ch == 'r' || ch == 'R');
        force |= token.chars().skip(1).any(|ch| ch == 'f');
    }
    recursive && force
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
    fn blocklist_rejects_recursive_force_rm_variants() {
        for command in [
            "rm -rf target",
            "rm -fr target",
            "rm -Rf target",
            "rm -r -f target",
            "rm -f -r target",
            "rm --recursive --force target",
            "command rm -rf target",
            "env rm -rf target",
            "/bin/rm -rf target",
            "true; rm -rf target",
            "printf ok && rm --recursive --force target",
            "cd target || rm -fr target",
            "find . -exec rm -rf {} \\;",
        ] {
            let error = BashCapability::build()
                .validate_command(command)
                .expect_err("recursive force rm variant should be blocked");

            assert!(
                error.to_string().contains("Command blocked by policy"),
                "expected blocklist rejection for {command:?}: {error}"
            );
        }
    }

    #[test]
    fn safe_and_default_reject_recursive_rm_without_force() {
        for capability in [BashCapability::safe(), BashCapability::new()] {
            for command in [
                "rm -r target",
                "rm -R target",
                "rm --recursive target",
                "command rm -r target",
                "env FOO=bar rm -R target",
                "/bin/rm -r target",
                "true; rm -r target",
                "printf ok && rm --recursive target",
                "xargs rm -r target",
            ] {
                let error = capability
                    .validate_command(command)
                    .expect_err("recursive rm variant should be blocked");

                assert!(
                    error.to_string().contains("Command blocked by policy"),
                    "expected blocklist rejection for {command:?}: {error}"
                );
            }
        }
    }

    #[test]
    fn default_bash_config_is_not_unrestricted() {
        let config = BashConfig::default();
        assert!(
            config
                .blocked_commands
                .contains(&BLOCKED_COMMAND_RM_RECURSIVE.to_string()),
            "default bash config should block recursive rm"
        );
        assert!(
            config
                .blocked_commands
                .contains(&BLOCKED_COMMAND_SUDO.to_string()),
            "default bash config should block sudo"
        );
    }

    #[test]
    fn bash_presets_express_expected_rm_policy() {
        let safe = BashConfig::safe_preset();
        let build = BashConfig::build_preset();

        assert!(
            safe.blocked_commands
                .contains(&BLOCKED_COMMAND_RM_RECURSIVE.to_string())
        );
        assert!(
            !safe
                .blocked_commands
                .contains(&BLOCKED_COMMAND_RM_RECURSIVE_FORCE.to_string()),
            "safe preset should use the broader recursive rm policy"
        );
        assert!(
            build
                .blocked_commands
                .contains(&BLOCKED_COMMAND_RM_RECURSIVE_FORCE.to_string()),
            "build preset should block recursive force rm"
        );
        assert!(
            !build
                .blocked_commands
                .contains(&BLOCKED_COMMAND_RM_RECURSIVE.to_string()),
            "build preset should not use the broader recursive rm policy"
        );
    }

    #[test]
    fn build_preset_allows_recursive_rm_without_force() {
        let capability = BashCapability::build();

        for command in ["rm -r target", "rm --recursive target"] {
            capability
                .validate_command(command)
                .expect("build preset should only block recursive rm when force is also used");
        }
    }

    #[test]
    fn rm_policy_stops_parsing_options_after_double_dash() {
        for capability in [BashCapability::safe(), BashCapability::build()] {
            capability
                .validate_command("rm -- -rf")
                .expect("operands after -- should not be treated as rm options");
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
