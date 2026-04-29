//! `/api/backends` — backend and model listing from the typed config store.

use axum::extract::Query;
use axum::response::{IntoResponse, Json};
use serde::Deserialize;
use serde::Serialize;

use apxm_backends::llm::{BackendConfig, ModelConfig};
use apxm_credentials::backend::BackendStore;

use crate::error::{ApiResult, AppError};

#[derive(Deserialize)]
pub struct BackendsQuery {
    #[serde(default)]
    pub probe: Option<bool>,
}

#[derive(Serialize, Clone)]
pub struct ModelDto {
    pub id: String,
    pub aliases: Vec<String>,
    pub context_window: usize,
    pub supports_vision: bool,
    pub supports_functions: bool,
    pub tags: Vec<String>,
}

impl From<&ModelConfig> for ModelDto {
    fn from(m: &ModelConfig) -> Self {
        Self {
            id: m.id.clone(),
            aliases: m.aliases.clone(),
            context_window: m.context_window,
            supports_vision: m.supports_vision,
            supports_functions: m.supports_functions,
            tags: m.tags.clone(),
        }
    }
}

#[derive(Serialize, Clone)]
pub struct BackendDto {
    pub name: String,
    pub endpoint: String,
    pub protocol: String,
    pub backend_type: String,
    pub model_count: usize,
    pub models: Vec<ModelDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

impl From<&BackendConfig> for BackendDto {
    fn from(b: &BackendConfig) -> Self {
        let models: Vec<ModelDto> = b.models.iter().map(ModelDto::from).collect();
        Self {
            name: b.name.clone(),
            endpoint: b.endpoint.clone().unwrap_or_default(),
            protocol: b.protocol.to_string(),
            backend_type: b.backend_type.to_string(),
            model_count: models.len(),
            models,
            status: None,
        }
    }
}

#[derive(Serialize)]
struct BackendsResponse {
    backends: Vec<BackendDto>,
}

/// Load all backends from the typed `BackendStore`. Returns empty if the
/// config file does not exist; surfaces parse errors as `AppError::internal`.
pub fn load_backends() -> ApiResult<Vec<BackendConfig>> {
    let store = BackendStore::open().map_err(|e| AppError::internal(e.to_string()))?;
    if !store.path().exists() {
        return Ok(Vec::new());
    }
    store
        .list()
        .map_err(|e| AppError::internal(format!("failed to read backend config: {e}")))
}

/// Probe a backend endpoint for TCP reachability (2s timeout).
pub async fn probe_backend_status(endpoint: &str) -> String {
    let addr = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .unwrap_or(endpoint);
    let host_port = addr.split('/').next().unwrap_or(addr);

    let timeout = std::time::Duration::from_secs(2);
    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(host_port)).await {
        Ok(Ok(_)) => "healthy".to_string(),
        _ => "unreachable".to_string(),
    }
}

/// GET /api/backends
pub async fn backends_handler(Query(params): Query<BackendsQuery>) -> ApiResult<impl IntoResponse> {
    let backends = load_backends()?;

    let mut dtos: Vec<BackendDto> = backends.iter().map(BackendDto::from).collect();

    if params.probe.unwrap_or(false) {
        let futs = dtos.iter().map(|b| {
            let ep = b.endpoint.clone();
            async move {
                if ep.is_empty() {
                    "unknown".to_string()
                } else {
                    probe_backend_status(&ep).await
                }
            }
        });
        let statuses = futures::future::join_all(futs).await;
        for (dto, status) in dtos.iter_mut().zip(statuses) {
            dto.status = Some(status);
        }
    } else {
        for dto in dtos.iter_mut() {
            dto.status = Some("unknown".to_string());
        }
    }

    Ok(Json(BackendsResponse { backends: dtos }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_backends::llm::{BackendType, ProviderProtocol};
    use std::collections::HashMap;

    fn sample_backend() -> BackendConfig {
        BackendConfig {
            name: "demo".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: Some("https://api.openai.com/v1".to_string()),
            api_key: Some("sk-test".to_string()),
            headers: HashMap::new(),
            models: vec![ModelConfig {
                id: "gpt-4".to_string(),
                aliases: vec!["gpt4".to_string()],
                context_window: 8192,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                supports_vision: false,
                supports_functions: true,
                supports_thinking: false,
                supports_custom_temperature: None,
                supports_structured_outputs: None,
                max_output_tokens: None,
                tags: vec!["prod".to_string()],
            }],
            docker: None,
            auto_tool_choice: None,
            supports_structured_outputs: None,
        }
    }

    #[test]
    fn backend_dto_from_backend_config() {
        let bc = sample_backend();
        let dto = BackendDto::from(&bc);
        assert_eq!(dto.name, "demo");
        assert_eq!(dto.endpoint, "https://api.openai.com/v1");
        assert_eq!(dto.protocol, "openai");
        assert_eq!(dto.backend_type, "cloud");
        assert_eq!(dto.model_count, 1);
        assert_eq!(dto.models[0].id, "gpt-4");
        assert_eq!(dto.models[0].aliases, vec!["gpt4".to_string()]);
        assert_eq!(dto.models[0].context_window, 8192);
        assert!(dto.models[0].supports_functions);
    }

    #[test]
    fn backend_dto_handles_no_endpoint() {
        let mut bc = sample_backend();
        bc.endpoint = None;
        let dto = BackendDto::from(&bc);
        assert_eq!(dto.endpoint, "");
    }

    #[test]
    fn model_dto_defaults_when_fields_missing() {
        let model = ModelConfig {
            id: "x".to_string(),
            aliases: vec![],
            context_window: 0,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            supports_vision: false,
            supports_functions: false,
            supports_thinking: false,
            supports_custom_temperature: None,
            supports_structured_outputs: None,
            max_output_tokens: None,
            tags: vec![],
        };
        let dto = ModelDto::from(&model);
        assert_eq!(dto.id, "x");
        assert!(dto.aliases.is_empty());
        assert!(dto.tags.is_empty());
        assert_eq!(dto.context_window, 0);
    }
}
