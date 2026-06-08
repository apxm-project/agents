//! Typed wire values for APXM's native orchestration control surface.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Synchronous response status for `apxm_orchestrate_start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationStartStatus {
    /// The bundle was materialized but not started (`dry_run=true`).
    Planned,
    /// The server accepted the workflow and returned a live `execution_id`.
    Running,
}

impl OrchestrationStartStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Running => "running",
        }
    }
}

impl fmt::Display for OrchestrationStartStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Transport used for a generated orchestration worker or supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationTransport {
    /// Spawn a registered ACP profile through the process table.
    Acp,
    /// Use a deterministic fixture graph instead of spawning a host process.
    Deterministic,
}

impl OrchestrationTransport {
    pub const WIRE_VALUES: &[&str] = &[Self::Acp.as_str(), Self::Deterministic.as_str()];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Acp => "acp",
            Self::Deterministic => "deterministic",
        }
    }
}

impl fmt::Display for OrchestrationTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OrchestrationTransport {
    type Err = UnknownOrchestrationTransport;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "acp" => Ok(Self::Acp),
            "deterministic" => Ok(Self::Deterministic),
            _ => Err(UnknownOrchestrationTransport(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownOrchestrationTransport(pub String);

impl fmt::Display for UnknownOrchestrationTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown orchestration transport: {}", self.0)
    }
}

impl std::error::Error for UnknownOrchestrationTransport {}

/// Workspace allocation policy for native orchestration workers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationWorkspaceMode {
    /// Allocate APXM-owned session directories.
    Session,
    /// Reuse the supplied/current repository directory.
    Shared,
    /// Allocate one detached Git worktree per worker.
    GitWorktree,
}

impl OrchestrationWorkspaceMode {
    pub const WIRE_VALUES: &[&str] = &[
        Self::Session.as_str(),
        Self::Shared.as_str(),
        Self::GitWorktree.as_str(),
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Shared => "shared",
            Self::GitWorktree => "git_worktree",
        }
    }
}

impl Default for OrchestrationWorkspaceMode {
    fn default() -> Self {
        Self::Session
    }
}

impl fmt::Display for OrchestrationWorkspaceMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OrchestrationWorkspaceMode {
    type Err = UnknownOrchestrationWorkspaceMode;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "session" => Ok(Self::Session),
            "shared" => Ok(Self::Shared),
            "git_worktree" => Ok(Self::GitWorktree),
            _ => Err(UnknownOrchestrationWorkspaceMode(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownOrchestrationWorkspaceMode(pub String);

impl fmt::Display for UnknownOrchestrationWorkspaceMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown orchestration workspace mode: {}", self.0)
    }
}

impl std::error::Error for UnknownOrchestrationWorkspaceMode {}

/// Cleanup policy for generated orchestration workspaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationWorkspaceCleanup {
    /// Keep artifacts/workspaces for review.
    Keep,
}

impl OrchestrationWorkspaceCleanup {
    pub const WIRE_VALUES: &[&str] = &[Self::Keep.as_str()];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
        }
    }
}

impl Default for OrchestrationWorkspaceCleanup {
    fn default() -> Self {
        Self::Keep
    }
}

impl fmt::Display for OrchestrationWorkspaceCleanup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OrchestrationWorkspaceCleanup {
    type Err = UnknownOrchestrationWorkspaceCleanup;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "keep" => Ok(Self::Keep),
            _ => Err(UnknownOrchestrationWorkspaceCleanup(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownOrchestrationWorkspaceCleanup(pub String);

impl fmt::Display for UnknownOrchestrationWorkspaceCleanup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown orchestration workspace cleanup policy: {}",
            self.0
        )
    }
}

impl std::error::Error for UnknownOrchestrationWorkspaceCleanup {}

/// Outcome value emitted by the native orchestrator wake event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationWakeOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

impl OrchestrationWakeOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for OrchestrationWakeOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_round_trips() {
        for value in OrchestrationTransport::WIRE_VALUES {
            let parsed: OrchestrationTransport = value.parse().expect("transport");
            assert_eq!(parsed.to_string(), *value);
        }
    }

    #[test]
    fn workspace_modes_round_trip() {
        for value in OrchestrationWorkspaceMode::WIRE_VALUES {
            let parsed: OrchestrationWorkspaceMode = value.parse().expect("workspace mode");
            assert_eq!(parsed.to_string(), *value);
        }
    }
}
