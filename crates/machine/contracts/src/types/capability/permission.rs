use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::common::{CapabilitySchemaError, validate_non_empty};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOperation {
    Read,
    List,
    Search,
    Create,
    Write,
    Append,
    Update,
    Delete,
    Execute,
    Send,
    Approve,
    Publish,
}

/// The one permission decision vocabulary, owned by the AIS crate so the
/// contracts, the Agent Program graph, the admission envelope, the runtime
/// chokepoint, and both authoring frontends all resolve through one definition.
pub use apxm_ais::permissions::PermissionDecision;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceHandle {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, serde_json::Value>,
}

impl ResourceHandle {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("resource_handle.kind", &self.kind)?;
        if let Some(uri) = &self.uri {
            validate_non_empty("resource_handle.uri", uri)?;
        }
        if self.uri.is_none() && self.attributes.is_empty() {
            return Err(CapabilitySchemaError::MissingResourceHandleTarget);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionScope {
    pub kind: String,
    pub boundary: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub selectors: BTreeMap<String, serde_json::Value>,
}

impl PermissionScope {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("scope.kind", &self.kind)?;
        validate_non_empty("scope.boundary", &self.boundary)
    }
}
