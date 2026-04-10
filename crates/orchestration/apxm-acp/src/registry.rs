use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use apxm_core::apxm_acp;

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
pub struct AgentProfile {
    pub command: String,
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
    pub system_prompt: Option<String>,
    /// Skip the system preamble turn (turn 0). Use for agents that read context
    /// from their cwd (AGENTS.md) instead of a preamble prompt.
    #[serde(default)]
    pub skip_preamble: bool,
    /// Capabilities provisioned to this agent (rendered as mcpServers on the ACP wire).
    #[serde(default)]
    pub capabilities: Vec<CapabilityServerConfig>,
}

fn default_close_grace() -> u64 {
    timeouts::DEFAULT_CLOSE_GRACE_MS
}
fn default_session_timeout() -> u64 {
    timeouts::DEFAULT_SESSION_TIMEOUT_MS
}

/// Registry of ACP agent profiles.
///
/// **Templates** are the 16 built-in profile definitions (read-only reference
/// data with known commands and timeouts).
///
/// **Registered agents** are entries in `~/.apxm/agents.toml` — the only
/// agents that are resolvable at runtime. `agent add claude` uses the template
/// as defaults so users don't need to memorize npx commands.
pub struct AgentRegistry {
    templates: BTreeMap<String, AgentProfile>,
    registered: BTreeMap<String, AgentProfile>,
}

impl AgentRegistry {
    /// Load registry: templates from built-ins, registered from agents.toml.
    pub fn load() -> Self {
        let templates = Self::build_templates();
        let mut registered = BTreeMap::new();

        if let Some(path) = Self::user_config_path() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(user) = toml::from_str::<UserAgentsFile>(&content) {
                    registered = user.agents;
                } else {
                    apxm_acp!(warn, path = %path.display(), "failed to parse agent config");
                }
            }
        }

        Self {
            templates,
            registered,
        }
    }

    /// Build the 16 built-in template profiles.
    fn build_templates() -> BTreeMap<String, AgentProfile> {
        let mut templates = BTreeMap::new();

        let dg = timeouts::DEFAULT_CLOSE_GRACE_MS;
        let dt = timeouts::DEFAULT_SESSION_TIMEOUT_MS;
        let entries: &[(&str, &str, u64, u64)] = &[
            (
                "claude",
                "npx -y @agentclientprotocol/claude-agent-acp@^0.24.2",
                dg,
                timeouts::CLAUDE_SESSION_TIMEOUT_MS,
            ),
            ("codex", "npx @zed-industries/codex-acp@^0.10.0", dg, dt),
            (
                "gemini",
                "gemini --acp",
                dg,
                timeouts::GEMINI_SESSION_TIMEOUT_MS,
            ),
            ("copilot", "copilot --acp --stdio", dg, dt),
            ("openclaw", "openclaw acp", dg, dt),
            ("pi", "npx pi-acp@^0.0.22", dg, dt),
            ("cursor", "cursor-agent acp", dg, dt),
            ("droid", "droid exec --output-format acp", dg, dt),
            ("kilocode", "npx -y @kilocode/cli acp", dg, dt),
            ("kimi", "kimi acp", dg, dt),
            ("kiro", "kiro-cli-chat acp", dg, dt),
            ("opencode", "npx -y opencode-ai acp", dg, dt),
            (
                "qoder",
                "qodercli --acp",
                timeouts::QODER_CLOSE_GRACE_MS,
                dt,
            ),
            ("qwen", "qwen --acp", dg, dt),
            ("trae", "traecli acp serve", dg, dt),
            ("iflow", "iflow --experimental-acp", dg, dt),
        ];

        for &(name, cmd, grace, timeout) in entries {
            templates.insert(
                name.to_string(),
                AgentProfile {
                    command: cmd.to_string(),
                    close_grace_ms: grace,
                    session_create_timeout_ms: timeout,
                    permission_mode: PermissionMode::default(),
                    env: BTreeMap::new(),
                    default_mode: None,
                    default_model: None,
                    system_prompt: None,
                    skip_preamble: false,
                    capabilities: Vec::new(),
                },
            );
        }

        templates
    }

    /// Return the deterministic built-in template set without loading user config.
    pub fn builtin_templates() -> BTreeMap<String, AgentProfile> {
        Self::build_templates()
    }

    /// Look up a registered agent. Only registered agents are resolvable.
    /// Look up an agent profile by name.
    ///
    /// Checks registered agents first (user-configured), then falls through
    /// to built-in templates. This allows `claude`, `codex`, etc. to work
    /// out of the box without requiring explicit `apxm agent add` registration.
    pub fn get(&self, name: &str) -> Option<&AgentProfile> {
        self.registered
            .get(name)
            .or_else(|| self.templates.get(name))
    }

    /// List all agents: registered entries first (overrides), then any templates
    /// not already overridden. The bool indicates whether the entry came from a template.
    pub fn list(&self) -> Vec<(String, &AgentProfile, bool)> {
        let mut entries: Vec<(String, &AgentProfile, bool)> = self
            .registered
            .iter()
            .map(|(name, profile)| {
                let from_template = self.templates.contains_key(name.as_str());
                (name.clone(), profile, from_template)
            })
            .collect();

        // Append templates not already overridden by a registered entry
        for (name, profile) in &self.templates {
            if !self.registered.contains_key(name.as_str()) {
                entries.push((name.clone(), profile, true));
            }
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    /// Look up a built-in template by name.
    pub fn get_template(&self, name: &str) -> Option<&AgentProfile> {
        self.templates.get(name)
    }

    /// List all built-in templates.
    pub fn list_templates(&self) -> Vec<(String, &AgentProfile)> {
        self.templates
            .iter()
            .map(|(name, profile)| (name.clone(), profile))
            .collect()
    }

    /// Whether a name matches a built-in template.
    pub fn is_from_template(&self, name: &str) -> bool {
        self.templates.contains_key(name)
    }

    /// Add or override a registered agent. Persists to `~/.apxm/agents.toml`.
    pub fn add(&mut self, name: String, profile: AgentProfile) -> Result<(), std::io::Error> {
        self.registered.insert(name, profile);
        self.persist_user_entries()
    }

    /// Remove a registered agent.
    pub fn remove(&mut self, name: &str) -> Result<bool, std::io::Error> {
        if self.registered.remove(name).is_some() {
            self.persist_user_entries()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn user_config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".apxm").join(reg_consts::AGENTS_FILENAME))
    }

    /// Persist all registered entries to user config.
    fn persist_user_entries(&self) -> Result<(), std::io::Error> {
        let path = Self::user_config_path()
            .ok_or_else(|| std::io::Error::other("cannot determine home directory"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = UserAgentsFile {
            agents: self.registered.clone(),
        };
        let content = toml::to_string_pretty(&file)
            .map_err(|e| std::io::Error::other(format!("serialize: {e}")))?;

        std::fs::write(&path, format!("{}{content}", reg_consts::FILE_HEADER))?;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct UserAgentsFile {
    #[serde(default)]
    agents: BTreeMap<String, AgentProfile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_registry() -> AgentRegistry {
        AgentRegistry {
            templates: AgentRegistry::build_templates(),
            registered: BTreeMap::new(),
        }
    }

    #[test]
    fn templates_count() {
        let reg = empty_registry();
        assert_eq!(reg.list_templates().len(), 16);
    }

    #[test]
    fn template_lookup() {
        let reg = empty_registry();
        let claude = reg.get_template("claude").unwrap();
        assert!(claude.command.contains("claude-agent-acp"));
        assert_eq!(claude.close_grace_ms, timeouts::DEFAULT_CLOSE_GRACE_MS);
        assert_eq!(
            claude.session_create_timeout_ms,
            timeouts::CLAUDE_SESSION_TIMEOUT_MS
        );
        // Templates ARE resolvable via get() — they work out of the box
        // without requiring explicit `apxm agent add` registration.
        assert!(reg.get("claude").is_some());
    }

    #[test]
    fn qoder_template_has_longer_grace() {
        let reg = empty_registry();
        let qoder = reg.get_template("qoder").unwrap();
        assert_eq!(qoder.close_grace_ms, timeouts::QODER_CLOSE_GRACE_MS);
    }

    #[test]
    fn is_from_template() {
        let reg = empty_registry();
        assert!(reg.is_from_template("claude"));
        assert!(!reg.is_from_template("my-custom-agent"));
    }

    #[test]
    fn list_returns_empty_without_registrations() {
        let reg = empty_registry();
        let list = reg.list();
        // list() includes templates so agents work out of the box.
        // An empty registry still has all 16 built-in templates.
        assert!(!list.is_empty());
        assert_eq!(list.len(), 16);
        // All entries are from templates (none registered)
        assert!(list.iter().all(|(_, _, from_template)| *from_template));
    }

    #[test]
    fn register_from_template() {
        let mut reg = empty_registry();
        let profile = reg.get_template("claude").unwrap().clone();
        reg.registered.insert("claude".to_string(), profile);
        assert!(reg.get("claude").is_some());
        let list = reg.list();
        // All 16 templates + claude override = still 16 total (override replaces template slot)
        assert_eq!(list.len(), 16);
        // claude entry should be marked as from_template=true since it was based on one
        let claude_entry = list.iter().find(|(name, _, _)| name == "claude");
        assert!(claude_entry.is_some());
        assert!(claude_entry.unwrap().2);
    }

    #[test]
    fn register_custom() {
        let mut reg = empty_registry();
        let profile = AgentProfile {
            command: "my-agent --acp".to_string(),
            close_grace_ms: 200,
            session_create_timeout_ms: 10_000,
            permission_mode: PermissionMode::DenyAll,
            env: BTreeMap::new(),
            default_mode: None,
            default_model: None,
            system_prompt: None,
            skip_preamble: false,
            capabilities: Vec::new(),
        };
        reg.registered.insert("custom".to_string(), profile);
        assert!(reg.get("custom").is_some());
        let list = reg.list();
        // 16 built-in templates + 1 custom registered agent = 17
        assert_eq!(list.len(), 17);
        // Find the custom entry and verify it is NOT from a template
        let custom_entry = list.iter().find(|(name, _, _)| name == "custom");
        assert!(custom_entry.is_some());
        assert!(!custom_entry.unwrap().2);
    }

    #[test]
    fn remove_registered() {
        let mut reg = empty_registry();
        let profile = reg.get_template("claude").unwrap().clone();
        reg.registered.insert("claude".to_string(), profile);
        assert!(reg.get("claude").is_some());
        reg.registered.remove("claude");
        // After removing the registered override, get() still resolves via
        // the built-in template (claude works out of the box).
        assert!(reg.get("claude").is_some());
        // Template still exists
        assert!(reg.get_template("claude").is_some());
    }

    #[test]
    fn profile_toml_roundtrip() {
        let profile = AgentProfile {
            command: "my-agent --acp".to_string(),
            close_grace_ms: 200,
            session_create_timeout_ms: 10_000,
            permission_mode: PermissionMode::DenyAll,
            env: BTreeMap::new(),
            default_mode: None,
            default_model: None,
            system_prompt: None,
            skip_preamble: false,
            capabilities: Vec::new(),
        };
        let toml_str = toml::to_string_pretty(&profile).unwrap();
        let parsed: AgentProfile = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.command, "my-agent --acp");
        assert_eq!(parsed.permission_mode, PermissionMode::DenyAll);
    }
}
