//! Object-store client boundary for run blobs and Runner artifacts.
//!
//! ## Contract
//!
//! Large blobs (>1 MiB runner artifacts, rollout JSONL chunks) are stored as
//! object references rather than embedded in the run record.  The server is the
//! sole writer; callers resolve blobs through the server's
//! `GET /v1/runs/{execution_id}/artifacts/{name}` endpoint.  Direct volume
//! access from callers is prohibited — the blob URI is opaque from the caller's
//! perspective.
//!
//! ## Blob reference format
//!
//! Each reference is a JSON object embedded in the run record's `artifact_refs`
//! array:
//!
//! ```json
//! {
//!   "artifact_ref": "s3://bucket/runs/exec-id/artifact.tar.gz",
//!   "content_hash": "sha256:<hex>",
//!   "size_bytes": 12345678,
//!   "media_type": "application/gzip",
//!   "created_at": "2026-06-01T00:00:00Z",
//!   "retention_class": "standard",
//!   "expires_at": "2026-09-01T00:00:00Z"
//! }
//! ```
//!
//! ## v0 backing
//!
//! In v0 (single-host compose) blobs are stored as files on the `apxm-runs`
//! named Docker volume.  The `artifact_ref` is a `file://` URI resolved by the
//! server; the volume is not exposed to callers.
//!
//! ## Production target
//!
//! S3 or MinIO.  The client boundary (`ObjectStoreClient`) abstracts the
//! backing so the resolution path does not change when the backing is swapped.
//! The `S3Client` variant requires `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
//! and `APXM_ARTIFACT_BUCKET` to be set in the server environment.
//!
//! ## Typed expiry
//!
//! When a caller requests a blob whose `expires_at` is in the past the server
//! returns:
//!
//! ```json
//! {
//!   "error": "artifact_expired",
//!   "artifact_ref": "<ref>",
//!   "expired_at": "<ISO-8601>",
//!   "retention_class": "<class>"
//! }
//! ```
//!
//! Returning `null` or a bare 404 without an expiry reason is a contract
//! violation.

use serde::{Deserialize, Serialize};

/// Opaque blob reference stored in a run record row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// Opaque URI: `s3://`, `gs://`, or `file://` for v0.
    pub artifact_ref: String,
    /// `sha256:<hex>` of the raw (uncompressed) bytes.
    pub content_hash: String,
    /// Pre-compressed size in bytes.
    pub size_bytes: u64,
    /// MIME type of the artifact.
    pub media_type: String,
    /// ISO-8601 UTC creation timestamp.
    pub created_at: String,
    /// Retention class: `ephemeral`, `short`, `standard`, or `permanent`.
    pub retention_class: String,
    /// ISO-8601 UTC expiry time.  `None` for `permanent` class.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// Typed error returned when a blob resolution fails.
///
/// Callers must distinguish `ArtifactExpired` (valid ref, TTL elapsed) from
/// `NotFound` (unknown ref).  A bare 404 or null for an expired artifact is
/// a contract violation.
#[derive(Debug, Serialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum ArtifactError {
    /// The artifact_ref was not found (unknown ref or already GC'd).
    NotFound { artifact_ref: String },
    /// The artifact_ref was valid but its retention TTL has elapsed.
    ArtifactExpired {
        artifact_ref: String,
        expired_at: String,
        retention_class: String,
    },
    /// Backend I/O error.
    Backend { message: String },
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { artifact_ref } => write!(f, "artifact not found: {artifact_ref}"),
            Self::ArtifactExpired {
                artifact_ref,
                expired_at,
                retention_class,
            } => write!(
                f,
                "artifact_expired: ref={artifact_ref} expired_at={expired_at} \
                 retention_class={retention_class}"
            ),
            Self::Backend { message } => write!(f, "object store backend error: {message}"),
        }
    }
}

impl std::error::Error for ArtifactError {}

/// Abstraction over the object-store backing.
///
/// Call `ObjectStoreClient::resolve` to read a blob by its opaque `artifact_ref`
/// URI.  The implementation selects the correct backing at runtime.
pub enum ObjectStoreClient {
    /// Local file system (v0: files on the `apxm-runs` volume).
    LocalFs { runs_root: std::path::PathBuf },
    /// S3-compatible object store (production target).
    S3 {
        bucket: String,
        /// AWS region (e.g. `us-east-1`).
        region: String,
    },
}

impl ObjectStoreClient {
    /// Resolve an artifact reference to raw bytes.
    ///
    /// Returns `Err(ArtifactError::ArtifactExpired)` when the blob's
    /// `expires_at` is in the past, `Err(ArtifactError::NotFound)` when the
    /// ref is unknown, and `Err(ArtifactError::Backend)` for I/O failures.
    pub fn resolve(&self, artifact: &ArtifactRef) -> Result<Vec<u8>, ArtifactError> {
        // Check expiry before hitting the backend.
        if let Some(ref expires_at) = artifact.expires_at
            && is_expired(expires_at) {
                return Err(ArtifactError::ArtifactExpired {
                    artifact_ref: artifact.artifact_ref.clone(),
                    expired_at: expires_at.clone(),
                    retention_class: artifact.retention_class.clone(),
                });
            }

        match self {
            Self::LocalFs { runs_root } => {
                let path = uri_to_local_path(&artifact.artifact_ref, runs_root)
                    .map_err(|e| ArtifactError::NotFound { artifact_ref: e })?;
                std::fs::read(&path).map_err(|e| ArtifactError::Backend {
                    message: format!("read {}: {e}", path.display()),
                })
            }
            Self::S3 { bucket, region } => {
                // Production implementation: use the AWS SDK or a minimal
                // HTTP call to `s3.<region>.amazonaws.com`.
                // Stub: returns Backend error until wired.
                Err(ArtifactError::Backend {
                    message: format!(
                        "S3 client not wired: bucket={bucket} region={region} \
                         ref={}",
                        artifact.artifact_ref
                    ),
                })
            }
        }
    }
}

/// Parse a `file:///path/to/artifact` URI to an absolute local path.
fn uri_to_local_path(uri: &str, runs_root: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if let Some(rel) = uri.strip_prefix("file://") {
        Ok(std::path::PathBuf::from(rel))
    } else if uri.starts_with('/') {
        Ok(std::path::PathBuf::from(uri))
    } else {
        // Relative URI: resolve against runs_root.
        Ok(runs_root.join(uri))
    }
}

/// Returns true when the RFC-3339 timestamp is in the past.
fn is_expired(expires_at: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(expires_at)
        .is_ok_and(|timestamp| timestamp.with_timezone(&chrono::Utc) <= chrono::Utc::now())
}
