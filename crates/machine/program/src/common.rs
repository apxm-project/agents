//! Shared `apxm.contract-common.v1` envelope primitives consumed by the owned
//! runtime schemas. These mirror the constitution envelope exactly; they are not
//! redefinitions of any owner schema.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Stable identity of the compiler contract that mints execution lineage.
///
/// This is intentionally an opaque product-neutral compiler identity. A
/// release may replace it with a digest-pinned implementation identity without
/// changing the lineage wire shape.
pub const EXECUTION_LINEAGE_COMPILER_IDENTITY: &str = "apxm.compiler/pipeline.v1";

/// Mint an opaque digest-bound execution lineage reference.
///
/// Length-prefixed components keep the binding unambiguous even when callers
/// supply references from different digest namespaces. The result is never a
/// source, artifact, or workflow identifier: it is only a stable commitment to
/// the exact source digest, canonical AIR/artifact digest, and compiler
/// identity used together.
#[must_use]
pub fn execution_lineage_ref(
    source_digest: &str,
    canonical_artifact_digest: &str,
    compiler_identity: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"apxm.execution-lineage.v1\0");
    for component in [source_digest, canonical_artifact_digest, compiler_identity] {
        hasher.update((component.len() as u64).to_be_bytes());
        hasher.update(component.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_lineage_is_stable_and_bound_to_each_component() {
        let base = execution_lineage_ref("source-a", "sha256:air-a", "compiler-a");
        assert_eq!(
            base,
            execution_lineage_ref("source-a", "sha256:air-a", "compiler-a")
        );
        assert_ne!(
            base,
            execution_lineage_ref("source-b", "sha256:air-a", "compiler-a")
        );
        assert_ne!(
            base,
            execution_lineage_ref("source-a", "sha256:air-b", "compiler-a")
        );
        assert_ne!(
            base,
            execution_lineage_ref("source-a", "sha256:air-a", "compiler-b")
        );
        assert!(base.starts_with("sha256:"));
    }
}

/// `apxm.contract-common.v1#/$defs/TypedRef`
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedRef {
    pub ref_type: String,
    #[serde(rename = "ref")]
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

/// `apxm.contract-common.v1#/$defs/IdempotencyKey`
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdempotencyKey {
    pub key_id: String,
    pub scope_ref: String,
    pub request_digest: String,
}

/// The closed `apxm.contract-common.v1#/$defs/TypedErrorEnvelope` category set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Validation,
    Admission,
    Authority,
    Configuration,
    Unavailable,
    Conflict,
    OutcomeUnknown,
    Internal,
}

/// `apxm.contract-common.v1#/$defs/TypedErrorEnvelope`
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedErrorEnvelope {
    pub error_id: String,
    pub category: ErrorCategory,
    pub code_ref: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details_digest: Option<String>,
}
