//! Typed surface for the metrics system.
//!
//! [`MetricsLevel`] is the emission tier selected on the CLI. It determines
//! which in-flight observers run during execution.
//!
//! New emission tiers must be added here first; consumers reach them through
//! the typed enum, never through ad-hoc string literals.

use serde::{Deserialize, Serialize};

use crate::error::runtime::RuntimeError;

/// CLI-selectable metrics emission tier.
///
/// `Basic` emits steady-state aggregates only. `Detailed` additionally enables
/// in-flight observers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricsLevel {
    #[default]
    Basic,
    Detailed,
}

impl MetricsLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Detailed => "detailed",
        }
    }
}

impl std::str::FromStr for MetricsLevel {
    type Err = RuntimeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "basic" => Ok(Self::Basic),
            "detailed" => Ok(Self::Detailed),
            _ => Err(RuntimeError::Serialization(format!(
                "Invalid metrics level: {s}. Valid values: basic, detailed"
            ))),
        }
    }
}

impl std::fmt::Display for MetricsLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
