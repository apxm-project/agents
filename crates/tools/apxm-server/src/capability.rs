use std::collections::HashMap;
use std::sync::Arc;

use apxm_core::error::RuntimeError;
use apxm_core::types::values::Value;
use apxm_runtime::capability::executor::{CapabilityExecutor, CapabilityResult};
use apxm_runtime::capability::metadata::CapabilityMetadata;
use async_trait::async_trait;
use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use tracing::info;

use crate::error::ApiError;
use crate::state::AppState;
use crate::types::responses::{CapabilityEntry, OkAckName};

/// A capability that forwards invocations to an external HTTP endpoint.
///
/// The endpoint receives the capability arguments as a JSON object via POST and
/// must return a JSON object with a `"result"` key (or any parseable JSON value).
#[derive(Clone)]
pub(crate) struct HttpCapability {
    pub(crate) metadata: CapabilityMetadata,
    pub(crate) endpoint: String,
    pub(crate) timeout_ms: u64,
    pub(crate) client: reqwest::Client,
}

#[async_trait]
impl CapabilityExecutor for HttpCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        // Serialize arguments to JSON
        let body: serde_json::Map<String, JsonValue> = args
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.to_json()
                        .unwrap_or_else(|_| JsonValue::String(v.to_string())),
                )
            })
            .collect();

        let cap_err = |msg: String| RuntimeError::Capability {
            capability: self.metadata.name.clone(),
            message: msg,
        };

        let resp = self
            .client
            .post(&self.endpoint)
            .timeout(std::time::Duration::from_millis(self.timeout_ms))
            .json(&body)
            .send()
            .await
            .map_err(|e| cap_err(format!("request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(cap_err(format!("endpoint returned {status}: {text}")));
        }

        let result: JsonValue = resp
            .json()
            .await
            .map_err(|e| cap_err(format!("response parse failed: {e}")))?;

        // Unwrap a `{"result": ...}` envelope if present, otherwise use the raw response
        let inner = if let Some(r) = result.get("result") {
            r.clone()
        } else {
            result
        };

        Value::try_from(inner).map_err(|e| cap_err(format!("value conversion failed: {e}")))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegisterCapabilityRequest {
    name: String,
    description: String,
    #[serde(default)]
    parameters_schema: JsonValue,
    /// Mark capability as read-only for raw workflow execution admission.
    #[serde(default)]
    read_only: bool,
    /// Optional least-privilege tool groups exposed to LLM/tool admission.
    #[serde(default)]
    groups: Vec<String>,
    /// Optional tags for inventory and routing.
    #[serde(default)]
    tags: Vec<String>,
    /// If set, an `HttpCapability` is created that POSTs to this URL.
    /// Takes priority over `static_response`.
    #[serde(default)]
    endpoint: Option<String>,
    /// Timeout in milliseconds for HTTP capability calls (default: 30 000).
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Fallback: return a fixed static value (used when `endpoint` is absent).
    #[serde(default)]
    static_response: JsonValue,
}

#[derive(Clone)]
pub(crate) struct StaticCapability {
    pub(crate) metadata: CapabilityMetadata,
    pub(crate) static_response: Value,
}

#[async_trait]
impl CapabilityExecutor for StaticCapability {
    async fn execute(&self, _args: HashMap<String, Value>) -> CapabilityResult<Value> {
        Ok(self.static_response.clone())
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

pub(crate) async fn list_capabilities(
    State(state): State<AppState>,
) -> Result<Json<Vec<CapabilityEntry>>, ApiError> {
    let caps: Vec<CapabilityEntry> = state
        .runtime
        .capability_system()
        .list_capabilities()
        .iter()
        .map(|m| CapabilityEntry {
            name: m.name.clone(),
            description: m.description.clone(),
            parameters_schema: m.parameters_schema.clone(),
        })
        .collect();
    Ok(Json(caps))
}

pub(crate) async fn register_capability(
    State(state): State<AppState>,
    Json(req): Json<RegisterCapabilityRequest>,
) -> Result<Json<OkAckName>, ApiError> {
    let mut metadata = CapabilityMetadata::new(
        req.name.clone(),
        req.description.clone(),
        req.parameters_schema,
    );
    if req.read_only {
        metadata = metadata.with_read_only();
    }
    if !req.groups.is_empty() {
        metadata = metadata.with_groups(req.groups.clone());
    }
    if !req.tags.is_empty() {
        metadata = metadata.with_tags(req.tags.clone());
    }

    let capability: Arc<dyn CapabilityExecutor> = if let Some(endpoint) = req.endpoint {
        // HTTP capability — forwards invocations to external server
        info!(name = %req.name, endpoint = %endpoint, "registering HTTP capability");
        Arc::new(HttpCapability {
            metadata,
            endpoint,
            timeout_ms: req.timeout_ms.unwrap_or(30_000),
            client: reqwest::Client::new(),
        })
    } else {
        // Static capability — always returns the same configured value.
        let response_value = Value::try_from(req.static_response)
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        info!(name = %req.name, "registering static capability");
        Arc::new(StaticCapability {
            metadata,
            static_response: response_value,
        })
    };

    state
        .runtime
        .capability_system()
        .register(capability)
        .map_err(ApiError::runtime)?;
    Ok(Json(OkAckName::new(req.name)))
}
