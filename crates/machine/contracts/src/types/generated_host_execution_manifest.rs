// AUTO-GENERATED from apxm.host-execution-manifest.v1; DO NOT EDIT.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostExecutionManifest {
    pub schema_version: serde_json::Value,
    pub semantic_owner: serde_json::Value,
    pub executable: serde_json::Value,
    pub protocol: serde_json::Value,
    pub readiness_contract: serde_json::Value,
    pub drain_quiescence_contract: serde_json::Value,
    pub admission_contract: serde_json::Value,
    pub evidence_contract: serde_json::Value,
    pub lifecycle: serde_json::Value,
}
