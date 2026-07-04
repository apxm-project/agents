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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::common::{Delegability, Lifecycle, LifecycleBound, RuntimeLimits, Sensitivity};
    use super::super::grant::{CAPABILITY_GRANT_SCHEMA_V1, GrantProvenance};
    use super::super::permission::{PermissionScope, ResourceHandle};
    use super::super::policy::{PromptMode, PromptPolicy, SubjectContext};
    use std::collections::BTreeMap;

    fn grant() -> CapabilityGrant {
        CapabilityGrant {
            schema_version: CAPABILITY_GRANT_SCHEMA_V1.to_string(),
            grant_id: "grant_01jz_wire_projection".to_string(),
            capability: "github.issue.create".to_string(),
            template_key: None,
            capability_binding: "github.issue".to_string(),
            resource: ResourceHandle {
                kind: "github_issue_set".to_string(),
                uri: Some("github://apxm-project/agents/issues".to_string()),
                attributes: BTreeMap::new(),
            },
            operations: vec![PermissionOperation::Create],
            subject: SubjectContext {
                principal_id: "user:rafael".to_string(),
                agent_id: None,
                session_id: None,
                execution_id: None,
                parent_subject_id: None,
                delegation_mode: "on_behalf_of".to_string(),
                credential_ref: None,
            },
            scope: PermissionScope {
                kind: "repository".to_string(),
                boundary: "apxm-project/agents".to_string(),
                selectors: BTreeMap::new(),
            },
            runtime_limits: RuntimeLimits {
                surfaces: BTreeMap::new(),
                quotas: BTreeMap::new(),
                tool_constraints: BTreeMap::new(),
            },
            sensitivity: Sensitivity::Internal,
            prompt_policy: PromptPolicy {
                default: PromptMode::Confirm,
                by_operation: BTreeMap::new(),
            },
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
    fn capability_grant_projects_to_runtime_capability_grant() {
        let grant = grant();
        let wire: RuntimeCapabilityGrant = (&grant).into();

        assert_eq!(wire.grant_id, grant.grant_id);
        assert_eq!(wire.capability_binding, grant.capability_binding);
        assert_eq!(wire.operations, grant.operations);
        assert_eq!(wire.expires_at, grant.lifecycle.expires_at);
        assert_eq!(wire.status, grant.status);
    }
}
