//! Execution-scoped capability admission helpers shared by operation handlers.

use std::collections::HashMap;

use apxm_capability_iface::{CapabilityGrantScopeRequirements, RuntimeCapability};
use apxm_core::types::values::Value;
use apxm_core::types::{GrantStatus, PermissionOperation, RuntimeCapabilityGrant};
use chrono::{DateTime, Utc};

use crate::metadata_keys;

/// Returns true when runtime `capability_grants` metadata admits a direct write
/// to `capability_binding`. Missing, malformed, expired, or non-mutating grants fail closed.
pub(crate) fn capability_grant_admits_write(
    metadata: Option<&str>,
    capability_binding: &str,
) -> bool {
    parse_runtime_grants(metadata).iter().any(|grant| {
        grant.status == GrantStatus::Active
            && grant.capability_binding == capability_binding
            && grant
                .operations
                .iter()
                .copied()
                .any(PermissionOperation::is_mutating)
            && !capability_grant_expired(grant.expires_at.as_deref())
    })
}

pub(crate) fn metadata_admits_write(
    metadata: &HashMap<String, String>,
    capability_binding: &str,
) -> bool {
    capability_grant_admits_write(
        metadata
            .get(metadata_keys::CAPABILITY_GRANTS)
            .map(String::as_str),
        capability_binding,
    )
}

/// Returns grants that satisfy a capability's metadata-declared authority.
///
/// The matching rules are intentionally data-driven: resource kind, selector
/// names, request bindings, and runtime quota names all come from capability
/// metadata. A capability implementation receives these trusted projections
/// only after this function admits the normal invocation path.
pub(crate) fn metadata_matching_capability_grants(
    metadata: &HashMap<String, String>,
    capability_binding: &str,
    capability: &RuntimeCapability,
    args: &HashMap<String, Value>,
) -> Vec<RuntimeCapabilityGrant> {
    let Some(raw) = metadata.get(metadata_keys::CAPABILITY_GRANTS) else {
        return Vec::new();
    };
    parse_runtime_grants(Some(raw))
        .into_iter()
        .filter(|grant| {
            capability_grant_matches_capability(grant, capability_binding, capability, args)
        })
        .collect()
}

/// Returns whether metadata carries a grant that satisfies the capability's
/// declared grant operations, selectors, and runtime quotas.
pub(crate) fn metadata_admits_capability(
    metadata: &HashMap<String, String>,
    capability_binding: &str,
    capability: &RuntimeCapability,
    args: &HashMap<String, Value>,
) -> bool {
    !metadata_matching_capability_grants(metadata, capability_binding, capability, args).is_empty()
}

/// Return the active, matching mutating grants from runtime metadata.
///
/// Parsing and eligibility intentionally match `capability_grant_admits_write`
/// so an effect preparation cannot cite a grant that would not admit the call.
pub(crate) fn metadata_matching_write_grants(
    metadata: &HashMap<String, String>,
    capability_binding: &str,
) -> Vec<String> {
    let Some(raw) = metadata.get(metadata_keys::CAPABILITY_GRANTS) else {
        return Vec::new();
    };
    parse_runtime_grants(Some(raw))
        .into_iter()
        .filter(|grant| {
            grant.status == GrantStatus::Active
                && grant.capability_binding == capability_binding
                && grant
                    .operations
                    .iter()
                    .copied()
                    .any(PermissionOperation::is_mutating)
                && !capability_grant_expired(grant.expires_at.as_deref())
        })
        .map(|grant| grant.grant_id)
        .collect()
}

fn capability_grant_matches_capability(
    grant: &RuntimeCapabilityGrant,
    capability_binding: &str,
    capability: &RuntimeCapability,
    args: &HashMap<String, Value>,
) -> bool {
    grant.status == GrantStatus::Active
        && grant.capability_binding == capability_binding
        && !capability_grant_expired(grant.expires_at.as_deref())
        && capability
            .required_grant_operations
            .iter()
            .all(|operation| grant.operations.contains(operation))
        && grant_scope_matches(grant, &capability.grant_scope_requirements, args)
}

fn grant_scope_matches(
    grant: &RuntimeCapabilityGrant,
    requirements: &CapabilityGrantScopeRequirements,
    args: &HashMap<String, Value>,
) -> bool {
    if let Some(resource_kind) = &requirements.resource_kind
        && grant.resource.kind != *resource_kind
    {
        return false;
    }

    for requirement in &requirements.scope_selectors {
        let Some(scope_value) = grant.scope.selectors.get(&requirement.selector) else {
            return false;
        };
        let Some(allowed) = json_string_values(scope_value) else {
            return false;
        };
        if allowed.is_empty() {
            return false;
        }
        let Some(argument) = &requirement.argument else {
            continue;
        };
        let Some(requested) = args.get(argument) else {
            continue;
        };
        let Some(requested) = runtime_string_values(requested) else {
            return false;
        };
        if requested.is_empty() || requested.iter().any(|value| !allowed.contains(value)) {
            return false;
        }
    }

    for requirement in &requirements.runtime_quotas {
        let Some(limit) = grant.runtime_limits.quotas.get(&requirement.quota) else {
            return false;
        };
        let Some(argument) = &requirement.argument else {
            continue;
        };
        let Some(requested) = args.get(argument) else {
            continue;
        };
        let Some(requested) = requested.as_u64() else {
            return false;
        };
        if requested > *limit {
            return false;
        }
    }

    true
}

fn parse_runtime_grants(metadata: Option<&str>) -> Vec<RuntimeCapabilityGrant> {
    metadata
        .and_then(|raw| serde_json::from_str::<Vec<RuntimeCapabilityGrant>>(raw).ok())
        .unwrap_or_default()
}

fn json_string_values(value: &serde_json::Value) -> Option<Vec<&str>> {
    match value {
        serde_json::Value::String(value) => Some(vec![value]),
        serde_json::Value::Array(values) => values.iter().map(serde_json::Value::as_str).collect(),
        _ => None,
    }
}

fn runtime_string_values(value: &Value) -> Option<Vec<&str>> {
    match value {
        Value::String(value) => Some(vec![value]),
        Value::Array(values) => values.iter().map(Value::as_str).collect(),
        _ => None,
    }
}

fn capability_grant_expired(expires_at: Option<&str>) -> bool {
    let Some(expires_at) = expires_at else {
        return false;
    };
    DateTime::parse_from_rfc3339(expires_at)
        .map(|expires_at| expires_at.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use apxm_capability_iface::{
        CapabilityGrantRuntimeQuota, CapabilityGrantScopeRequirements, CapabilityGrantScopeSelector,
    };
    use apxm_core::types::values::Value;
    use apxm_core::types::{PermissionOperation, RuntimeCapabilityGrant};

    use super::{
        capability_grant_admits_write, metadata_admits_capability,
        metadata_matching_capability_grants,
    };

    const CAPABILITY: &str = "fixture.scoped_read";
    const RESOURCE_KIND: &str = "fixture_catalog";
    const ROOT_SELECTOR: &str = "catalog_roots";
    const SKILL_SELECTOR: &str = "skill_ids";
    const BYTE_QUOTA: &str = "max_result_bytes";
    const ROOT_ARGUMENT: &str = "catalog_root";
    const SKILL_ARGUMENT: &str = "skill_id";
    const BYTE_ARGUMENT: &str = "max_result_bytes";

    fn scoped_capability() -> apxm_capability_iface::RuntimeCapability {
        apxm_capability_iface::RuntimeCapability::new(
            CAPABILITY,
            "Scoped fixture read",
            serde_json::json!({"type": "object"}),
        )
        .with_read_only()
        .with_required_grant_operations(vec![PermissionOperation::Read])
        .with_grant_scope_requirements(CapabilityGrantScopeRequirements {
            resource_kind: Some(RESOURCE_KIND.to_string()),
            scope_selectors: vec![
                CapabilityGrantScopeSelector {
                    selector: ROOT_SELECTOR.to_string(),
                    argument: Some(ROOT_ARGUMENT.to_string()),
                },
                CapabilityGrantScopeSelector {
                    selector: SKILL_SELECTOR.to_string(),
                    argument: Some(SKILL_ARGUMENT.to_string()),
                },
            ],
            runtime_quotas: vec![CapabilityGrantRuntimeQuota {
                quota: BYTE_QUOTA.to_string(),
                argument: Some(BYTE_ARGUMENT.to_string()),
            }],
        })
    }

    fn scoped_grant() -> RuntimeCapabilityGrant {
        RuntimeCapabilityGrant {
            grant_id: "grant_scoped_fixture".to_string(),
            capability_binding: CAPABILITY.to_string(),
            operations: vec![PermissionOperation::Read],
            resource: apxm_core::types::ResourceHandle {
                kind: RESOURCE_KIND.to_string(),
                uri: Some("catalog://fixture".to_string()),
                attributes: BTreeMap::new(),
            },
            scope: apxm_core::types::PermissionScope {
                kind: "catalog".to_string(),
                boundary: "fixture".to_string(),
                selectors: BTreeMap::from([
                    (
                        ROOT_SELECTOR.to_string(),
                        serde_json::json!(["catalog://fixture"]),
                    ),
                    (
                        SKILL_SELECTOR.to_string(),
                        serde_json::json!(["fixture-skill"]),
                    ),
                ]),
            },
            runtime_limits: apxm_core::types::RuntimeLimits {
                surfaces: BTreeMap::new(),
                quotas: BTreeMap::from([(BYTE_QUOTA.to_string(), 64)]),
                tool_constraints: BTreeMap::new(),
            },
            expires_at: None,
            status: apxm_core::types::GrantStatus::Active,
        }
    }

    fn grant_metadata(grant: RuntimeCapabilityGrant) -> HashMap<String, String> {
        HashMap::from([(
            crate::metadata_keys::CAPABILITY_GRANTS.to_string(),
            serde_json::to_string(&vec![grant]).expect("serialize fixture grant"),
        )])
    }

    fn scoped_args() -> HashMap<String, Value> {
        HashMap::from([
            (
                ROOT_ARGUMENT.to_string(),
                Value::String("catalog://fixture".to_string()),
            ),
            (
                SKILL_ARGUMENT.to_string(),
                Value::String("fixture-skill".to_string()),
            ),
            (BYTE_ARGUMENT.to_string(), Value::from(64_i64)),
        ])
    }

    #[test]
    fn admits_write_requires_runtime_minted_capability_binding_grant() {
        let metadata = serde_json::json!([{
            "grant_id": "grant_fixture",
            "capability_binding": "fixture.write",
            "operations": ["write"],
            "resource": {"kind": "fixture", "uri": "fixture://write"},
            "scope": {"kind": "fixture", "boundary": "fixture"},
            "runtime_limits": {},
            "expires_at": null,
            "status": "active"
        }])
        .to_string();

        assert!(capability_grant_admits_write(
            Some(&metadata),
            "fixture.write"
        ));
        assert!(!capability_grant_admits_write(
            Some(&metadata),
            "other.write"
        ));
        assert!(!capability_grant_admits_write(
            Some("not capability grant metadata"),
            "fixture.write"
        ));
    }

    #[test]
    fn admits_write_rejects_expired_or_non_mutating_grants() {
        let expired = serde_json::json!([{
            "grant_id": "grant_fixture",
            "capability_binding": "fixture.write",
            "operations": ["write"],
            "resource": {"kind": "fixture", "uri": "fixture://write"},
            "scope": {"kind": "fixture", "boundary": "fixture"},
            "runtime_limits": {},
            "expires_at": "2000-01-01T00:00:00Z",
            "status": "active"
        }])
        .to_string();
        let read_only = serde_json::json!([{
            "grant_id": "grant_fixture",
            "capability_binding": "fixture.write",
            "operations": ["read"],
            "resource": {"kind": "fixture", "uri": "fixture://write"},
            "scope": {"kind": "fixture", "boundary": "fixture"},
            "runtime_limits": {},
            "expires_at": null,
            "status": "active"
        }])
        .to_string();

        assert!(!capability_grant_admits_write(
            Some(&expired),
            "fixture.write"
        ));
        assert!(!capability_grant_admits_write(
            Some(&read_only),
            "fixture.write"
        ));
    }

    #[test]
    fn scoped_grants_require_matching_operation_resource_selectors_and_quota() {
        let capability = scoped_capability();
        let metadata = grant_metadata(scoped_grant());
        let args = scoped_args();

        assert!(metadata_admits_capability(
            &metadata,
            CAPABILITY,
            &capability,
            &args
        ));
        let contexts =
            metadata_matching_capability_grants(&metadata, CAPABILITY, &capability, &args);
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].grant_id, "grant_scoped_fixture");
    }

    #[test]
    fn scoped_grants_fail_closed_for_wrong_or_missing_authority() {
        let capability = scoped_capability();
        let mut args = scoped_args();
        let metadata = grant_metadata(scoped_grant());

        args.insert(
            SKILL_ARGUMENT.to_string(),
            Value::String("other-skill".to_string()),
        );
        assert!(!metadata_admits_capability(
            &metadata,
            CAPABILITY,
            &capability,
            &args
        ));

        let mut grant = scoped_grant();
        grant.operations = vec![PermissionOperation::Search];
        assert!(!metadata_admits_capability(
            &grant_metadata(grant),
            CAPABILITY,
            &capability,
            &scoped_args()
        ));

        let mut grant = scoped_grant();
        grant.runtime_limits.quotas.clear();
        assert!(!metadata_admits_capability(
            &grant_metadata(grant),
            CAPABILITY,
            &capability,
            &scoped_args()
        ));
    }

    #[test]
    fn scoped_grants_reject_requested_results_above_the_granted_quota() {
        let capability = scoped_capability();
        let metadata = grant_metadata(scoped_grant());
        let mut args = scoped_args();
        args.insert(BYTE_ARGUMENT.to_string(), Value::from(65_i64));

        assert!(!metadata_admits_capability(
            &metadata,
            CAPABILITY,
            &capability,
            &args
        ));
    }
}
