use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::common::{
    validate_grant_id, validate_non_empty, validate_operations, CapabilitySchemaError, Delegability,
    Lifecycle, LifecycleBound, RuntimeLimits, Sensitivity,
};
use super::permission::{PermissionOperation, PermissionScope, ResourceHandle};
use super::policy::{PromptPolicy, SubjectContext};

pub const CAPABILITY_GRANT_SCHEMA_V1: &str = "apxm.capability-grant.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityGrant {
    pub schema_version: String,
    pub grant_id: String,
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_key: Option<String>,
    pub tool_binding: String,
    pub resource: ResourceHandle,
    pub operations: Vec<PermissionOperation>,
    pub subject: SubjectContext,
    pub scope: PermissionScope,
    pub runtime_limits: RuntimeLimits,
    pub sensitivity: Sensitivity,
    pub prompt_policy: PromptPolicy,
    pub delegability: Delegability,
    pub lifecycle: Lifecycle,
    pub provenance: GrantProvenance,
    pub status: GrantStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_grant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub trace_tags: BTreeMap<String, String>,
}

impl CapabilityGrant {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        if self.schema_version != CAPABILITY_GRANT_SCHEMA_V1 {
            return Err(CapabilitySchemaError::InvalidSchemaVersion {
                expected: CAPABILITY_GRANT_SCHEMA_V1,
                actual: self.schema_version.clone(),
            });
        }
        validate_grant_id(&self.grant_id)?;
        validate_non_empty("capability", &self.capability)?;
        if let Some(parent_id) = &self.parent_grant_id {
            validate_grant_id(parent_id)?;
        }
        if let Some(template_key) = &self.template_key {
            validate_non_empty("template_key", template_key)?;
        }
        validate_non_empty("tool_binding", &self.tool_binding)?;
        validate_operations(&self.operations)?;
        self.resource.validate()?;
        self.subject.validate()?;
        self.scope.validate()?;
        self.runtime_limits.validate()?;
        self.prompt_policy
            .validate_for_operations(&self.operations)?;
        self.lifecycle.validate()?;
        self.provenance.validate()?;

        if self.lifecycle.bound == LifecycleBound::OneShot && self.lifecycle.max_uses != Some(1) {
            return Err(CapabilitySchemaError::InvalidLifecycle(
                "one_shot grants require max_uses = 1".to_string(),
            ));
        }
        if self.lifecycle.bound == LifecycleBound::Lease && self.lifecycle.expires_at.is_none() {
            return Err(CapabilitySchemaError::InvalidLifecycle(
                "lease grants require expires_at".to_string(),
            ));
        }
        if self.operations.iter().any(|op| op.is_mutating())
            && self.lifecycle.expires_at.is_none()
            && !matches!(self.lifecycle.bound, LifecycleBound::OneShot)
        {
            return Err(CapabilitySchemaError::InvalidLifecycle(
                "mutable capability grants require finite expiry".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantStatus {
    Active,
    PromptPending,
    Revoked,
    Expired,
    Exhausted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantProvenance {
    pub minted_by: String,
    pub policy_version: String,
    pub request_id: String,
}

impl GrantProvenance {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("provenance.minted_by", &self.minted_by)?;
        validate_non_empty("provenance.policy_version", &self.policy_version)?;
        validate_non_empty("provenance.request_id", &self.request_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::capability::{PromptMode, ResourceHandle, PermissionScope};
    use std::collections::BTreeMap;

    fn scope() -> PermissionScope {
        PermissionScope {
            kind: "repository".to_string(),
            boundary: "apxm-project/apxm".to_string(),
            selectors: BTreeMap::new(),
        }
    }

    fn runtime_limits() -> RuntimeLimits {
        RuntimeLimits {
            surfaces: BTreeMap::new(),
            quotas: BTreeMap::new(),
            tool_constraints: BTreeMap::new(),
        }
    }

    fn prompt_policy() -> PromptPolicy {
        PromptPolicy {
            default: PromptMode::Confirm,
            by_operation: BTreeMap::from([(PermissionOperation::Create, PromptMode::Confirm)]),
        }
    }

    fn subject() -> SubjectContext {
        SubjectContext {
            principal_id: "user:rafael".to_string(),
            agent_id: None,
            session_id: None,
            execution_id: None,
            parent_subject_id: None,
            delegation_mode: "on_behalf_of".to_string(),
            credential_ref: Some("github/main".to_string()),
        }
    }

    fn grant() -> CapabilityGrant {
        CapabilityGrant {
            schema_version: CAPABILITY_GRANT_SCHEMA_V1.to_string(),
            grant_id: "grant_01jz_runtime_minted".to_string(),
            capability: "github.issue.create".to_string(),
            template_key: Some("github.issue.create".to_string()),
            tool_binding: "github.issue".to_string(),
            resource: ResourceHandle {
                kind: "github_issue_set".to_string(),
                uri: Some("github://apxm-project/apxm/issues".to_string()),
                attributes: BTreeMap::new(),
            },
            operations: vec![PermissionOperation::Create],
            subject: subject(),
            scope: scope(),
            runtime_limits: runtime_limits(),
            sensitivity: Sensitivity::Internal,
            prompt_policy: prompt_policy(),
            delegability: Delegability::AttenuateOnly,
            lifecycle: Lifecycle {
                bound: LifecycleBound::Task,
                expires_at: Some("2026-06-24T21:00:00Z".to_string()),
                max_uses: Some(1),
            },
            provenance: GrantProvenance {
                minted_by: "apxm-server".to_string(),
                policy_version: "capability-policy.v1".to_string(),
                request_id: "req_123".to_string(),
            },
            status: GrantStatus::Active,
            parent_grant_id: None,
            description: None,
            trace_tags: BTreeMap::new(),
        }
    }

    #[test]
    fn capability_grant_validates_required_shape() {
        grant().validate().expect("valid capability grant");
    }

    #[test]
    fn capability_grant_rejects_invalid_grant_ids() {
        let mut g = grant();
        g.grant_id = "cap_01jz_runtime_minted".to_string();

        assert_eq!(
            g.validate().unwrap_err(),
            CapabilitySchemaError::InvalidGrantId
        );
    }

    #[test]
    fn empty_operations_are_rejected() {
        let mut g = grant();
        g.operations.clear();

        assert_eq!(
            g.validate().unwrap_err(),
            CapabilitySchemaError::EmptyOperations
        );
    }

    #[test]
    fn duplicate_operations_are_rejected() {
        let mut g = grant();
        g.operations.push(PermissionOperation::Create);

        assert_eq!(
            g.validate().unwrap_err(),
            CapabilitySchemaError::DuplicateOperations
        );
    }

    #[test]
    fn prompt_overrides_must_match_operations() {
        let mut g = grant();
        g.prompt_policy
            .by_operation
            .insert(PermissionOperation::Delete, PromptMode::DualControl);

        assert_eq!(
            g.validate().unwrap_err(),
            CapabilitySchemaError::PromptOverrideNotInOperations(PermissionOperation::Delete)
        );
    }

    #[test]
    fn mutable_grants_require_expiry() {
        let mut g = grant();
        g.lifecycle.expires_at = None;

        assert!(matches!(
            g.validate().unwrap_err(),
            CapabilitySchemaError::InvalidLifecycle(_)
        ));
    }

    #[test]
    fn unknown_fields_are_rejected_in_enforcement_schema() {
        let raw = serde_json::json!({
            "schema_version": CAPABILITY_GRANT_SCHEMA_V1,
            "grant_id": "grant_01jz_runtime_minted",
            "capability": "github.issue.create",
            "tool_binding": "github.issue",
            "resource": {"kind": "github_issue_set", "uri": "github://apxm-project/apxm/issues"},
            "operations": ["create"],
            "subject": {"principal_id": "user:rafael", "delegation_mode": "on_behalf_of"},
            "scope": {"kind": "repository", "boundary": "apxm-project/apxm"},
            "runtime_limits": {},
            "sensitivity": "internal",
            "prompt_policy": {"default": "confirm"},
            "delegability": "attenuate_only",
            "lifecycle": {"bound": "task", "expires_at": "2026-06-24T21:00:00Z"},
            "provenance": {"minted_by": "apxm-server", "policy_version": "capability-policy.v1", "request_id": "req_123"},
            "status": "active",
            "legacy_name": "github.create_issue"
        });

        assert!(serde_json::from_value::<CapabilityGrant>(raw).is_err());
    }
}
