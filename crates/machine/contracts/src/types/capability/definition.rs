use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::common::{
    CapabilitySchemaError, LifecycleBound, PlannerVisibility, RuntimeLimits, Sensitivity,
    validate_capability_id, validate_non_empty, validate_operations,
};
use super::policy::PromptPolicy;

pub const CAPABILITY_TEMPLATE_SCHEMA_V1: &str = "apxm.capability-template.v1";
pub const CAPABILITY_DEFINITION_SCHEMA_V1: &str = "apxm.capability-definition.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityTemplateV1 {
    pub schema_version: String,
    pub template_key: String,
    pub tool_binding: String,
    pub operations: Vec<super::permission::PermissionOperation>,
    pub resource_selector: super::permission::ResourceSelector,
    pub scope: super::permission::PermissionScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_sensitivity: Option<Sensitivity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_policy: Option<PromptPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_limits: Option<RuntimeLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegability: Option<super::common::Delegability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lifecycle: Option<LifecycleBound>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_visibility: Option<PlannerVisibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl CapabilityTemplateV1 {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        if self.schema_version != CAPABILITY_TEMPLATE_SCHEMA_V1 {
            return Err(CapabilitySchemaError::InvalidSchemaVersion {
                expected: CAPABILITY_TEMPLATE_SCHEMA_V1,
                actual: self.schema_version.clone(),
            });
        }
        validate_non_empty("template_key", &self.template_key)?;
        validate_non_empty("tool_binding", &self.tool_binding)?;
        validate_operations(&self.operations)?;
        self.resource_selector.validate()?;
        self.scope.validate()?;
        if let Some(policy) = &self.prompt_policy {
            policy.validate_for_operations(&self.operations)?;
        }
        if let Some(limits) = &self.runtime_limits {
            limits.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDefinition {
    pub schema_version: String,
    pub id: String,
    pub description: String,
    pub tool: super::tool_binding::ToolBinding,
    pub permissions: super::permission::PermissionPolicy,
    pub metadata: CapabilityMetadata,
}

impl CapabilityDefinition {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        if self.schema_version != CAPABILITY_DEFINITION_SCHEMA_V1 {
            return Err(CapabilitySchemaError::InvalidSchemaVersion {
                expected: CAPABILITY_DEFINITION_SCHEMA_V1,
                actual: self.schema_version.clone(),
            });
        }
        validate_capability_id(&self.id)?;
        validate_non_empty("description", &self.description)?;
        self.tool.validate()?;
        self.permissions.validate()?;
        self.metadata.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_sensitivity: Option<Sensitivity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_visibility: Option<PlannerVisibility>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, String>,
}

impl CapabilityMetadata {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        Ok(())
    }
}
