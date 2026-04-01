use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

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
}

fn default_close_grace() -> u64 {
    timeouts::DEFAULT_CLOSE_GRACE_MS
}
fn default_session_timeout() -> u64 {
    timeouts::DEFAULT_SESSION_TIMEOUT_MS
}

/// Registry of known ACP agent profiles: built-ins + user overrides.
pub struct AgentRegistry {
    profiles: BTreeMap<String, AgentProfile>,
    builtins: std::collections::HashSet<String>,
}

impl AgentRegistry {
    /// Load registry: merge built-in profiles with user overrides from
    /// `~/.apxm/agents.toml`.
    pub fn load() -> Self {
        let mut reg = Self::builtins();

        if let Some(path) = Self::user_config_path() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(user) = toml::from_str::<UserAgentsFile>(&content) {
                    for (name, profile) in user.agents {
                        reg.profiles.insert(name, profile);
                    }
                } else {
                    tracing::warn!("Failed to parse {}", path.display());
                }
            }
        }

        reg
    }

    /// Build the registry with only built-in profiles.
    fn builtins() -> Self {
        let mut profiles = BTreeMap::new();
        let mut builtin_names = std::collections::HashSet::new();

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
            builtin_names.insert(name.to_string());
            profiles.insert(
                name.to_string(),
                AgentProfile {
                    command: cmd.to_string(),
                    close_grace_ms: grace,
                    session_create_timeout_ms: timeout,
                    permission_mode: PermissionMode::default(),
                    env: BTreeMap::new(),
                },
            );
        }

        Self {
            profiles,
            builtins: builtin_names,
        }
    }

    pub fn get(&self, name: &str) -> Option<&AgentProfile> {
        self.profiles.get(name)
    }

    pub fn list(&self) -> Vec<(String, &AgentProfile, bool)> {
        self.profiles
            .iter()
            .map(|(name, profile)| {
                let is_builtin = self.builtins.contains(name);
                (name.clone(), profile, is_builtin)
            })
            .collect()
    }

    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtins.contains(name)
    }

    /// Add or override a user profile. Persists to `~/.apxm/agents.toml`.
    pub fn add(&mut self, name: String, profile: AgentProfile) -> Result<(), std::io::Error> {
        self.profiles.insert(name.clone(), profile.clone());
        self.persist_user_entries()
    }

    /// Remove a user-registered profile. Cannot remove builtins.
    pub fn remove(&mut self, name: &str) -> Result<bool, std::io::Error> {
        if self.builtins.contains(name) {
            return Ok(false);
        }
        if self.profiles.remove(name).is_some() {
            self.persist_user_entries()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn user_config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".apxm").join(reg_consts::AGENTS_FILENAME))
    }

    /// Persist only non-builtin entries to user config.
    fn persist_user_entries(&self) -> Result<(), std::io::Error> {
        let path = Self::user_config_path()
            .ok_or_else(|| std::io::Error::other("cannot determine home directory"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let user_entries: BTreeMap<String, AgentProfile> = self
            .profiles
            .iter()
            .filter(|(name, _)| !self.builtins.contains(name.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let file = UserAgentsFile {
            agents: user_entries,
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

    #[test]
    fn builtins_count() {
        let reg = AgentRegistry::builtins();
        assert_eq!(reg.profiles.len(), 16);
    }

    #[test]
    fn builtin_lookup() {
        let reg = AgentRegistry::builtins();
        let claude = reg.get("claude").unwrap();
        assert!(claude.command.contains("claude-agent-acp"));
        assert_eq!(claude.close_grace_ms, timeouts::DEFAULT_CLOSE_GRACE_MS);
        assert_eq!(
            claude.session_create_timeout_ms,
            timeouts::CLAUDE_SESSION_TIMEOUT_MS
        );
    }

    #[test]
    fn qoder_has_longer_grace() {
        let reg = AgentRegistry::builtins();
        let qoder = reg.get("qoder").unwrap();
        assert_eq!(qoder.close_grace_ms, timeouts::QODER_CLOSE_GRACE_MS);
    }

    #[test]
    fn is_builtin() {
        let reg = AgentRegistry::builtins();
        assert!(reg.is_builtin("claude"));
        assert!(!reg.is_builtin("my-custom-agent"));
    }

    #[test]
    fn list_returns_all() {
        let reg = AgentRegistry::builtins();
        let list = reg.list();
        assert_eq!(list.len(), 16);
        assert!(list.iter().all(|(_, _, b)| *b));
    }

    #[test]
    fn profile_toml_roundtrip() {
        let profile = AgentProfile {
            command: "my-agent --acp".to_string(),
            close_grace_ms: 200,
            session_create_timeout_ms: 10_000,
            permission_mode: PermissionMode::DenyAll,
            env: BTreeMap::new(),
        };
        let toml_str = toml::to_string_pretty(&profile).unwrap();
        let parsed: AgentProfile = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.command, "my-agent --acp");
        assert_eq!(parsed.permission_mode, PermissionMode::DenyAll);
    }
}
