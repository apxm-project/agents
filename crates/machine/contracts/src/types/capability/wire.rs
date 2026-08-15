//! Runtime capability-grant metadata exchanged with the APXM runtime.

use serde::{Deserialize, Serialize};

use super::common::RuntimeLimits;
use super::permission::{PermissionOperation, PermissionScope, ResourceHandle};

/// Where a grant stands at the moment it is carried into execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantStatus {
    Active,
    PromptPending,
    Revoked,
    Expired,
    Exhausted,
}

/// JSON element stored in execution metadata under `capability_grants`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCapabilityGrant {
    pub grant_id: String,
    pub capability_binding: String,
    pub operations: Vec<PermissionOperation>,
    /// The resource boundary selected when this grant was minted.
    pub resource: ResourceHandle,
    /// Typed selectors constraining the grant's visible resource set.
    pub scope: PermissionScope,
    /// Quotas and tool constraints carried with the grant into execution.
    pub runtime_limits: RuntimeLimits,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    pub status: GrantStatus,
}
