use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::common::{validate_non_empty, CapabilitySchemaError, LifecycleBound, RuntimeLimits};
use super::policy::PromptPolicy;

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

impl PermissionOperation {
    pub fn is_mutating(self) -> bool {
        matches!(
            self,
            Self::Create
                | Self::Write
                | Self::Append
                | Self::Update
                | Self::Delete
                | Self::Execute
                | Self::Send
                | Self::Approve
                | Self::Publish
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionEffect {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceSelector {
    pub kind: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub selectors: BTreeMap<String, serde_json::Value>,
}

impl ResourceSelector {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("resource_selector.kind", &self.kind)
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionPolicy {
    pub operations: Vec<PermissionOperation>,
    pub resources: Vec<ResourceSelector>,
    pub effect: PermissionEffect,
    pub prompt_policy: PromptPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_ceiling: Option<LifecycleBound>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<RuntimeLimits>,
}

impl PermissionPolicy {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        super::common::validate_operations(&self.operations)?;
        for resource in &self.resources {
            resource.validate()?;
        }
        self.prompt_policy.validate_for_operations(&self.operations)?;
        if let Some(limits) = &self.limits {
            limits.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionRule {
    pub tool: String,
    pub operations: Vec<PermissionOperation>,
    pub resources: Vec<ResourceSelector>,
    pub effect: PermissionEffect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lifecycle: Option<LifecycleBound>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_prompt: Option<bool>,
}

impl PermissionRule {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("permission_rule.tool", &self.tool)?;
        super::common::validate_operations(&self.operations)?;
        for resource in &self.resources {
            resource.validate()?;
        }
        Ok(())
    }
}
