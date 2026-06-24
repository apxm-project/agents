//! Capability authority schema.
//!
//! Capability templates are authoring-time authority requests. Delegated
//! capabilities are runtime-minted authority objects and are the only records
//! that can authorize external action.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub const CAPABILITY_TEMPLATE_SCHEMA_V1: &str = "apxm.capability-template.v1";
pub const DELEGATED_CAPABILITY_SCHEMA_V1: &str = "apxm.capability.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityTemplateV1 {
    pub schema_version: String,
    pub template_key: String,
    pub tool_binding: String,
    pub operations: Vec<CapabilityOperation>,
    pub resource_selector: ResourceSelector,
    pub scope: CapabilityScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_sensitivity: Option<Sensitivity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<ApprovalPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_limits: Option<RuntimeLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegability: Option<Delegability>,
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
        if let Some(policy) = &self.approval_policy {
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
pub struct DelegatedCapabilityV1 {
    pub schema_version: String,
    pub capability_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_key: Option<String>,
    pub tool_binding: String,
    pub resource_handle: ResourceHandle,
    pub operations: Vec<CapabilityOperation>,
    pub auth_context: AuthContext,
    pub scope: CapabilityScope,
    pub runtime_limits: RuntimeLimits,
    pub sensitivity: Sensitivity,
    pub approval_policy: ApprovalPolicy,
    pub delegability: Delegability,
    pub lifecycle: Lifecycle,
    pub provenance: CapabilityProvenance,
    pub status: CapabilityStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_capability_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub trace_tags: BTreeMap<String, String>,
}

impl DelegatedCapabilityV1 {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        if self.schema_version != DELEGATED_CAPABILITY_SCHEMA_V1 {
            return Err(CapabilitySchemaError::InvalidSchemaVersion {
                expected: DELEGATED_CAPABILITY_SCHEMA_V1,
                actual: self.schema_version.clone(),
            });
        }
        validate_capability_id(&self.capability_id)?;
        if let Some(parent_id) = &self.parent_capability_id {
            validate_capability_id(parent_id)?;
        }
        if let Some(template_key) = &self.template_key {
            validate_non_empty("template_key", template_key)?;
        }
        validate_non_empty("tool_binding", &self.tool_binding)?;
        validate_operations(&self.operations)?;
        self.resource_handle.validate()?;
        self.auth_context.validate()?;
        self.scope.validate()?;
        self.runtime_limits.validate()?;
        self.approval_policy
            .validate_for_operations(&self.operations)?;
        self.lifecycle.validate()?;
        self.provenance.validate()?;

        if self.lifecycle.bound == LifecycleBound::OneShot && self.lifecycle.max_uses != Some(1) {
            return Err(CapabilitySchemaError::InvalidLifecycle(
                "one_shot capabilities require max_uses = 1".to_string(),
            ));
        }
        if self.lifecycle.bound == LifecycleBound::Lease && self.lifecycle.expires_at.is_none() {
            return Err(CapabilitySchemaError::InvalidLifecycle(
                "lease capabilities require expires_at".to_string(),
            ));
        }
        if self.operations.iter().any(|op| op.is_mutating())
            && self.lifecycle.expires_at.is_none()
            && !matches!(self.lifecycle.bound, LifecycleBound::OneShot)
        {
            return Err(CapabilitySchemaError::InvalidLifecycle(
                "mutable delegated capabilities require finite expiry".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityOperation {
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

impl CapabilityOperation {
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
pub enum Sensitivity {
    Public,
    Internal,
    Confidential,
    Regulated,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    Auto,
    Confirm,
    DualControl,
    ExternalSignoff,
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
pub enum CapabilityStatus {
    Active,
    ApprovalPending,
    Revoked,
    Expired,
    Exhausted,
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
pub struct CapabilityScope {
    pub kind: String,
    pub boundary: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub selectors: BTreeMap<String, serde_json::Value>,
}

impl CapabilityScope {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("scope.kind", &self.kind)?;
        validate_non_empty("scope.boundary", &self.boundary)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthContext {
    pub principal: String,
    pub delegation_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

impl AuthContext {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("auth_context.principal", &self.principal)?;
        validate_non_empty("auth_context.delegation_mode", &self.delegation_mode)?;
        if let Some(credential_ref) = &self.credential_ref {
            validate_non_empty("auth_context.credential_ref", credential_ref)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalPolicy {
    pub default: ApprovalMode,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_operation: BTreeMap<CapabilityOperation, ApprovalMode>,
}

impl ApprovalPolicy {
    pub fn validate_for_operations(
        &self,
        operations: &[CapabilityOperation],
    ) -> Result<(), CapabilitySchemaError> {
        let allowed: BTreeSet<_> = operations.iter().copied().collect();
        for op in self.by_operation.keys() {
            if !allowed.contains(op) {
                return Err(CapabilitySchemaError::ApprovalOverrideNotInOperations(*op));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLimits {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub surfaces: BTreeMap<String, RuntimeSurfaceLimit>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub quotas: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tool_constraints: BTreeMap<String, serde_json::Value>,
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
pub struct CapabilityProvenance {
    pub minted_by: String,
    pub policy_version: String,
    pub request_id: String,
}

impl CapabilityProvenance {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("provenance.minted_by", &self.minted_by)?;
        validate_non_empty("provenance.policy_version", &self.policy_version)?;
        validate_non_empty("provenance.request_id", &self.request_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolBindingMetadata {
    pub name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
    #[serde(default)]
    pub requires_auth: bool,
    #[serde(default)]
    pub read_only_hint: bool,
}

impl ToolBindingMetadata {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("tool_binding.name", &self.name)?;
        validate_non_empty("tool_binding.description", &self.description)
    }
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
    #[error("capability_id must be APXM-minted and begin with 'cap_'")]
    InvalidCapabilityId,
    #[error("operations must not be empty")]
    EmptyOperations,
    #[error("operations must be unique")]
    DuplicateOperations,
    #[error("approval override for {0:?} is not present in operations")]
    ApprovalOverrideNotInOperations(CapabilityOperation),
    #[error("resource_handle requires uri or attributes")]
    MissingResourceHandleTarget,
    #[error("invalid lifecycle: {0}")]
    InvalidLifecycle(String),
}

fn validate_non_empty(field: &'static str, value: &str) -> Result<(), CapabilitySchemaError> {
    if value.trim().is_empty() {
        Err(CapabilitySchemaError::EmptyField(field))
    } else {
        Ok(())
    }
}

fn validate_capability_id(id: &str) -> Result<(), CapabilitySchemaError> {
    if id.starts_with("cap_") && id.len() > 8 {
        Ok(())
    } else {
        Err(CapabilitySchemaError::InvalidCapabilityId)
    }
}

fn validate_operations(operations: &[CapabilityOperation]) -> Result<(), CapabilitySchemaError> {
    if operations.is_empty() {
        return Err(CapabilitySchemaError::EmptyOperations);
    }
    let unique: BTreeSet<_> = operations.iter().copied().collect();
    if unique.len() != operations.len() {
        return Err(CapabilitySchemaError::DuplicateOperations);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> CapabilityScope {
        CapabilityScope {
            kind: "repository".to_string(),
            boundary: "apxm-project/apxm".to_string(),
            selectors: BTreeMap::new(),
        }
    }

    fn runtime_limits() -> RuntimeLimits {
        let mut surfaces = BTreeMap::new();
        surfaces.insert(
            "shell".to_string(),
            RuntimeSurfaceLimit::Mode(RuntimeSurfaceMode::Deny),
        );
        RuntimeLimits {
            surfaces,
            quotas: BTreeMap::new(),
            tool_constraints: BTreeMap::new(),
        }
    }

    fn approval_policy() -> ApprovalPolicy {
        ApprovalPolicy {
            default: ApprovalMode::Confirm,
            by_operation: BTreeMap::from([(CapabilityOperation::Create, ApprovalMode::Confirm)]),
        }
    }

    fn delegated() -> DelegatedCapabilityV1 {
        DelegatedCapabilityV1 {
            schema_version: DELEGATED_CAPABILITY_SCHEMA_V1.to_string(),
            capability_id: "cap_01jz_runtime_minted".to_string(),
            template_key: Some("github.issue.create".to_string()),
            tool_binding: "github.issue".to_string(),
            resource_handle: ResourceHandle {
                kind: "github_issue_set".to_string(),
                uri: Some("github://apxm-project/apxm/issues".to_string()),
                attributes: BTreeMap::new(),
            },
            operations: vec![CapabilityOperation::Create],
            auth_context: AuthContext {
                principal: "user:rafael".to_string(),
                delegation_mode: "on_behalf_of".to_string(),
                credential_ref: Some("github/main".to_string()),
            },
            scope: scope(),
            runtime_limits: runtime_limits(),
            sensitivity: Sensitivity::Internal,
            approval_policy: approval_policy(),
            delegability: Delegability::AttenuateOnly,
            lifecycle: Lifecycle {
                bound: LifecycleBound::Task,
                expires_at: Some("2026-06-24T21:00:00Z".to_string()),
                max_uses: Some(1),
            },
            provenance: CapabilityProvenance {
                minted_by: "apxm-server".to_string(),
                policy_version: "capability-policy.v1".to_string(),
                request_id: "req_123".to_string(),
            },
            status: CapabilityStatus::Active,
            parent_capability_id: None,
            description: None,
            trace_tags: BTreeMap::new(),
        }
    }

    #[test]
    fn capability_template_validates_required_shape() {
        let template = CapabilityTemplateV1 {
            schema_version: CAPABILITY_TEMPLATE_SCHEMA_V1.to_string(),
            template_key: "github.issue.create".to_string(),
            tool_binding: "github.issue".to_string(),
            operations: vec![CapabilityOperation::Create],
            resource_selector: ResourceSelector {
                kind: "github_issue".to_string(),
                selectors: BTreeMap::new(),
            },
            scope: scope(),
            default_sensitivity: Some(Sensitivity::Internal),
            approval_policy: Some(approval_policy()),
            runtime_limits: Some(runtime_limits()),
            delegability: Some(Delegability::AttenuateOnly),
            max_lifecycle: Some(LifecycleBound::Task),
            planner_visibility: Some(PlannerVisibility::Planner),
            description: Some("Create issues".to_string()),
        };

        template.validate().expect("valid template");
    }

    #[test]
    fn delegated_capability_validates_required_shape() {
        delegated().validate().expect("valid delegated capability");
    }

    #[test]
    fn delegated_capability_rejects_model_authored_ids() {
        let mut cap = delegated();
        cap.capability_id = "github.create_issue".to_string();

        assert_eq!(
            cap.validate().unwrap_err(),
            CapabilitySchemaError::InvalidCapabilityId
        );
    }

    #[test]
    fn empty_operations_are_rejected() {
        let mut cap = delegated();
        cap.operations.clear();

        assert_eq!(
            cap.validate().unwrap_err(),
            CapabilitySchemaError::EmptyOperations
        );
    }

    #[test]
    fn duplicate_operations_are_rejected() {
        let mut cap = delegated();
        cap.operations.push(CapabilityOperation::Create);

        assert_eq!(
            cap.validate().unwrap_err(),
            CapabilitySchemaError::DuplicateOperations
        );
    }

    #[test]
    fn approval_overrides_must_match_operations() {
        let mut cap = delegated();
        cap.approval_policy
            .by_operation
            .insert(CapabilityOperation::Delete, ApprovalMode::DualControl);

        assert_eq!(
            cap.validate().unwrap_err(),
            CapabilitySchemaError::ApprovalOverrideNotInOperations(CapabilityOperation::Delete)
        );
    }

    #[test]
    fn mutable_delegated_capabilities_require_expiry() {
        let mut cap = delegated();
        cap.lifecycle.expires_at = None;

        assert!(matches!(
            cap.validate().unwrap_err(),
            CapabilitySchemaError::InvalidLifecycle(_)
        ));
    }

    #[test]
    fn unknown_fields_are_rejected_in_enforcement_schema() {
        let raw = serde_json::json!({
            "schema_version": DELEGATED_CAPABILITY_SCHEMA_V1,
            "capability_id": "cap_01jz_runtime_minted",
            "tool_binding": "github.issue",
            "resource_handle": {"kind": "github_issue_set", "uri": "github://apxm-project/apxm/issues"},
            "operations": ["create"],
            "auth_context": {"principal": "user:rafael", "delegation_mode": "on_behalf_of"},
            "scope": {"kind": "repository", "boundary": "apxm-project/apxm"},
            "runtime_limits": {},
            "sensitivity": "internal",
            "approval_policy": {"default": "confirm"},
            "delegability": "attenuate_only",
            "lifecycle": {"bound": "task", "expires_at": "2026-06-24T21:00:00Z"},
            "provenance": {"minted_by": "apxm-server", "policy_version": "capability-policy.v1", "request_id": "req_123"},
            "status": "active",
            "legacy_name": "github.create_issue"
        });

        assert!(serde_json::from_value::<DelegatedCapabilityV1>(raw).is_err());
    }
}
