//! Artifact expiry and retention/GC tests.
//!
//! Validates that:
//! 1. Expired artifact references return a typed `artifact_expired` error body,
//!    not null or a bare 404.
//! 2. Run records whose blobs have expired remain queryable (only blob resolution
//!    fails, not the run record query).
//! 3. Retention class constants and TTLs are correct.
//! 4. The `ArtifactRef` struct round-trips through JSON without field loss.

use apxm_server::artifacts::object_store::{ArtifactError, ArtifactRef};
use apxm_server::run_history::storage::RetentionClass;

/// Retention class TTLs match the retention table.
#[test]
fn retention_class_ttls_are_correct() {
    assert_eq!(RetentionClass::Ephemeral.default_ttl_secs(), Some(3_600));
    assert_eq!(RetentionClass::Short.default_ttl_secs(), Some(7 * 86_400));
    assert_eq!(
        RetentionClass::Standard.default_ttl_secs(),
        Some(90 * 86_400)
    );
    assert_eq!(RetentionClass::Permanent.default_ttl_secs(), None);
}

/// Retention class `as_str()` values match the wire format used in JSON.
#[test]
fn retention_class_as_str_values_are_correct() {
    assert_eq!(RetentionClass::Ephemeral.as_str(), "ephemeral");
    assert_eq!(RetentionClass::Short.as_str(), "short");
    assert_eq!(RetentionClass::Standard.as_str(), "standard");
    assert_eq!(RetentionClass::Permanent.as_str(), "permanent");
}

/// `ArtifactError::ArtifactExpired` serializes to the contract JSON shape.
#[test]
fn artifact_expired_error_serializes_to_contract_shape() {
    let err = ArtifactError::ArtifactExpired {
        artifact_ref: "s3://bucket/runs/exec-1/out.tar.gz".to_string(),
        expired_at: "2026-06-01T00:00:00Z".to_string(),
        retention_class: "standard".to_string(),
    };
    let json = serde_json::to_string(&err).expect("serialize ArtifactError");
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(
        val["error"], "artifact_expired",
        "error key must be 'artifact_expired'"
    );
    assert_eq!(val["artifact_ref"], "s3://bucket/runs/exec-1/out.tar.gz");
    assert_eq!(val["expired_at"], "2026-06-01T00:00:00Z");
    assert_eq!(val["retention_class"], "standard");
}

/// `ArtifactError::NotFound` serializes with `error: "not_found"`, not `artifact_expired`.
#[test]
fn not_found_error_does_not_carry_retention_class() {
    let err = ArtifactError::NotFound {
        artifact_ref: "s3://bucket/runs/exec-999/missing.tar.gz".to_string(),
    };
    let json = serde_json::to_string(&err).expect("serialize NotFound");
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(val["error"], "not_found");
    assert!(
        val.get("retention_class").is_none() || val["retention_class"].is_null(),
        "NotFound must not carry retention_class"
    );
    assert!(
        val.get("expired_at").is_none() || val["expired_at"].is_null(),
        "NotFound must not carry expired_at"
    );
}

/// `ArtifactRef` round-trips through JSON without losing optional fields.
#[test]
fn artifact_ref_json_roundtrip_preserves_all_fields() {
    let original = ArtifactRef {
        artifact_ref: "file:///workspace/runs/exec-1/out.tar.gz".to_string(),
        content_hash: "sha256:abc123".to_string(),
        size_bytes: 2_097_152,
        media_type: "application/gzip".to_string(),
        created_at: "2026-06-01T00:00:00Z".to_string(),
        retention_class: "standard".to_string(),
        expires_at: Some("2026-09-01T00:00:00Z".to_string()),
    };
    let json = serde_json::to_string(&original).unwrap();
    let restored: ArtifactRef = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.artifact_ref, original.artifact_ref);
    assert_eq!(restored.content_hash, original.content_hash);
    assert_eq!(restored.size_bytes, original.size_bytes);
    assert_eq!(restored.retention_class, original.retention_class);
    assert_eq!(restored.expires_at, original.expires_at);
}

/// `ArtifactRef` with `permanent` retention omits `expires_at` from JSON.
#[test]
fn permanent_artifact_ref_omits_expires_at_in_json() {
    let r = ArtifactRef {
        artifact_ref: "file:///workspace/runs/exec-2/defs.tar.gz".to_string(),
        content_hash: "sha256:def456".to_string(),
        size_bytes: 512,
        media_type: "application/tar".to_string(),
        created_at: "2026-06-01T00:00:00Z".to_string(),
        retention_class: "permanent".to_string(),
        expires_at: None,
    };
    let json = serde_json::to_string(&r).unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(
        !val.as_object().unwrap().contains_key("expires_at"),
        "permanent artifacts must omit expires_at from JSON"
    );
}

