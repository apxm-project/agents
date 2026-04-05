//! Declarative per-node capability profiles.
use serde::{Deserialize, Serialize};

/// Context-window scope policy for a profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BeliefScope {
    #[default]
    All,
    Keys(Vec<String>),
    Prefix(String),
    None,
}

/// Declarative capability and context profile for an agent node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub name: String,
    pub description: String,
    pub upstream_depth: usize,
    pub upstream_frame_budget: usize,
    pub include_upstream_prompts: bool,
    pub allowed_skills: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub constraints: Vec<String>,
    pub belief_read_scope: BeliefScope,
    pub belief_write_scope: BeliefScope,
}

impl AgentProfile {
    pub fn architect() -> Self {
        Self {
            name: "architect".to_string(),
            description: "System design and high-level planning".to_string(),
            upstream_depth: 3,
            upstream_frame_budget: 4000,
            include_upstream_prompts: false,
            allowed_skills: vec!["system_design".to_string(), "api_design".to_string()],
            allowed_tools: vec![],
            constraints: vec![
                "Stay at design/interface level".to_string(),
                "Do not write implementation code".to_string(),
            ],
            belief_read_scope: BeliefScope::All,
            belief_write_scope: BeliefScope::Keys(vec![
                "architecture".to_string(),
                "interfaces".to_string(),
                "design_decisions".to_string(),
            ]),
        }
    }

    pub fn coder() -> Self {
        Self {
            name: "coder".to_string(),
            description: "Implementation and code generation".to_string(),
            upstream_depth: 2,
            upstream_frame_budget: 3000,
            include_upstream_prompts: false,
            allowed_skills: vec![
                "code_gen".to_string(),
                "write_tests".to_string(),
                "debug".to_string(),
            ],
            allowed_tools: vec![
                "file_read".to_string(),
                "file_write".to_string(),
                "shell".to_string(),
                "git".to_string(),
            ],
            constraints: vec![
                "Follow the provided architecture".to_string(),
                "Write tests for new code".to_string(),
            ],
            belief_read_scope: BeliefScope::Keys(vec![
                "architecture".to_string(),
                "requirements".to_string(),
            ]),
            belief_write_scope: BeliefScope::Keys(vec!["implementation".to_string()]),
        }
    }

    pub fn reviewer() -> Self {
        Self {
            name: "reviewer".to_string(),
            description: "Code review and quality assurance".to_string(),
            upstream_depth: usize::MAX,
            upstream_frame_budget: 2000,
            include_upstream_prompts: true,
            allowed_skills: vec!["code_review".to_string()],
            allowed_tools: vec!["file_read".to_string()],
            constraints: vec![
                "Cannot modify code, only review".to_string(),
                "Must cite specific line numbers for issues".to_string(),
            ],
            belief_read_scope: BeliefScope::All,
            belief_write_scope: BeliefScope::Keys(vec!["review_results".to_string()]),
        }
    }
}

/// Registry of agent profiles. Checks built-ins first, then YAML files.
pub struct AgentProfileRegistry {
    builtins: Vec<AgentProfile>,
}

impl Default for AgentProfileRegistry {
    fn default() -> Self {
        Self {
            builtins: vec![
                AgentProfile::architect(),
                AgentProfile::coder(),
                AgentProfile::reviewer(),
            ],
        }
    }
}

impl AgentProfileRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a profile by name. Checks built-ins first.
    /// Returns None if not found (caller can fall back to defaults).
    pub fn resolve(&self, name: &str) -> Option<&AgentProfile> {
        self.builtins
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Resolve or return architect as the default profile.
    pub fn resolve_or_default<'a>(&'a self, name: &str) -> &'a AgentProfile {
        self.resolve(name).unwrap_or_else(|| &self.builtins[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_profiles_exist() {
        let reg = AgentProfileRegistry::new();
        assert!(reg.resolve("architect").is_some());
        assert!(reg.resolve("coder").is_some());
        assert!(reg.resolve("reviewer").is_some());
    }

    #[test]
    fn resolve_case_insensitive() {
        let reg = AgentProfileRegistry::new();
        assert!(reg.resolve("Claude").is_none()); // not a profile name
        assert!(reg.resolve("ARCHITECT").is_some());
    }

    #[test]
    fn resolve_unknown_returns_none() {
        let reg = AgentProfileRegistry::new();
        assert!(reg.resolve("nonexistent").is_none());
    }

    #[test]
    fn reviewer_has_max_depth() {
        let p = AgentProfile::reviewer();
        assert_eq!(p.upstream_depth, usize::MAX);
        assert!(p.include_upstream_prompts);
    }
}
