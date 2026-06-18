//! Typed surface for the metrics system.
//!
//! Two pieces live here:
//!
//! - [`MetricsLevel`]: emission tier selected on the CLI. Determines which
//!   in-flight observers run during execution.
//! - [`GraphStatusKey`]: typed wire keys for `GraphStatusSnapshot::to_metrics_json`.
//!   The string constants in `crate::constants::session::metrics_keys::graph_status_keys`
//!   are derived from these variants so the wire format stays single-sourced.
//!
//! New emission tiers and new wire keys must be added here first; consumers
//! reach them through the typed enum, never through ad-hoc string literals.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::error::runtime::RuntimeError;

/// CLI-selectable metrics emission tier.
///
/// `Basic` emits steady-state aggregates only. `Detailed` additionally enables
/// in-flight observers (currently: per-graph pin-peak polling); the suggested
/// poll cadence is exposed via [`Self::pin_poll_interval`].
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

    /// Pin-poll cadence for the executor's `start_pin_polling` task.
    /// `None` disables polling.
    pub const fn pin_poll_interval(self) -> Option<Duration> {
        match self {
            Self::Basic => None,
            Self::Detailed => Some(Duration::from_millis(50)),
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

/// Wire keys serialized by `GraphStatusSnapshot::to_metrics_json`.
///
/// Each variant is the single source of truth for one JSON field name.
/// Constants in `constants::session::metrics_keys::graph_status_keys` re-export
/// these values for shared metric serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GraphStatusKey {
    Object,
    BackendKind,
    BackendName,
    GraphId,
    Registered,
    PinnedHandles,
    PinnedBlocks,
    PinnedHandlesPeak,
    PinnedBlocksPeak,
    NodeCount,
    CriticalPathLength,
}

impl GraphStatusKey {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::BackendKind => "backend_kind",
            Self::BackendName => "backend_name",
            Self::GraphId => "graph_id",
            Self::Registered => "registered",
            Self::PinnedHandles => "pinned_handles",
            Self::PinnedBlocks => "pinned_blocks",
            Self::PinnedHandlesPeak => "pinned_handles_peak",
            Self::PinnedBlocksPeak => "pinned_blocks_peak",
            Self::NodeCount => "node_count",
            Self::CriticalPathLength => "critical_path_length",
        }
    }
}
