// AUTO-GENERATED from apxm.host-execution-manifest.v1; DO NOT EDIT.

use crate::types::host::HostTier;
use crate::types::principal::HostPrincipal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostExecutionManifest {
    pub execution_id: String,
    pub host_id: String,
    pub tier: HostTier,
    pub capabilities: Vec<String>,
    pub principal: HostPrincipal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}
