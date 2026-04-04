//! Profile-specific ContextStack scope rules.

use apxm_core::constants::runtime::context_stack as context_stack_consts;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeRules {
    pub upstream_depth: usize,
    pub upstream_frame_budget: usize,
    pub session_frame_budget: usize,
    pub include_upstream_prompts: bool,
}

impl ScopeRules {
    pub fn for_profile(profile: &str) -> Self {
        if profile.eq_ignore_ascii_case(context_stack_consts::PROFILE_CLAUDE) {
            Self {
                upstream_depth: context_stack_consts::CLAUDE_UPSTREAM_DEPTH,
                upstream_frame_budget: context_stack_consts::CLAUDE_UPSTREAM_FRAME_BUDGET_TOKENS,
                session_frame_budget: context_stack_consts::DEFAULT_SESSION_FRAME_BUDGET_TOKENS,
                include_upstream_prompts: false,
            }
        } else if profile.eq_ignore_ascii_case(context_stack_consts::PROFILE_CODEX) {
            Self {
                upstream_depth: context_stack_consts::CODEX_UPSTREAM_DEPTH,
                upstream_frame_budget: context_stack_consts::CODEX_UPSTREAM_FRAME_BUDGET_TOKENS,
                session_frame_budget: context_stack_consts::DEFAULT_SESSION_FRAME_BUDGET_TOKENS,
                include_upstream_prompts: false,
            }
        } else if profile.eq_ignore_ascii_case(context_stack_consts::PROFILE_REVIEWER) {
            Self {
                upstream_depth: context_stack_consts::REVIEWER_UPSTREAM_DEPTH,
                upstream_frame_budget: context_stack_consts::REVIEWER_UPSTREAM_FRAME_BUDGET_TOKENS,
                session_frame_budget: context_stack_consts::DEFAULT_SESSION_FRAME_BUDGET_TOKENS,
                include_upstream_prompts: true,
            }
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
        let claude = ScopeRules::for_profile("claude");
        assert_eq!(
            claude.upstream_depth,
            context_stack_consts::CLAUDE_UPSTREAM_DEPTH
        );

        let codex = ScopeRules::for_profile("codex");
        assert_eq!(
            codex.upstream_depth,
            context_stack_consts::CODEX_UPSTREAM_DEPTH
        );

        let reviewer = ScopeRules::for_profile("reviewer");
        assert_eq!(reviewer.upstream_depth, usize::MAX);
        assert!(reviewer.include_upstream_prompts);

        let unknown = ScopeRules::for_profile("unknown");
        assert_eq!(unknown, ScopeRules::default());
    }
}
