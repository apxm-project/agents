//! Built-in tool and capability implementations for APXM agents.
//!
//! Provides `BashCapability`, `ReadCapability`, `WriteCapability`, and
//! `SearchWebCapability` that agents can invoke during workflow execution.

pub mod bash;
pub mod read;
pub mod web_search;
pub mod write;

pub use bash::{BashCapability, BashConfig};
pub use read::{ReadCapability, ReadConfig};
pub use web_search::{SearchDepth, SearchWebCapability, SearchWebConfig};
pub use write::{WriteCapability, WriteConfig};

use apxm_core::{error::RuntimeError, types::Value};
use crate::CapabilitySystem;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

/// Serde default helper for boolean fields that should default to `true`.
pub(crate) fn default_true() -> bool {
    true
}

/// Extract a required string argument from a capability argument map.
///
/// Looks up `primary_key`, then `alt_key`, then `"arg0"`.
pub(crate) fn require_string_arg<'a>(
    args: &'a HashMap<String, Value>,
    primary_key: &str,
    alt_key: &str,
    capability_name: &str,
) -> Result<&'a str, RuntimeError> {
    args.get(primary_key)
        .or_else(|| args.get(alt_key))
        .or_else(|| args.get("arg0"))
        .and_then(|v| v.as_string())
        .map(|s| s.as_str())
        .ok_or_else(|| RuntimeError::Capability {
            capability: capability_name.to_string(),
            message: format!("Missing required '{primary_key}' argument"),
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
