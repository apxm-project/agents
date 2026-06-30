use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::permission::PermissionOperation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Public,
    Internal,
    Confidential,
    Regulated,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delegability {
    None,
    AttenuateOnly,
    DelegateWithCeiling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleBound {
    OneShot,
    Turn,
    Task,
    Session,
    Lease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerVisibility {
    Planner,
    AuditOnly,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lifecycle {
    pub bound: LifecycleBound,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u64>,
}

impl Lifecycle {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        if let Some(expires_at) = &self.expires_at {
            validate_non_empty("lifecycle.expires_at", expires_at)?;
        }
        if self.max_uses == Some(0) {
            return Err(CapabilitySchemaError::InvalidLifecycle(
                "max_uses must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLimits {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub surfaces: std::collections::BTreeMap<String, RuntimeSurfaceLimit>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub quotas: std::collections::BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tool_constraints: std::collections::BTreeMap<String, serde_json::Value>,
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
    #[error("invalid schema_version: expected {expected}, got {actual}")]
    InvalidSchemaVersion {
        expected: &'static str,
        actual: String,
    },
    #[error("{0} must not be empty")]
    EmptyField(&'static str),
    #[error("grant_id must be APXM-minted and begin with 'grant_'")]
    InvalidGrantId,
    #[error("capability id must not be empty")]
    InvalidCapabilityId,
    #[error("operations must not be empty")]
    EmptyOperations,
    #[error("operations must be unique")]
    DuplicateOperations,
    #[error("prompt override for {0:?} is not present in operations")]
    PromptOverrideNotInOperations(PermissionOperation),
    #[error("resource_handle requires uri or attributes")]
    MissingResourceHandleTarget,
    #[error("invalid lifecycle: {0}")]
    InvalidLifecycle(String),
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

pub(crate) fn validate_grant_id(id: &str) -> Result<(), CapabilitySchemaError> {
    if id.starts_with("grant_") && id.len() > 8 {
        Ok(())
    } else {
        Err(CapabilitySchemaError::InvalidGrantId)
    }
}

pub(crate) fn validate_capability_id(id: &str) -> Result<(), CapabilitySchemaError> {
    validate_non_empty("capability", id)
}

pub(crate) fn validate_operations(
    operations: &[PermissionOperation],
) -> Result<(), CapabilitySchemaError> {
    if operations.is_empty() {
        return Err(CapabilitySchemaError::EmptyOperations);
    }
    let unique: BTreeSet<_> = operations.iter().copied().collect();
    if unique.len() != operations.len() {
        return Err(CapabilitySchemaError::DuplicateOperations);
    }
    Ok(())
}
