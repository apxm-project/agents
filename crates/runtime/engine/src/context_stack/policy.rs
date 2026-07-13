//! Profile-specific ContextStack scope rules.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::frame::ContextTokenizer;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeRules {
    pub upstream_depth: usize,
    pub upstream_frame_budget: usize,
    pub session_frame_budget: usize,
    pub include_upstream_prompts: bool,
}

/// Explicit planning inputs for context assembly.
///
/// The configured tokenizer, capacity, and profile rules are the complete
/// authority for context packing. Missing profile evidence does not select a
/// built-in profile or capacity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPlanningPolicy {
    pub tokenizer: ContextTokenizer,
    pub token_budget: usize,
    pub profiles: BTreeMap<String, ScopeRules>,
}

impl ContextPlanningPolicy {
    pub fn rules_for(&self, profile: &str) -> Result<&ScopeRules, ContextPlanningError> {
        self.profiles
            .get(profile)
            .ok_or_else(|| ContextPlanningError::ProfileNotConfigured {
                profile: profile.to_string(),
            })
    }
}

/// Context assembly cannot proceed when the configured policy does not prove
/// that the requested profile is available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextPlanningError {
    PolicyNotConfigured,
    ProfileNotSpecified,
    ProfileNotConfigured {
        profile: String,
    },
    ProtectedSegmentExceedsBudget {
        provenance: String,
        required_tokens: usize,
        available_tokens: usize,
    },
}

impl fmt::Display for ContextPlanningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PolicyNotConfigured => {
                write!(f, "context planning policy is not configured")
            }
            Self::ProfileNotSpecified => {
                write!(f, "context profile is not specified")
            }
            Self::ProfileNotConfigured { profile } => {
                write!(f, "context profile is not configured: {profile}")
            }
            Self::ProtectedSegmentExceedsBudget {
                provenance,
                required_tokens,
                available_tokens,
            } => write!(
                f,
                "protected context segment '{provenance}' requires {required_tokens} tokens but only {available_tokens} are available"
            ),
        }
    }
}

impl std::error::Error for ContextPlanningError {}
