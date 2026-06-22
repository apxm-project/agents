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
/// **Templates** are the 15 built-in profile definitions (read-only reference
/// data with known commands, descriptions, route capabilities, and timeouts).
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

        if let Some(path) = Self::user_config_path() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(user) = toml::from_str::<UserAgentsFile>(&content) {
                    user_profiles = user.agents;
                } else {
                    apxm_acp!(warn, path = %path.display(), "failed to parse agent config");
                }
            }
        }

        for (name, profile) in user_profiles.iter_mut() {
            if let Some(template) = templates.get(name) {
                if profile.description.is_none() {
                    profile.description = template.description.clone();
                }
                if profile.route_capabilities.is_empty() {
                    profile.route_capabilities = template.route_capabilities.clone();
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

    /// Build the 15 built-in template profiles.
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
            (
                "copilot",
                "copilot --acp --stdio",
                "GitHub Copilot ACP profile for coding assistance, repository edits, and review.",
                dg,
                dt,
            ),
            (
                "pi",
                "npx pi-acp@^0.0.22",
                "Pi ACP profile for general reasoning and coding-agent tasks when the Pi CLI is installed.",
                dg,
                dt,
            ),
            (
                "cursor",
                "cursor-agent acp",
                "Cursor agent ACP profile for codebase navigation, edits, and verification.",
                dg,
                dt,
            ),
            (
                "droid",
                "droid exec --output-format acp",
                "Droid ACP profile for repository tasks exposed through Droid's ACP output mode.",
                dg,
                dt,
            ),
            (
                "kilocode",
                "npx -y @kilocode/cli acp",
                "Kilo Code ACP profile for implementation, command execution, and verification tasks.",
                dg,
                dt,
            ),
            (
                "kimi",
                "kimi acp",
                "Kimi ACP profile for coding, analysis, and review tasks.",
                dg,
                dt,
            ),
            (
                "kiro",
                "kiro-cli-chat acp",
                "Kiro ACP profile for coding-agent tasks through kiro-cli-chat.",
                dg,
                dt,
            ),
            (
                "opencode",
                "npx -y opencode-ai acp",
                "OpenCode ACP profile for implementation, command execution, and code review.",
                dg,
                dt,
            ),
            (
                "qoder",
                "qodercli --acp",
                "Qoder ACP profile for repository implementation and review workflows.",
                timeouts::QODER_CLOSE_GRACE_MS,
                dt,
            ),
            (
                "qwen",
                "qwen --acp",
                "Qwen ACP profile for coding, analysis, workflow planning, and verification.",
                dg,
                dt,
            ),
            (
                "trae",
                "traecli acp serve",
                "Trae ACP profile for codebase implementation and review tasks.",
                dg,
                dt,
            ),
            (
                "iflow",
                "iflow --experimental-acp",
                "iFlow experimental ACP profile for repository analysis and implementation tasks.",
                dg,
                dt,
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

fn profile_runtime_dependencies_available(profile: &str) -> bool {
    let dependencies = match profile {
        "pi" => &["pi"][..],
        _ => &[][..],
    };
    dependencies
        .iter()
        .all(|program| resolvable_program(program).is_some())
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct UserAgentsFile {
    #[serde(default)]
    agents: BTreeMap<String, AcpAgentProfile>,
}
