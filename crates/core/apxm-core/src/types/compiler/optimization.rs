//! Optimization related types.

use std::path::PathBuf;

use crate::error::runtime::RuntimeError;
use serde::{Deserialize, Serialize};

/// Optimization level for compilation
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[derive(Default)]
pub enum OptimizationLevel {
    /// Required normalization and executable lowering only; no optional optimization
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

/// Optimization target for compilation - what to optimize for
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptimizationTarget {
    /// Minimize end-to-end latency using production-safe scheduling hints
    Latency,
    /// Minimize LLM API cost using production-safe cleanup passes
    Cost,
    /// Minimize token usage using dead context elimination
    Tokens,
    /// Maximize parallel execution (aggressive scheduling)
    Parallelism,
    /// Balanced optimization (default)
    Balanced,
}

impl Default for OptimizationTarget {
    fn default() -> Self {
        Self::Balanced
    }
}

impl std::str::FromStr for OptimizationTarget {
    type Err = RuntimeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "latency" => Ok(Self::Latency),
            "cost" => Ok(Self::Cost),
            "tokens" => Ok(Self::Tokens),
            "parallelism" | "parallel" => Ok(Self::Parallelism),
            "balanced" => Ok(Self::Balanced),
            _ => Err(RuntimeError::Serialization(format!(
                "Invalid optimization target: {}. Valid targets: latency, cost, tokens, parallelism, balanced",
                s
            ))),
        }
    }
}

impl std::fmt::Display for OptimizationTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Latency => write!(f, "latency"),
            Self::Cost => write!(f, "cost"),
            Self::Tokens => write!(f, "tokens"),
            Self::Parallelism => write!(f, "parallelism"),
            Self::Balanced => write!(f, "balanced"),
        }
    }
}

/// Pipeline configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Optimization level
    pub opt_level: OptimizationLevel,

    /// Optimization target - what to optimize for
    #[serde(default)]
    pub target: OptimizationTarget,

    /// Verify module before and after passes
    pub verify: bool,

    /// Skip CSE (Common Subexpression Elimination) for LLM operations.
    ///
    /// This also filters explicit pass lists so deterministic-only experiments
    /// cannot accidentally enable LLM CSE when this guard is set.
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

    /// Optional APXM config file path used for compiler-owned optimization
    /// settings.
    ///
    /// The compiler interprets only compiler-specific sections from this file;
    /// runtime/frontend configuration remains outside the compiler pipeline
    /// contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiler_config_path: Option<PathBuf>,

    /// Enable the diagnostic `unconsumed-value-warning` pass.
    ///
    /// Off by default — the pass is purely diagnostic (no IR mutation) and
    /// pure overhead in normal compiles. Wired through CLI `--warn`.
    #[serde(default)]
    pub warn_unconsumed: bool,

    /// Pass names to drop from the materialized pass list (ablation studies).
    ///
    /// Filter is applied after the default list is built (or after
    /// `pass_list_override` is substituted). Empty = no filtering. Wired
    /// through CLI `--disable-pass <name>` (repeatable).
    #[serde(default)]
    pub disable_passes: Vec<String>,

    /// Replace the entire default pass list with this explicit sequence.
    ///
    /// When `Some`, `opt_level` / `target` / `warn_unconsumed`
    /// no longer determine pass selection — only ordering matters here.
    /// `no_cse_llm` and `disable_passes` still filter the override. Wired
    /// through CLI `--pass-list <a,b,c>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_list_override: Option<Vec<String>>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            opt_level: OptimizationLevel::O2,
            target: OptimizationTarget::Balanced,
            verify: true,
            no_cse_llm: false,
            profile_path: None,
            token_budget: None,
            compiler_config_path: None,
            warn_unconsumed: false,
            disable_passes: Vec::new(),
            pass_list_override: None,
        }
    }
}

impl PipelineConfig {
    /// Create configuration for development (debugging enabled, O0)
    pub fn development() -> Self {
        Self {
            opt_level: OptimizationLevel::O0,
            target: OptimizationTarget::Balanced,
            verify: true,
            no_cse_llm: false,
            profile_path: None,
            token_budget: None,
            compiler_config_path: None,
            warn_unconsumed: false,
            disable_passes: Vec::new(),
            pass_list_override: None,
        }
    }

    /// Create configuration for production (O3, verification disabled for performance)
    pub fn production() -> Self {
        Self {
            opt_level: OptimizationLevel::O3,
            target: OptimizationTarget::Balanced,
            verify: false,
            no_cse_llm: false,
            profile_path: None,
            token_budget: None,
            compiler_config_path: None,
            warn_unconsumed: false,
            disable_passes: Vec::new(),
            pass_list_override: None,
        }
    }

    /// Builder: Set optimization level
    pub fn with_opt_level(mut self, level: OptimizationLevel) -> Self {
        self.opt_level = level;
        self
    }

    /// Builder: Set optimization target
    pub fn with_target(mut self, target: OptimizationTarget) -> Self {
        self.target = target;
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

    /// Builder: Set APXM config path for compiler-owned optimization settings.
    pub fn with_compiler_config_path(mut self, path: PathBuf) -> Self {
        self.compiler_config_path = Some(path);
        self
    }
}
