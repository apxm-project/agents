//! Profile-specific ContextStack scope rules.

use apxm_core::agent_profile::AgentProfile;
use apxm_core::constants::runtime::context_stack as context_stack_consts;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeRules {
    pub upstream_depth: usize,
    pub upstream_frame_budget: usize,
    pub session_frame_budget: usize,
    pub include_upstream_prompts: bool,
}

impl ScopeRules {
    /// Derive scope rules from a resolved AgentProfile.
    pub fn from_agent_profile(profile: &AgentProfile) -> Self {
        Self {
            upstream_depth: profile.upstream_depth,
            upstream_frame_budget: profile.upstream_frame_budget,
            session_frame_budget:
                apxm_core::constants::runtime::context_stack::DEFAULT_SESSION_FRAME_BUDGET_TOKENS,
            include_upstream_prompts: profile.include_upstream_prompts,
        }
    }

    pub fn for_profile(profile: &str) -> Self {
        let reg = apxm_core::agent_profile::AgentProfileRegistry::new();
        if let Some(p) = reg.resolve(profile) {
            Self::from_agent_profile(p)
        } else {
            Self::default()
        }
    }
}

impl Default for ScopeRules {
    fn default() -> Self {
        Self {
            upstream_depth: context_stack_consts::DEFAULT_UPSTREAM_DEPTH,
            upstream_frame_budget: context_stack_consts::DEFAULT_UPSTREAM_FRAME_BUDGET_TOKENS,
            session_frame_budget: context_stack_consts::DEFAULT_SESSION_FRAME_BUDGET_TOKENS,
            include_upstream_prompts: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_rules_for_profiles_match_defaults() {
        let architect = ScopeRules::for_profile("architect");
        assert_eq!(architect.upstream_depth, 3);

        let coder = ScopeRules::for_profile("coder");
        assert_eq!(coder.upstream_depth, 2);

        let reviewer = ScopeRules::for_profile("reviewer");
        assert_eq!(reviewer.upstream_depth, usize::MAX);
        assert!(reviewer.include_upstream_prompts);

        let unknown = ScopeRules::for_profile("unknown");
        assert_eq!(unknown, ScopeRules::default());
    }
}
