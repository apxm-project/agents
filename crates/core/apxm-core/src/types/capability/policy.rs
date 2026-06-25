use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::common::{validate_non_empty, CapabilitySchemaError};
use super::permission::{PermissionOperation, PermissionRule, PermissionScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    Operator,
    Service,
    Agent,
    Anonymous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    Bearer,
    Local,
    Delegated,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub id: String,
    pub kind: PrincipalKind,
    pub auth_method: AuthMethod,
}

impl Principal {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("principal.id", &self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectContext {
    pub principal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_subject_id: Option<String>,
    pub delegation_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

impl SubjectContext {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("subject.principal_id", &self.principal_id)?;
        validate_non_empty("subject.delegation_mode", &self.delegation_mode)?;
        if let Some(credential_ref) = &self.credential_ref {
            validate_non_empty("subject.credential_ref", credential_ref)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectSelector {
    pub principal_id: Option<String>,
    pub agent_id: Option<String>,
    pub role_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleDefinition {
    pub id: String,
    pub description: String,
    pub allows: Vec<PermissionRule>,
}

impl RoleDefinition {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("role.id", &self.id)?;
        validate_non_empty("role.description", &self.description)?;
        for rule in &self.allows {
            rule.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleAssignment {
    pub subject: SubjectSelector,
    pub role_id: String,
    pub scope: PermissionScope,
}

impl RoleAssignment {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("role_assignment.role_id", &self.role_id)?;
        self.scope.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMode {
    Auto,
    Confirm,
    DualControl,
    ExternalSignoff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptPolicy {
    pub default: PromptMode,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_operation: BTreeMap<PermissionOperation, PromptMode>,
}

impl PromptPolicy {
    pub fn validate_for_operations(
        &self,
        operations: &[PermissionOperation],
    ) -> Result<(), CapabilitySchemaError> {
        let allowed: std::collections::BTreeSet<_> = operations.iter().copied().collect();
        for op in self.by_operation.keys() {
            if !allowed.contains(op) {
                return Err(CapabilitySchemaError::PromptOverrideNotInOperations(*op));
            }
        }
        Ok(())
    }
}
