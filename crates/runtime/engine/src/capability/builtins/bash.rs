use super::require_string_arg;
use crate::capability::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::RuntimeCapability,
};
use crate::sandbox::constants::{executables, shell_args};
use crate::sandbox::{ExecRequest, IsolationLevel};
use apxm_core::{
    error::RuntimeError,
    types::{AISOperationType, Value},
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};
use tokio::time::Duration;

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
    metadata: RuntimeCapability,
    config: BashConfig,
}

impl BashCapability {
    pub fn new() -> Self {
        Self::with_config(BashConfig::default())
    }

    pub fn with_config(config: BashConfig) -> Self {
        Self {
            metadata: RuntimeCapability::new(
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
            origin_op: Some(AISOperationType::InvCap.to_string()),
            ..ExecRequest::default()
        })
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
    async fn execute(&self, _args: HashMap<String, Value>) -> CapabilityResult<Value> {
        // bash is a process-spawning capability: it declares an ExecRequest via
        // `to_exec_request`, so `CapabilitySystem` always routes it through the
        // sandbox registry and never calls this method. Refuse direct execution
        // so a stray caller can never spawn bash outside the sandbox.
        Err(RuntimeError::Capability {
            capability: self.metadata.name.clone(),
            message: "bash must be invoked through the sandbox (to_exec_request path), \
                      not executed directly"
                .to_string(),
        })
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }

    fn to_exec_request(&self, args: &HashMap<String, Value>) -> Option<ExecRequest> {
        self.build_exec_request(args).ok()
    }
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
    // Match on unquoted shell-word boundaries (+ basename) so quoting tricks
    // (`s""udo`) and path-qualified forms (`/usr/bin/sudo`) are caught and
    // incidental substrings aren't. Defense-in-depth — the write boundary is the
    // real gate (command substitution can evade any shell denylist).
    shell_word_segments_for_policy(command)
        .iter()
        .flatten()
        .any(|word| word == blocked || word.rsplit('/').next() == Some(blocked))
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
