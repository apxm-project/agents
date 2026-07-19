//! Shared `apxm.contract-common.v1` envelope primitives consumed by the owned
//! runtime schemas. These mirror the constitution envelope exactly; they are not
//! redefinitions of any owner schema.

use serde::{Deserialize, Serialize};

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
