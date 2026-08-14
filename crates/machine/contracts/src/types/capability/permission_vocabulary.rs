//! Typed permission vocabulary (`apxm.permission-policy` building blocks).

use serde::{Deserialize, Serialize};

use super::common::CapabilitySchemaError;

pub const PERMISSION_POLICY_SCHEMA_V1: &str = "apxm.permission-policy";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationClass {
    Read,
    Write,
    Destructive,
}

impl OperationClass {
    /// Wire-shape (`snake_case`) string for this variant. Used as the single
    /// source of truth for both serde and non-serde contexts (e.g. metric
    /// labels, which must use typed enum values rather than ad-hoc strings).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Destructive => "destructive",
        }
    }
}

impl std::fmt::Display for OperationClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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

impl ApprovalPosture {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Confirm => "confirm",
            Self::DualControl => "dual_control",
            Self::ExternalSignoff => "external_signoff",
        }
    }
}

impl std::fmt::Display for ApprovalPosture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionScopeKind {
    Session,
    Workspace,
    Org,
    Global,
}

impl PermissionScopeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Workspace => "workspace",
            Self::Org => "org",
            Self::Global => "global",
        }
    }
}

impl std::fmt::Display for PermissionScopeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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
pub enum GrantState {
    Active,
    Revoked,
    Expired,
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
