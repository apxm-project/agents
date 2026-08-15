use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLimits {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub surfaces: BTreeMap<String, RuntimeSurfaceLimit>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub quotas: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tool_constraints: BTreeMap<String, serde_json::Value>,
}

impl RuntimeLimits {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        for surface in self.surfaces.keys() {
            validate_non_empty("runtime_limits.surfaces key", surface)?;
        }
        for quota in self.quotas.keys() {
            validate_non_empty("runtime_limits.quotas key", quota)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RuntimeSurfaceLimit {
    Mode(RuntimeSurfaceMode),
    Policy(RuntimeSurfacePolicy),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSurfaceMode {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSurfacePolicy {
    pub mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CapabilitySchemaError {
    #[error("{0} must not be empty")]
    EmptyField(&'static str),
    #[error("resource_handle requires uri or attributes")]
    MissingResourceHandleTarget,
}

pub(crate) fn validate_non_empty(
    field: &'static str,
    value: &str,
) -> Result<(), CapabilitySchemaError> {
    if value.trim().is_empty() {
        Err(CapabilitySchemaError::EmptyField(field))
    } else {
        Ok(())
    }
}
