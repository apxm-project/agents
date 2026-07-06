use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use apxm_core::apxm_acp;
use apxm_runtime::{AGENT_ROUTE_CAPABILITIES, AgentRouteCandidate};

use crate::constants::{registry as reg_consts, timeouts};

/// Permission mode for handling agent reverse requests.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    /// Approve all file/terminal operations.
    ApproveAll,
    /// Approve reads/searches, require approval for writes/exec.
    #[default]
    ApproveReads,
    /// Deny all file/terminal operations.
    DenyAll,
}

impl std::str::FromStr for PermissionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use crate::constants::permission_modes;
        match s {
            s if s == permission_modes::APPROVE_ALL => Ok(Self::ApproveAll),
            s if s == permission_modes::APPROVE_READS => Ok(Self::ApproveReads),
            s if s == permission_modes::DENY_ALL => Ok(Self::DenyAll),
            other => Err(format!(
                "unknown permission mode: '{other}'. Use: {}, {}, {}",
                permission_modes::APPROVE_ALL,
                permission_modes::APPROVE_READS,
                permission_modes::DENY_ALL,
            )),
        }
    }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::constants::permission_modes;
        match self {
            Self::ApproveAll => f.write_str(permission_modes::APPROVE_ALL),
            Self::ApproveReads => f.write_str(permission_modes::APPROVE_READS),
            Self::DenyAll => f.write_str(permission_modes::DENY_ALL),
        }
    }
}

/// Configuration for a capability server provisioned to an agent session.
///
/// On the ACP wire these are rendered as `mcpServers` (the protocol's name),
/// but within APXM they represent **Capabilities** (C) granted to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// Profile describing how to spawn and interact with an ACP agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpAgentProfile {
    pub command: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_close_grace")]
    pub close_grace_ms: u64,
    #[serde(default = "default_session_timeout")]
    pub session_create_timeout_ms: u64,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub default_mode: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub route_capabilities: Vec<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Skip the system preamble turn (turn 0). Use for agents that read context
    /// from their cwd (AGENTS.md) instead of a preamble prompt.
    #[serde(default)]
    pub skip_preamble: bool,
    /// Capabilities provisioned to this agent (rendered as mcpServers on the ACP wire).
    #[serde(default)]
    pub capabilities: Vec<CapabilityServerConfig>,
    /// Run the agent (and any terminals it opens) under the host sandbox
    /// backend when one capable of confining a long-running child is available.
    /// Defaults off so existing profiles spawn unchanged; the driver only
    /// applies confinement when this is set and a capable backend exists.
    #[serde(default)]
    pub sandbox: bool,
}

fn default_close_grace() -> u64 {
    timeouts::DEFAULT_CLOSE_GRACE_MS
}
fn default_session_timeout() -> u64 {
    timeouts::DEFAULT_SESSION_TIMEOUT_MS
}

pub fn default_route_capabilities() -> Vec<String> {
    AGENT_ROUTE_CAPABILITIES
        .iter()
        .copied()
        .map(str::to_string)
        .collect()
}

/// Registry of ACP agent profiles.
///
/// **Templates** are the built-in profile definitions (read-only reference
/// data). Three are tuned, verified working-set profiles with known-good
/// commands, descriptions, route capabilities, and timeouts: `claude`,
/// `codex`, `gemini`. A fourth, [`reg_consts::CUSTOM_TEMPLATE_NAME`], is a
/// generic custom-command template: it carries no working command and exists
/// to document the shape of an [`AcpAgentProfile`] for any other
/// ACP-speaking CLI. Rather than hardcoding a Rust variant per CLI, those
/// agents are added through configuration — copy the custom template into
/// `~/.apxm/agents.toml` (or run `apxm agent add <name> --command "..."`)
/// with `command` set to that CLI's ACP invocation.
///
/// **User profiles** are entries in `~/.apxm/agents.toml`. Built-in templates
/// are also resolvable by name, and user profiles override templates with the
/// same name.
pub struct AgentRegistry {
    templates: BTreeMap<String, AcpAgentProfile>,
    user_profiles: BTreeMap<String, AcpAgentProfile>,
}

impl AgentRegistry {
    /// Load registry: built-in templates plus user profiles from agents.toml.
    pub fn load() -> Self {
        let templates = Self::build_templates();
        let mut user_profiles = BTreeMap::new();

        if let Some(path) = Self::user_config_path()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            if let Ok(user) = toml::from_str::<UserAgentsFile>(&content) {
                user_profiles = user.agents;
            } else {
                apxm_acp!(warn, path = %path.display(), "failed to parse agent config");
            }
        }

        for (name, profile) in &mut user_profiles {
            if let Some(template) = templates.get(name) {
                if profile.description.is_none() {
                    profile.description.clone_from(&template.description);
                }
                if profile.route_capabilities.is_empty() {
                    profile
                        .route_capabilities
                        .clone_from(&template.route_capabilities);
                }
            } else if profile.route_capabilities.is_empty() {
                profile.route_capabilities = default_route_capabilities();
            }
        }

        Self {
            templates,
            user_profiles,
        }
    }

    /// Build the built-in template set: the tuned working-set profiles
    /// (`claude`, `codex`, `gemini`) plus one generic custom-command
    /// template ([`reg_consts::CUSTOM_TEMPLATE_NAME`]).
    fn build_templates() -> BTreeMap<String, AcpAgentProfile> {
        let mut templates = BTreeMap::new();

        let dg = timeouts::DEFAULT_CLOSE_GRACE_MS;
        let dt = timeouts::DEFAULT_SESSION_TIMEOUT_MS;
        let default_route_capabilities = default_route_capabilities();
        let entries: &[(&str, &str, &str, u64, u64)] = &[
            (
                "claude",
                "npx -y @agentclientprotocol/claude-agent-acp@^0.24.2",
                "Claude Code ACP profile for repository analysis, edits, command execution, review, and workflow planning.",
                dg,
                timeouts::CLAUDE_SESSION_TIMEOUT_MS,
            ),
            (
                "codex",
                "npx -y @zed-industries/codex-acp@^0.16.0",
                "Codex ACP profile for coding, tests, code review, and workflow implementation.",
                dg,
                dt,
            ),
            (
                "gemini",
                "gemini --acp",
                "Gemini ACP profile for codebase analysis, implementation, and verification.",
                dg,
                timeouts::GEMINI_SESSION_TIMEOUT_MS,
            ),
        ];

        for &(name, cmd, description, grace, timeout) in entries {
            templates.insert(
                name.to_string(),
                AcpAgentProfile {
                    command: cmd.to_string(),
                    description: Some(description.to_string()),
                    close_grace_ms: grace,
                    session_create_timeout_ms: timeout,
                    permission_mode: PermissionMode::default(),
                    env: BTreeMap::new(),
                    default_mode: None,
                    default_model: None,
                    route_capabilities: default_route_capabilities.clone(),
                    system_prompt: None,
                    skip_preamble: false,
                    capabilities: Vec::new(),
                    sandbox: false,
                },
            );
        }

        templates.insert(
            reg_consts::CUSTOM_TEMPLATE_NAME.to_string(),
            custom_command_template(default_route_capabilities),
        );

        templates
    }

    /// Return the deterministic built-in template set without loading user config.
    pub fn builtin_templates() -> BTreeMap<String, AcpAgentProfile> {
        Self::build_templates()
    }

    /// Look up an agent profile by name.
    ///
    /// Checks user profiles first, then falls through to built-in templates.
    /// This allows `claude`, `codex`, etc. to work out of the box.
    pub fn get(&self, name: &str) -> Option<&AcpAgentProfile> {
        self.user_profiles
            .get(name)
            .or_else(|| self.templates.get(name))
    }

    /// List all agent profiles: user profiles first, then templates not already
    /// overridden. The bool indicates whether the returned entry is a built-in
    /// template rather than a user profile.
    pub fn list(&self) -> Vec<(String, &AcpAgentProfile, bool)> {
        let mut entries: Vec<(String, &AcpAgentProfile, bool)> = self
            .user_profiles
            .iter()
            .map(|(name, profile)| (name.clone(), profile, false))
            .collect();

        // Append templates not already overridden by a user profile.
        for (name, profile) in &self.templates {
            if !self.user_profiles.contains_key(name.as_str()) {
                entries.push((name.clone(), profile, true));
            }
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    /// List only user profiles.
    ///
    /// Built-in templates are spawnable through [`Self::get`], while this
    /// accessor returns only explicit user profile overrides and custom entries.
    pub fn user_profiles(&self) -> Vec<(String, &AcpAgentProfile)> {
        self.user_profiles
            .iter()
            .map(|(name, profile)| (name.clone(), profile))
            .collect()
    }

    /// Look up a built-in template by name.
    pub fn get_template(&self, name: &str) -> Option<&AcpAgentProfile> {
        self.templates.get(name)
    }

    /// List all built-in templates.
    pub fn list_templates(&self) -> Vec<(String, &AcpAgentProfile)> {
        self.templates
            .iter()
            .map(|(name, profile)| (name.clone(), profile))
            .collect()
    }

    /// Whether a name matches a built-in template.
    pub fn is_from_template(&self, name: &str) -> bool {
        self.templates.contains_key(name)
    }

    /// Return resolvable ACP profiles as route candidates for APXM runtime selection.
    pub fn route_candidates(&self) -> Vec<AgentRouteCandidate> {
        self.list()
            .into_iter()
            .filter_map(|(name, profile, from_template)| {
                let executable = resolvable_command_program(&profile.command)?;
                if !profile_runtime_dependencies_available(&name) {
                    return None;
                }
                Some(AgentRouteCandidate {
                    profile: name,
                    description: profile.description.clone(),
                    source: Some(
                        if from_template {
                            reg_consts::sources::TEMPLATE
                        } else {
                            reg_consts::sources::USER_PROFILE
                        }
                        .to_string(),
                    ),
                    executable,
                    capabilities: if profile.route_capabilities.is_empty() {
                        default_route_capabilities()
                    } else {
                        profile.route_capabilities.clone()
                    },
                    default_mode: profile.default_mode.clone(),
                    default_model: profile.default_model.clone(),
                })
            })
            .collect()
    }

    /// Add or override a user profile. Persists to `~/.apxm/agents.toml`.
    pub fn add(&mut self, name: String, profile: AcpAgentProfile) -> Result<(), std::io::Error> {
        self.user_profiles.insert(name, profile);
        self.persist_user_entries()
    }

    /// Remove a user profile.
    pub fn remove(&mut self, name: &str) -> Result<bool, std::io::Error> {
        if self.user_profiles.remove(name).is_some() {
            self.persist_user_entries()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn user_config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".apxm").join(reg_consts::AGENTS_FILENAME))
    }

    /// Persist all user profiles to user config.
    fn persist_user_entries(&self) -> Result<(), std::io::Error> {
        let path = Self::user_config_path()
            .ok_or_else(|| std::io::Error::other("cannot determine home directory"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = UserAgentsFile {
            agents: self.user_profiles.clone(),
        };
        let content = toml::to_string_pretty(&file)
            .map_err(|e| std::io::Error::other(format!("serialize: {e}")))?;

        std::fs::write(&path, format!("{}{content}", reg_consts::FILE_HEADER))?;
        Ok(())
    }
}

/// Build the generic custom-command template.
///
/// This is a config-driven stand-in for any ACP-speaking coding-agent CLI
/// that isn't part of the tuned working set (`claude`, `codex`, `gemini`).
/// It intentionally ships with an empty `command` — that's not a bug in the
/// template, it's the point: an empty command never resolves to a runnable
/// executable (see [`resolvable_command_program`]), so this entry can never
/// be selected as a live route candidate by accident. To actually spawn one
/// of these agents, set `command` via `apxm agent add <name> --command
/// "<cli> <acp-flag>"` or by copying this profile into `~/.apxm/agents.toml`
/// under a new name.
fn custom_command_template(route_capabilities: Vec<String>) -> AcpAgentProfile {
    AcpAgentProfile {
        command: String::new(),
        description: Some(
            "Generic custom-command profile for any ACP-speaking CLI outside \
             the tuned working set (claude, codex, gemini) — for example \
             copilot, pi, cursor, droid, kilocode, kimi, kiro, opencode, \
             qoder, qwen, trae, or iflow. Set `command` to that CLI's ACP \
             invocation via `apxm agent add <name> --command \"...\"`, or \
             copy this profile into ~/.apxm/agents.toml with `command` \
             filled in; no dedicated Rust template is required."
                .to_string(),
        ),
        close_grace_ms: timeouts::DEFAULT_CLOSE_GRACE_MS,
        session_create_timeout_ms: timeouts::DEFAULT_SESSION_TIMEOUT_MS,
        permission_mode: PermissionMode::default(),
        env: BTreeMap::new(),
        default_mode: None,
        default_model: None,
        route_capabilities,
        system_prompt: None,
        skip_preamble: false,
        capabilities: Vec::new(),
        sandbox: false,
    }
}

fn resolvable_command_program(command: &str) -> Option<String> {
    let parts = shell_words::split(command).ok()?;
    let program = parts
        .iter()
        .find(|part| !part.contains('=') && part.as_str() != "env")?;
    resolvable_program(program)
}

fn resolvable_program(program: &str) -> Option<String> {
    if program.contains('/') {
        return std::path::Path::new(program)
            .is_file()
            .then(|| program.to_string());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .any(|path| path.join(program).is_file())
            .then(|| program.to_string())
    })
}

/// Extra runtime dependencies (beyond the resolvable command program itself)
/// that a profile needs before it's a live route candidate.
///
/// Empty for the current working-set templates (`claude`, `codex`, `gemini`)
/// and the generic `custom` template, which resolve entirely through
/// [`resolvable_command_program`]. Kept as a hook rather than removed
/// outright: a user profile added via config may still need this if its
/// wrapper command (e.g. `npx some-cli`) resolves even when the underlying
/// tool isn't installed.
fn profile_runtime_dependencies_available(_profile: &str) -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct UserAgentsFile {
    #[serde(default)]
    agents: BTreeMap<String, AcpAgentProfile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_roster_is_trimmed_to_the_working_set_plus_one_custom_template() {
        let templates = AgentRegistry::builtin_templates();
        let mut names: Vec<&str> = templates.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec!["claude", "codex", "custom", "gemini"],
            "the working set (claude, codex, gemini) plus the generic custom \
             template should be the entire built-in roster — no more \
             hardcoded per-CLI variants"
        );
    }

    #[test]
    fn working_set_templates_have_real_resolvable_commands() {
        let templates = AgentRegistry::builtin_templates();
        for name in ["claude", "codex", "gemini"] {
            let profile = templates
                .get(name)
                .unwrap_or_else(|| panic!("missing {name}"));
            assert!(
                !profile.command.trim().is_empty(),
                "{name} must have a concrete command"
            );
        }
    }

    #[test]
    fn custom_template_has_no_command_and_is_never_a_route_candidate() {
        let templates = AgentRegistry::builtin_templates();
        let custom = templates
            .get(reg_consts::CUSTOM_TEMPLATE_NAME)
            .expect("custom template must exist");
        assert!(
            custom.command.is_empty(),
            "the generic custom template must not ship a runnable command"
        );
        assert!(resolvable_command_program(&custom.command).is_none());
    }

    #[test]
    fn custom_template_becomes_spawnable_once_a_command_is_configured() {
        let mut profile = AgentRegistry::builtin_templates()
            .remove(reg_consts::CUSTOM_TEMPLATE_NAME)
            .unwrap();
        // Simulate a user filling in the generic template for some other
        // ACP-speaking CLI, e.g. one of the twelve no longer hardcoded here.
        profile.command = "echo not-a-real-acp-cli".to_string();
        assert_eq!(
            resolvable_command_program(&profile.command),
            Some("echo".to_string())
        );
    }

    #[test]
    fn get_falls_through_to_templates_for_working_set_names() {
        let registry = AgentRegistry {
            templates: AgentRegistry::builtin_templates(),
            user_profiles: BTreeMap::new(),
        };
        assert!(registry.get("claude").is_some());
        assert!(registry.get("codex").is_some());
        assert!(registry.get("gemini").is_some());
        assert!(registry.get("custom").is_some());
        assert!(registry.get("does-not-exist").is_none());
    }
}
