//! Execution-scoped capability admission helpers shared by operation handlers.

use crate::metadata_keys;
use apxm_core::types::{GrantStatus, PermissionOperation, RuntimeCapabilityGrant};
use chrono::{DateTime, Utc};

/// Returns true when runtime `capability_grants` metadata admits a direct write
/// to `capability_binding`. Missing, malformed, expired, or non-mutating grants fail closed.
pub(crate) fn capability_grant_admits_write(
    metadata: Option<&str>,
    capability_binding: &str,
) -> bool {
    let Some(metadata) = metadata else {
        return false;
    };
    let Ok(grants) = serde_json::from_str::<Vec<RuntimeCapabilityGrant>>(metadata) else {
        return false;
    };
    grants.into_iter().any(|grant| {
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
    metadata: &std::collections::HashMap<String, String>,
    capability_binding: &str,
) -> bool {
    capability_grant_admits_write(
        metadata
            .get(metadata_keys::CAPABILITY_GRANTS)
            .map(String::as_str),
        capability_binding,
    )
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
    use super::capability_grant_admits_write;

    #[test]
    fn admits_write_requires_runtime_minted_capability_binding_grant() {
        let metadata = serde_json::json!([{
            "grant_id": "grant_fixture",
            "capability_binding": "fixture.write",
            "operations": ["write"],
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
            "expires_at": "2000-01-01T00:00:00Z",
            "status": "active"
        }])
        .to_string();
        let read_only = serde_json::json!([{
            "grant_id": "grant_fixture",
            "capability_binding": "fixture.write",
            "operations": ["read"],
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
}
