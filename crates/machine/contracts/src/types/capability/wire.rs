//! Runtime capability-grant metadata exchanged between apxm-server and apxm-runtime.

use serde::{Deserialize, Serialize};

use super::grant::{CapabilityGrant, GrantStatus};
use super::permission::PermissionOperation;

/// JSON element stored in execution metadata under `capability_grants`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCapabilityGrant {
    pub grant_id: String,
    pub capability_binding: String,
    pub operations: Vec<PermissionOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    pub status: GrantStatus,
}

impl From<&CapabilityGrant> for RuntimeCapabilityGrant {
    fn from(grant: &CapabilityGrant) -> Self {
        Self {
            grant_id: grant.grant_id.clone(),
            capability_binding: grant.capability_binding.clone(),
            operations: grant.operations.clone(),
            expires_at: grant.lifecycle.expires_at.clone(),
            status: grant.status,
        }
    }
}
