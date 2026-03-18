//! Optimization related types.

use std::path::PathBuf;

use crate::error::runtime::RuntimeError;
use serde::{Deserialize, Serialize};

/// Optimization level for compilation
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize
)]
#[repr(u8)]
#[derive(Default)]
pub enum OptimizationLevel {
    /// No optimization, fastest compilation
    O0 = 0,
    /// Basic optimizations
    O1 = 1,
    /// Standard optimizations (default)
    #[default]
    O2 = 2,
    /// Aggressive optimizations
    O3 = 3,
}

impl std::str::FromStr for OptimizationLevel {
    type Err = RuntimeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "o0" | "0" | "none" => Ok(Self::O0),
            "o1" | "1" | "basic" => Ok(Self::O1),
            "o2" | "2" | "standard" => Ok(Self::O2),
            "o3" | "3" | "aggressive" => Ok(Self::O3),
            _ => Err(RuntimeError::Serialization(format!(
                "Invalid optimization level: {}",
                s
            ))),
        }
    }
}

impl std::fmt::Display for OptimizationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::O0 => write!(f, "O0"),
            Self::O1 => write!(f, "O1"),
            Self::O2 => write!(f, "O2"),
            Self::O3 => write!(f, "O3"),
        }
    }
}

/// Pipeline configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Optimization level
    pub opt_level: OptimizationLevel,

    /// Verify module before and after passes
    pub verify: bool,

    /// Skip CSE (Common Subexpression Elimination) for LLM operations.
    #[serde(default)]
    pub no_cse_llm: bool,

    /// Optional path to a profile JSON file for profile-guided optimization.
    ///
    /// When set, the compiler loads the [`ExecutionProfile`] and annotates graph
    /// nodes with observed latency, error-rate, and token-usage data before MLIR
    /// lowering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_path: Option<PathBuf>,

    /// Optional token budget for profile-guided warnings.
    ///
    /// When profile data is loaded, nodes whose average token consumption
    /// exceeds this budget receive a `__profile_token_warning` attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            opt_level: OptimizationLevel::O2,
            verify: true,
            no_cse_llm: false,
            profile_path: None,
            token_budget: None,
        }
    }
}

impl PipelineConfig {
    /// Create configuration for development (debugging enabled, O0)
    pub fn development() -> Self {
        Self {
            opt_level: OptimizationLevel::O0,
            verify: true,
            no_cse_llm: false,
            profile_path: None,
            token_budget: None,
        }
    }

    /// Create configuration for production (O3, verification disabled for performance)
    pub fn production() -> Self {
        Self {
            opt_level: OptimizationLevel::O3,
            verify: false,
            no_cse_llm: false,
            profile_path: None,
            token_budget: None,
        }
    }

    /// Builder: Set optimization level
    pub fn with_opt_level(mut self, level: OptimizationLevel) -> Self {
        self.opt_level = level;
        self
    }

    /// Builder: Enable/disable verification
    pub fn with_verify(mut self, enable: bool) -> Self {
        self.verify = enable;
        self
    }

    /// Builder: Set profile path for profile-guided optimization
    pub fn with_profile(mut self, path: PathBuf) -> Self {
        self.profile_path = Some(path);
        self
    }

    /// Builder: Set token budget for profile-guided warnings
    pub fn with_token_budget(mut self, budget: u64) -> Self {
        self.token_budget = Some(budget);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opt_level_from_str() -> Result<(), RuntimeError> {
        assert_eq!("o0".parse::<OptimizationLevel>()?, OptimizationLevel::O0);
        assert_eq!("O2".parse::<OptimizationLevel>()?, OptimizationLevel::O2);
        assert_eq!(
            "aggressive".parse::<OptimizationLevel>()?,
            OptimizationLevel::O3
        );
        assert!("invalid".parse::<OptimizationLevel>().is_err());
        Ok(())
    }
}
