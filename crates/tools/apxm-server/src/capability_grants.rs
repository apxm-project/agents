use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use apxm_core::constants::orchestration::admission as orchestration_admission;
use apxm_core::types::{
    CapabilityGrant, CAPABILITY_GRANT_SCHEMA_V1, Delegability, GrantProvenance, GrantStatus,
    Lifecycle, LifecycleBound, PermissionOperation, PermissionScope, PromptMode, PromptPolicy,
    ResourceHandle, RuntimeCapabilityGrant, RuntimeLimits, Sensitivity, SubjectContext,
};
use axum::Json;
use axum::extract::{Path, State};
use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

const DEFAULT_MUTATING_LEASE_SECONDS: i64 = 15 * 60;
const POLICY_VERSION: &str = "capability-policy.v1";

#[derive(Clone, Default)]
pub(crate) struct CapabilityGrantStore {
    entries: Arc<DashMap<String, CapabilityGrant>>,
}

impl CapabilityGrantStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert(&self, grant: CapabilityGrant) {
        self.entries.insert(grant.grant_id.clone(), grant);
    }

    pub(crate) fn get_active(&self, grant_id: &str) -> Result<CapabilityGrant, ApiError> {
        let mut entry = self.entries.get_mut(grant_id).ok_or_else(|| {
            ApiError::admission_denied(format!(
                "capability grant id '{grant_id}' is not recognized"
            ))
        })?;
        if entry.status != GrantStatus::Active {
            return Err(ApiError::admission_denied(format!(
                "capability grant id '{grant_id}' is {:?}",
                entry.status
            )));
        }
        if lifecycle_expired(&entry.lifecycle) {
            entry.status = GrantStatus::Expired;
            return Err(ApiError::admission_denied(format!(
                "capability grant id '{grant_id}' is expired"
            )));
        }
        Ok(entry.clone())
    }

    pub(crate) fn resolve(
        &self,
        grant_ids: &HashSet<String>,
    ) -> Result<ResolvedCapabilityGrants, ApiError> {
        let mut grants = Vec::with_capacity(grant_ids.len());
        for grant_id in grant_ids {
            if !grant_id.starts_with("grant_") {
                return Err(ApiError::admission_denied(format!(
                    "capability_grant_ids must contain APXM-minted grant_* ids; '{grant_id}' is not a capability grant id"
                )));
            }
            grants.push(self.get_active(grant_id)?);
        }
        Ok(ResolvedCapabilityGrants::new(grants))
    }

    fn revoke(&self, grant_id: &str) -> Result<CapabilityGrant, ApiError> {
        let mut entry = self.entries.get_mut(grant_id).ok_or_else(|| {
            ApiError::not_found(format!(
                "capability grant id '{grant_id}' is not recognized"
            ))
        })?;
        entry.status = GrantStatus::Revoked;
        Ok(entry.clone())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedCapabilityGrants {
    grants: Vec<CapabilityGrant>,
}

impl ResolvedCapabilityGrants {
    fn new(grants: Vec<CapabilityGrant>) -> Self {
        Self { grants }
    }

    pub(crate) fn empty() -> Self {
        Self { grants: Vec::new() }
    }

    pub(crate) fn admits_mutating_tool(&self, tool_binding: &str) -> bool {
        self.grants.iter().any(|grant| {
            grant.status == GrantStatus::Active
                && grant.tool_binding == tool_binding
                && grant
                    .operations
                    .iter()
                    .copied()
                    .any(PermissionOperation::is_mutating)
                && !lifecycle_expired(&grant.lifecycle)
        })
    }

    pub(crate) fn to_runtime_metadata_json(&self) -> Result<String, ApiError> {
        let runtime_grants: Vec<RuntimeCapabilityGrant> = self
            .grants
            .iter()
            .map(RuntimeCapabilityGrant::from)
            .collect();
        serde_json::to_string(&runtime_grants).map_err(|error| {
            ApiError::internal_message(format!(
                "failed to serialize capability grant metadata: {error}"
            ))
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MintCapabilityGrantRequest {
    #[serde(default)]
    pub(crate) template_key: Option<String>,
    #[serde(default)]
    pub(crate) capability: Option<String>,
    pub(crate) tool_binding: String,
    #[serde(default)]
    pub(crate) operations: Vec<PermissionOperation>,
    #[serde(default)]
    pub(crate) resource: Option<ResourceHandle>,
    #[serde(default)]
    pub(crate) subject: Option<SubjectContext>,
    #[serde(default)]
    pub(crate) scope: Option<PermissionScope>,
    #[serde(default)]
    pub(crate) runtime_limits: Option<RuntimeLimits>,
    #[serde(default)]
    pub(crate) sensitivity: Option<Sensitivity>,
    #[serde(default)]
    pub(crate) prompt_policy: Option<PromptPolicy>,
    #[serde(default)]
    pub(crate) delegability: Option<Delegability>,
    #[serde(default)]
    pub(crate) lifecycle: Option<Lifecycle>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) trace_tags: BTreeMap<String, String>,
}

pub(crate) async fn mint_capability_grant(
    State(state): State<AppState>,
    Json(req): Json<MintCapabilityGrantRequest>,
) -> Result<Json<CapabilityGrant>, ApiError> {
    let grant = mint_capability_grant_inner(&state, req)?;
    state.capability_grants.insert(grant.clone());
    Ok(Json(grant))
}

pub(crate) async fn revoke_capability_grant(
    State(state): State<AppState>,
    Path(grant_id): Path<String>,
) -> Result<Json<CapabilityGrant>, ApiError> {
    Ok(Json(state.capability_grants.revoke(&grant_id)?))
}

fn mint_capability_grant_inner(
    state: &AppState,
    req: MintCapabilityGrantRequest,
) -> Result<CapabilityGrant, ApiError> {
    let tool_binding = req.tool_binding.trim().to_string();
    if tool_binding.is_empty() {
        return Err(ApiError::bad_request("tool_binding must not be empty"));
    }
    let operations = if req.operations.is_empty() {
        default_operations(state, &tool_binding)
    } else {
        req.operations
    };
    let lifecycle = req
        .lifecycle
        .unwrap_or_else(|| default_lifecycle(&operations));
    let capability = req
        .capability
        .or(req.template_key.clone())
        .unwrap_or_else(|| tool_binding.clone());
    let grant = CapabilityGrant {
        schema_version: CAPABILITY_GRANT_SCHEMA_V1.to_string(),
        grant_id: format!("grant_{}", uuid::Uuid::new_v4().simple()),
        capability,
        template_key: req.template_key,
        tool_binding,
        resource: req.resource.unwrap_or_else(default_resource),
        operations,
        subject: req.subject.unwrap_or_else(default_subject),
        scope: req.scope.unwrap_or_else(default_scope),
        runtime_limits: req.runtime_limits.unwrap_or_else(default_runtime_limits),
        sensitivity: req.sensitivity.unwrap_or(Sensitivity::Internal),
        prompt_policy: req.prompt_policy.unwrap_or_else(default_prompt_policy),
        delegability: req.delegability.unwrap_or(Delegability::AttenuateOnly),
        lifecycle,
        provenance: GrantProvenance {
            minted_by: "apxm-server".to_string(),
            policy_version: POLICY_VERSION.to_string(),
            request_id: format!("req_{}", uuid::Uuid::new_v4().simple()),
        },
        status: GrantStatus::Active,
        parent_grant_id: None,
        description: req.description,
        trace_tags: req.trace_tags,
    };
    grant
        .validate()
        .map_err(|error| ApiError::bad_request(format!("invalid capability grant: {error}")))?;
    Ok(grant)
}

fn default_operations(state: &AppState, tool_binding: &str) -> Vec<PermissionOperation> {
    if tool_binding == orchestration_admission::SPAWN_AGENT
        || tool_binding == orchestration_admission::SPAWN_TEAM
    {
        return vec![PermissionOperation::Execute];
    }
    if state.runtime.capability_system().is_read_only(tool_binding) {
        vec![PermissionOperation::Read]
    } else {
        vec![PermissionOperation::Write]
    }
}

fn default_lifecycle(operations: &[PermissionOperation]) -> Lifecycle {
    if operations
        .iter()
        .copied()
        .any(PermissionOperation::is_mutating)
    {
        Lifecycle {
            bound: LifecycleBound::Lease,
            expires_at: Some(
                (Utc::now() + Duration::seconds(DEFAULT_MUTATING_LEASE_SECONDS)).to_rfc3339(),
            ),
            max_uses: None,
        }
    } else {
        Lifecycle {
            bound: LifecycleBound::Turn,
            expires_at: None,
            max_uses: None,
        }
    }
}

fn default_resource() -> ResourceHandle {
    ResourceHandle {
        kind: "tool_binding".to_string(),
        uri: Some("apxm://capability-template".to_string()),
        attributes: BTreeMap::new(),
    }
}

fn default_subject() -> SubjectContext {
    SubjectContext {
        principal_id: "local-user".to_string(),
        agent_id: None,
        session_id: None,
        execution_id: None,
        parent_subject_id: None,
        delegation_mode: "on_behalf_of".to_string(),
        credential_ref: None,
    }
}

fn default_scope() -> PermissionScope {
    PermissionScope {
        kind: "server".to_string(),
        boundary: "local".to_string(),
        selectors: BTreeMap::new(),
    }
}

fn default_runtime_limits() -> RuntimeLimits {
    RuntimeLimits {
        surfaces: BTreeMap::new(),
        quotas: BTreeMap::new(),
        tool_constraints: BTreeMap::new(),
    }
}

fn default_prompt_policy() -> PromptPolicy {
    PromptPolicy {
        default: PromptMode::Auto,
        by_operation: BTreeMap::new(),
    }
}

fn lifecycle_expired(lifecycle: &Lifecycle) -> bool {
    let Some(expires_at) = lifecycle.expires_at.as_deref() else {
        return false;
    };
    DateTime::parse_from_rfc3339(expires_at)
        .map(|expires_at| expires_at.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true)
}
