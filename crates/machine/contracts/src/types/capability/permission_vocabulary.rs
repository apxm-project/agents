//! Typed permission vocabulary (`apxm.permission-policy.v1` building blocks).

use serde::{Deserialize, Serialize};

use super::common::CapabilitySchemaError;

pub const PERMISSION_POLICY_SCHEMA_V1: &str = "apxm.permission-policy.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationClass {
    Read,
    Write,
    Destructive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Approval posture aligned with `prompt-approval.v1` mode (`auto` = no gate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPosture {
    Auto,
    Confirm,
    DualControl,
    ExternalSignoff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionScopeKind {
    Session,
    Workspace,
    Org,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialScope {
    None,
    Connection,
    Owner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditPayloadPolicy {
    None,
    Metadata,
    Redacted,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecisionKind {
    Allow,
    Deny,
    RequireApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantState {
    Active,
    Revoked,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionReason {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl DecisionReason {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        super::common::validate_non_empty("decision_reason.code", &self.code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionDecision {
    pub decision: PermissionDecisionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<DecisionReason>,
}

impl PermissionDecision {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        if let Some(reason) = &self.reason {
            reason.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionPolicyV1 {
    pub schema_version: String,
    pub operation_class: OperationClass,
    pub risk_level: RiskLevel,
    pub approval_posture: ApprovalPosture,
    pub scope: PermissionScopeKind,
    pub credential_scope: CredentialScope,
    pub audit_payload: AuditPayloadPolicy,
}

impl PermissionPolicyV1 {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        if self.schema_version != PERMISSION_POLICY_SCHEMA_V1 {
            return Err(CapabilitySchemaError::InvalidSchemaVersion {
                expected: PERMISSION_POLICY_SCHEMA_V1,
                actual: self.schema_version.clone(),
            });
        }
        Ok(())
    }
}
