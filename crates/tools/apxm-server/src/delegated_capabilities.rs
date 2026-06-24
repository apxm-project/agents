use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use apxm_core::constants::orchestration::admission as orchestration_admission;
use apxm_core::types::{
    ApprovalMode, ApprovalPolicy, AuthContext, CapabilityOperation, CapabilityProvenance,
    CapabilityScope, CapabilityStatus, DELEGATED_CAPABILITY_SCHEMA_V1, Delegability,
    DelegatedCapabilityV1, Lifecycle, LifecycleBound, ResourceHandle, RuntimeLimits, Sensitivity,
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
pub(crate) struct DelegatedCapabilityStore {
    entries: Arc<DashMap<String, DelegatedCapabilityV1>>,
}

impl DelegatedCapabilityStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert(&self, capability: DelegatedCapabilityV1) {
        self.entries
            .insert(capability.capability_id.clone(), capability);
    }

    pub(crate) fn get_active(
        &self,
        capability_id: &str,
    ) -> Result<DelegatedCapabilityV1, ApiError> {
        let mut entry = self.entries.get_mut(capability_id).ok_or_else(|| {
            ApiError::admission_denied(format!(
                "delegated capability id '{capability_id}' is not recognized"
            ))
        })?;
        if entry.status != CapabilityStatus::Active {
            return Err(ApiError::admission_denied(format!(
                "delegated capability id '{capability_id}' is {:?}",
                entry.status
            )));
        }
        if lifecycle_expired(&entry.lifecycle) {
            entry.status = CapabilityStatus::Expired;
            return Err(ApiError::admission_denied(format!(
                "delegated capability id '{capability_id}' is expired"
            )));
        }
        Ok(entry.clone())
    }

    pub(crate) fn resolve(
        &self,
        capability_ids: &HashSet<String>,
    ) -> Result<ResolvedDelegatedCapabilities, ApiError> {
        let mut capabilities = Vec::with_capacity(capability_ids.len());
        for capability_id in capability_ids {
            if !capability_id.starts_with("cap_") {
                return Err(ApiError::admission_denied(format!(
                    "delegated_capability_ids must contain APXM-minted cap_* ids; '{capability_id}' is not a delegated capability id"
                )));
            }
            capabilities.push(self.get_active(capability_id)?);
        }
        Ok(ResolvedDelegatedCapabilities::new(capabilities))
    }

    fn revoke(&self, capability_id: &str) -> Result<DelegatedCapabilityV1, ApiError> {
        let mut entry = self.entries.get_mut(capability_id).ok_or_else(|| {
            ApiError::not_found(format!(
                "delegated capability id '{capability_id}' is not recognized"
            ))
        })?;
        entry.status = CapabilityStatus::Revoked;
        Ok(entry.clone())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedDelegatedCapabilities {
    capabilities: Vec<DelegatedCapabilityV1>,
}

impl ResolvedDelegatedCapabilities {
    fn new(capabilities: Vec<DelegatedCapabilityV1>) -> Self {
        Self { capabilities }
    }

    pub(crate) fn empty() -> Self {
        Self {
            capabilities: Vec::new(),
        }
    }

    pub(crate) fn admits_mutating_tool(&self, tool_binding: &str) -> bool {
        self.capabilities.iter().any(|capability| {
            capability.status == CapabilityStatus::Active
                && capability.tool_binding == tool_binding
                && capability
                    .operations
                    .iter()
                    .copied()
                    .any(CapabilityOperation::is_mutating)
                && !lifecycle_expired(&capability.lifecycle)
        })
    }

    pub(crate) fn to_runtime_metadata_json(&self) -> Result<String, ApiError> {
        let grants: Vec<RuntimeDelegatedCapability> = self
            .capabilities
            .iter()
            .map(RuntimeDelegatedCapability::from)
            .collect();
        serde_json::to_string(&grants).map_err(|error| {
            ApiError::internal_message(format!(
                "failed to serialize delegated capability metadata: {error}"
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RuntimeDelegatedCapability {
    pub(crate) capability_id: String,
    pub(crate) tool_binding: String,
    pub(crate) operations: Vec<CapabilityOperation>,
    pub(crate) expires_at: Option<String>,
    pub(crate) status: CapabilityStatus,
}

impl From<&DelegatedCapabilityV1> for RuntimeDelegatedCapability {
    fn from(capability: &DelegatedCapabilityV1) -> Self {
        Self {
            capability_id: capability.capability_id.clone(),
            tool_binding: capability.tool_binding.clone(),
            operations: capability.operations.clone(),
            expires_at: capability.lifecycle.expires_at.clone(),
            status: capability.status,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DelegateCapabilityRequest {
    #[serde(default)]
    pub(crate) template_key: Option<String>,
    pub(crate) tool_binding: String,
    #[serde(default)]
    pub(crate) operations: Vec<CapabilityOperation>,
    #[serde(default)]
    pub(crate) resource_handle: Option<ResourceHandle>,
    #[serde(default)]
    pub(crate) auth_context: Option<AuthContext>,
    #[serde(default)]
    pub(crate) scope: Option<CapabilityScope>,
    #[serde(default)]
    pub(crate) runtime_limits: Option<RuntimeLimits>,
    #[serde(default)]
    pub(crate) sensitivity: Option<Sensitivity>,
    #[serde(default)]
    pub(crate) approval_policy: Option<ApprovalPolicy>,
    #[serde(default)]
    pub(crate) delegability: Option<Delegability>,
    #[serde(default)]
    pub(crate) lifecycle: Option<Lifecycle>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) trace_tags: BTreeMap<String, String>,
}

pub(crate) async fn delegate_capability(
    State(state): State<AppState>,
    Json(req): Json<DelegateCapabilityRequest>,
) -> Result<Json<DelegatedCapabilityV1>, ApiError> {
    let capability = mint_delegated_capability(&state, req)?;
    state.delegated_capabilities.insert(capability.clone());
    Ok(Json(capability))
}

pub(crate) async fn revoke_capability(
    State(state): State<AppState>,
    Path(capability_id): Path<String>,
) -> Result<Json<DelegatedCapabilityV1>, ApiError> {
    Ok(Json(state.delegated_capabilities.revoke(&capability_id)?))
}

fn mint_delegated_capability(
    state: &AppState,
    req: DelegateCapabilityRequest,
) -> Result<DelegatedCapabilityV1, ApiError> {
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
    let capability = DelegatedCapabilityV1 {
        schema_version: DELEGATED_CAPABILITY_SCHEMA_V1.to_string(),
        capability_id: format!("cap_{}", uuid::Uuid::new_v4().simple()),
        template_key: req.template_key,
        tool_binding,
        resource_handle: req.resource_handle.unwrap_or_else(default_resource_handle),
        operations,
        auth_context: req.auth_context.unwrap_or_else(default_auth_context),
        scope: req.scope.unwrap_or_else(default_scope),
        runtime_limits: req.runtime_limits.unwrap_or_else(default_runtime_limits),
        sensitivity: req.sensitivity.unwrap_or(Sensitivity::Internal),
        approval_policy: req.approval_policy.unwrap_or_else(default_approval_policy),
        delegability: req.delegability.unwrap_or(Delegability::AttenuateOnly),
        lifecycle,
        provenance: CapabilityProvenance {
            minted_by: "apxm-server".to_string(),
            policy_version: POLICY_VERSION.to_string(),
            request_id: format!("req_{}", uuid::Uuid::new_v4().simple()),
        },
        status: CapabilityStatus::Active,
        parent_capability_id: None,
        description: req.description,
        trace_tags: req.trace_tags,
    };
    capability
        .validate()
        .map_err(|error| ApiError::bad_request(format!("invalid delegated capability: {error}")))?;
    Ok(capability)
}

fn default_operations(state: &AppState, tool_binding: &str) -> Vec<CapabilityOperation> {
    if tool_binding == orchestration_admission::SPAWN_AGENT
        || tool_binding == orchestration_admission::SPAWN_TEAM
    {
        return vec![CapabilityOperation::Execute];
    }
    if state.runtime.capability_system().is_read_only(tool_binding) {
        vec![CapabilityOperation::Read]
    } else {
        vec![CapabilityOperation::Write]
    }
}

fn default_lifecycle(operations: &[CapabilityOperation]) -> Lifecycle {
    if operations
        .iter()
        .copied()
        .any(CapabilityOperation::is_mutating)
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

fn default_resource_handle() -> ResourceHandle {
    ResourceHandle {
        kind: "tool_binding".to_string(),
        uri: Some("apxm://capability-template".to_string()),
        attributes: BTreeMap::new(),
    }
}

fn default_auth_context() -> AuthContext {
    AuthContext {
        principal: "local-user".to_string(),
        delegation_mode: "on_behalf_of".to_string(),
        credential_ref: None,
    }
}

fn default_scope() -> CapabilityScope {
    CapabilityScope {
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

fn default_approval_policy() -> ApprovalPolicy {
    ApprovalPolicy {
        default: ApprovalMode::Auto,
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
