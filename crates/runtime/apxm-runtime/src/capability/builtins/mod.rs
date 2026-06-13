//! Built-in tool and capability implementations for APXM agents.
//!
//! Provides `BashCapability`, `ReadCapability`, `WriteCapability`, and
//! `SearchWebCapability` that agents can invoke during workflow execution.

pub mod authoring;
pub mod bash;
pub mod count_tokens;
pub mod http;
pub mod mcp_bridge;
pub mod provider_call;
pub mod read;
pub mod web_search;
pub mod write;

// Durable agent-management tools (require the `sqlite` feature for persistence).
#[cfg(feature = "sqlite")]
pub mod manage_task;
#[cfg(feature = "sqlite")]
pub mod schedule;
#[cfg(feature = "sqlite")]
pub mod store;

pub use authoring::{ComposeWorkflowCapability, RunWorkflowCapability};
pub use bash::{BashCapability, BashConfig};
pub use count_tokens::CountTokensCapability;
pub use http::{HttpGetCapability, HttpPostCapability, guard_url_ssrf, guard_url_ssrf_pinned};
pub(crate) use http::{client_for, shared_client};
pub use mcp_bridge::McpBridgeCapability;
pub use provider_call::ProviderCallCapability;
pub use read::{ReadCapability, ReadConfig};
pub use web_search::{SearchDepth, SearchWebCapability, SearchWebConfig};
pub use write::{WriteCapability, WriteConfig};

#[cfg(feature = "sqlite")]
pub use manage_task::{ManageTaskCapability, parse_policy, wire_policy};
#[cfg(feature = "sqlite")]
pub use schedule::{FiredSchedule, OnFire, ScheduleCapability, fire_due, spawn_firer};
#[cfg(feature = "sqlite")]
pub use store::{ScheduleRow, TaskRow, ToolsStore};

use crate::CapabilitySystem;
use apxm_core::{error::RuntimeError, types::Value};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ffi::OsString,
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// Serde default helper for boolean fields that should default to `true`.
pub(crate) fn default_true() -> bool {
    true
}

/// Extract a required string argument from a capability argument map.
pub(crate) fn require_string_arg<'a>(
    args: &'a HashMap<String, Value>,
    primary_key: &str,
    capability_name: &str,
) -> Result<&'a str, RuntimeError> {
    args.get(primary_key)
        .and_then(|v| v.as_string())
        .map(|s| s.as_str())
        .ok_or_else(|| RuntimeError::Capability {
            capability: capability_name.to_string(),
            message: format!("Missing required '{primary_key}' argument"),
        })
}

/// Normalize `.` and `..` components without touching the filesystem.
///
/// Capability policy checks use this before prefix comparisons so
/// `base/../outside` is not treated as being inside `base`.
pub(crate) fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push("..");
                }
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

/// Canonicalize a path, or if it does not exist yet, canonicalize the nearest
/// existing ancestor and append the missing suffix.
pub(crate) fn canonicalize_path_or_existing_ancestor(path: &Path) -> io::Result<PathBuf> {
    let path = normalize_path_lexically(path);
    if path.exists() {
        return std::fs::canonicalize(path);
    }

    let mut missing = Vec::<OsString>::new();
    let mut current = path.as_path();
    loop {
        if current.exists() {
            let mut canonical = std::fs::canonicalize(current)?;
            for component in missing.iter().rev() {
                canonical.push(component);
            }
            return Ok(canonical);
        }

        let Some(name) = current.file_name() else {
            return std::fs::canonicalize(current);
        };
        missing.push(name.to_os_string());
        let Some(parent) = current.parent() else {
            return std::fs::canonicalize(current);
        };
        current = parent;
    }
}

pub(crate) fn canonicalize_policy_path(
    path: &Path,
    capability_name: &str,
    policy_field: &str,
) -> Result<PathBuf, RuntimeError> {
    canonicalize_path_or_existing_ancestor(path).map_err(|error| RuntimeError::Capability {
        capability: capability_name.to_string(),
        message: format!(
            "Failed to resolve {policy_field} path '{}': {error}",
            path.display()
        ),
    })
}

/// Configuration for APxM standard tools.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ToolsConfig {
    #[serde(default)]
    pub bash: BashConfig,
    #[serde(default)]
    pub read: ReadConfig,
    #[serde(default)]
    pub write: WriteConfig,
    #[serde(default)]
    pub search_web: SearchWebConfig,
}

/// Register the standard APxM capabilities with the runtime capability system.
pub fn register_standard_tools(
    capability_system: &CapabilitySystem,
    config: &ToolsConfig,
) -> Result<(), RuntimeError> {
    if config.bash.enabled {
        capability_system.register(Arc::new(BashCapability::with_config(config.bash.clone())))?;
    }
    if config.read.enabled {
        capability_system.register(Arc::new(ReadCapability::with_config(config.read.clone())))?;
    }
    if config.write.enabled {
        capability_system.register(Arc::new(WriteCapability::with_config(config.write.clone())))?;
    }
    if config.search_web.enabled {
        capability_system.register(Arc::new(SearchWebCapability::with_config(
            config.search_web.clone(),
        )))?;
    }
    Ok(())
}
