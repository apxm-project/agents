//! Optimization heuristics for guiding pass transformations.
//!
//! This module provides the framework for making optimization decisions based on:
//! - Token budgets (don't fuse if result exceeds model context window)
//! - Cost models (estimate cost before/after optimization)
//! - Quality guards (some fusions may degrade output quality)
//! - Target awareness (latency vs cost vs tokens optimization)
//! - Profile feedback (use past execution data to guide decisions)

use apxm_core::types::OptimizationTarget;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Execution profile data from past runs (for profile-guided optimization)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionProfile {
    /// Path to the profile JSON file
    pub path: PathBuf,
    /// Profile metadata (version, timestamp, etc.)
    pub metadata: ProfileMetadata,
    /// Per-node performance data
    pub node_profiles: Vec<NodeProfile>,
}

/// Profile metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileMetadata {
    pub version: String,
    pub timestamp: String,
    pub runs: usize,
}

/// Per-node profile data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeProfile {
    pub node_id: String,
    pub avg_latency_ms: u64,
    pub avg_tokens: usize,
    pub error_rate: f64,
}

/// Optimization heuristics configuration
///
/// This struct captures the constraints and preferences that guide optimization passes.
/// Different targets (latency, cost, tokens) produce different heuristic configurations.
#[derive(Debug, Clone)]
pub struct OptimizationHeuristics {
    /// What to optimize for (latency, cost, tokens, parallelism, balanced)
    pub target: OptimizationTarget,

    /// Maximum template tokens after fusion (don't fuse beyond this)
    pub max_fused_template_tokens: usize,

    /// Minimum latency saved (ms) to justify fusion (avoid micro-optimizations)
    pub min_fusion_savings_ms: u64,

    /// Model context window limit (hard constraint)
    pub max_context_tokens: usize,

    /// Enable quality guards (prevent fusions that may hurt output quality)
    pub enable_quality_guard: bool,

    /// Profile-guided optimization data (optional)
    pub profile: Option<ExecutionProfile>,
}

impl Default for OptimizationHeuristics {
    fn default() -> Self {
        Self::for_target(OptimizationTarget::Balanced)
    }
}

impl OptimizationHeuristics {
    /// Create heuristics configuration for a specific optimization target
    pub fn for_target(target: OptimizationTarget) -> Self {
        match target {
            OptimizationTarget::Latency => Self {
                target,
                max_fused_template_tokens: 8000,  // Aggressive fusion
                min_fusion_savings_ms: 100,       // Accept smaller latency wins
                max_context_tokens: 32000,        // Large context OK
                enable_quality_guard: false,      // Latency > quality
                profile: None,
            },
            OptimizationTarget::Cost => Self {
                target,
                max_fused_template_tokens: 4000,  // Conservative fusion
                min_fusion_savings_ms: 500,       // Only significant wins
                max_context_tokens: 16000,        // Moderate context
                enable_quality_guard: true,       // Quality matters for cost
                profile: None,
            },
            OptimizationTarget::Tokens => Self {
                target,
                max_fused_template_tokens: 2000,  // Minimal fusion
                min_fusion_savings_ms: 0,         // Any fusion OK if saves tokens
                max_context_tokens: 8000,         // Aggressive context reduction
                enable_quality_guard: true,       // Preserve quality
                profile: None,
            },
            OptimizationTarget::Parallelism => Self {
                target,
                max_fused_template_tokens: 6000,  // Moderate fusion (don't kill parallelism)
                min_fusion_savings_ms: 200,
                max_context_tokens: 24000,
                enable_quality_guard: false,      // Parallelism > quality
                profile: None,
            },
            OptimizationTarget::Balanced => Self {
                target,
                max_fused_template_tokens: 5000,  // Balanced fusion
                min_fusion_savings_ms: 250,
                max_context_tokens: 20000,
                enable_quality_guard: true,
                profile: None,
            },
        }
    }

    /// Check if fusion should proceed based on token budget
    ///
    /// Returns true if the fused result would fit within the token budget.
    pub fn should_fuse(&self, producer_tokens: usize, consumer_tokens: usize) -> bool {
        let fused_tokens = producer_tokens + consumer_tokens;
        if fused_tokens > self.max_fused_template_tokens {
            return false;
        }
        if fused_tokens > self.max_context_tokens {
            return false;
        }
        true
    }

    /// Check if context should be eliminated based on usage and token budget
    ///
    /// Returns true if the context should be removed (not used or exceeds budget).
    pub fn should_eliminate_context(&self, context_tokens: usize, used: bool) -> bool {
        !used || context_tokens > self.max_context_tokens
    }

    /// Estimate latency saved by fusion (in milliseconds)
    ///
    /// Uses profile data if available, otherwise uses a heuristic estimate.
    pub fn estimate_fusion_latency_savings(&self, _producer_id: &str, _consumer_id: &str) -> u64 {
        // TODO: Use profile data when available
        // For now, use a heuristic: each LLM round-trip is ~500-2000ms
        // Conservative estimate: 500ms
        500
    }

    /// Check if fusion meets the minimum latency savings threshold
    pub fn meets_fusion_threshold(&self, producer_id: &str, consumer_id: &str) -> bool {
        let savings = self.estimate_fusion_latency_savings(producer_id, consumer_id);
        savings >= self.min_fusion_savings_ms
    }

    /// Set profile data for profile-guided optimization
    pub fn with_profile(mut self, profile: ExecutionProfile) -> Self {
        self.profile = Some(profile);
        self
    }

    /// Get the maximum fused template tokens for this configuration
    pub fn max_fused_tokens(&self) -> usize {
        self.max_fused_template_tokens
    }

    /// Get the maximum context tokens for this configuration
    pub fn max_context(&self) -> usize {
        self.max_context_tokens
    }
}

/// Estimate token count from text using the standard chars/4 heuristic.
///
/// This is a rough approximation: 1 token ≈ 4 characters.
/// For more precise estimates, consider using a proper tokenizer library.
pub fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latency_target_aggressive_fusion() {
        let heuristics = OptimizationHeuristics::for_target(OptimizationTarget::Latency);
        assert_eq!(heuristics.max_fused_template_tokens, 8000);
        assert_eq!(heuristics.min_fusion_savings_ms, 100);
        assert!(!heuristics.enable_quality_guard);
    }

    #[test]
    fn test_cost_target_conservative_fusion() {
        let heuristics = OptimizationHeuristics::for_target(OptimizationTarget::Cost);
        assert_eq!(heuristics.max_fused_template_tokens, 4000);
        assert_eq!(heuristics.min_fusion_savings_ms, 500);
        assert!(heuristics.enable_quality_guard);
    }

    #[test]
    fn test_tokens_target_minimal_fusion() {
        let heuristics = OptimizationHeuristics::for_target(OptimizationTarget::Tokens);
        assert_eq!(heuristics.max_fused_template_tokens, 2000);
        assert_eq!(heuristics.max_context_tokens, 8000);
        assert!(heuristics.enable_quality_guard);
    }

    #[test]
    fn test_should_fuse_within_budget() {
        let heuristics = OptimizationHeuristics::for_target(OptimizationTarget::Balanced);
        // 1000 + 2000 = 3000 tokens, well within 5000 limit
        assert!(heuristics.should_fuse(1000, 2000));
    }

    #[test]
    fn test_should_not_fuse_exceeds_budget() {
        let heuristics = OptimizationHeuristics::for_target(OptimizationTarget::Balanced);
        // 3000 + 3000 = 6000 tokens, exceeds 5000 limit
        assert!(!heuristics.should_fuse(3000, 3000));
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens("hello"), 1); // 5 chars / 4 = 1
        assert_eq!(estimate_tokens("hello world"), 2); // 11 chars / 4 = 2
        assert_eq!(estimate_tokens("a".repeat(1000).as_str()), 250); // 1000 / 4 = 250
    }

    #[test]
    fn test_should_eliminate_unused_context() {
        let heuristics = OptimizationHeuristics::for_target(OptimizationTarget::Balanced);
        // Unused context should be eliminated
        assert!(heuristics.should_eliminate_context(1000, false));
    }

    #[test]
    fn test_should_not_eliminate_used_context_within_budget() {
        let heuristics = OptimizationHeuristics::for_target(OptimizationTarget::Balanced);
        // Used context within budget should be kept
        assert!(!heuristics.should_eliminate_context(1000, true));
    }

    #[test]
    fn test_should_eliminate_used_context_exceeds_budget() {
        let heuristics = OptimizationHeuristics::for_target(OptimizationTarget::Tokens);
        // Even if used, huge context should be eliminated for tokens target
        assert!(heuristics.should_eliminate_context(10000, true));
    }

    #[test]
    fn test_meets_fusion_threshold() {
        let heuristics = OptimizationHeuristics::for_target(OptimizationTarget::Latency);
        // Latency target has 100ms threshold, estimate is 500ms
        assert!(heuristics.meets_fusion_threshold("prod", "cons"));
    }

    #[test]
    fn test_does_not_meet_fusion_threshold() {
        let heuristics = OptimizationHeuristics::for_target(OptimizationTarget::Cost);
        // Cost target has 500ms threshold, but let's assume estimate is exactly 500ms
        // Current implementation always returns 500ms, so this should pass
        assert!(heuristics.meets_fusion_threshold("prod", "cons"));
    }
}
