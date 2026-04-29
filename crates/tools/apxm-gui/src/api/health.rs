//! `/api/health` endpoint.

use axum::extract::Query;
use axum::response::{IntoResponse, Json};
use serde::Deserialize;

use crate::api::backend::{load_backends, probe_backend_status};
use crate::env::apxm_home;
use crate::error::ApiResult;

#[derive(Deserialize)]
pub struct HealthQuery {
    #[serde(default)]
    pub probe: Option<bool>,
}

/// GET /api/health
pub async fn health_handler(Query(params): Query<HealthQuery>) -> ApiResult<impl IntoResponse> {
    let do_probe = params.probe.unwrap_or(false);

    let backends = load_backends().unwrap_or_default();

    let infos: Vec<(String, String, String, usize)> = backends
        .iter()
        .map(|b| {
            (
                b.name.clone(),
                b.endpoint.clone().unwrap_or_default(),
                b.protocol.to_string(),
                b.models.len(),
            )
        })
        .collect();

    let statuses: Vec<String> = if do_probe {
        let futs = infos.iter().map(|(_, ep, _, _)| {
            let ep = ep.clone();
            async move {
                if ep.is_empty() {
                    "unknown".to_string()
                } else {
                    probe_backend_status(&ep).await
                }
            }
        });
        futures::future::join_all(futs).await
    } else {
        infos.iter().map(|_| "unknown".to_string()).collect()
    };

    let mut total_models: usize = 0;
    let mut backends_json: Vec<serde_json::Value> = Vec::new();
    for ((name, endpoint, protocol, model_count), status) in infos.iter().zip(statuses) {
        total_models += model_count;
        backends_json.push(serde_json::json!({
            "name": name,
            "endpoint": endpoint,
            "protocol": protocol,
            "model_count": model_count,
            "status": status,
        }));
    }

    let apxm_dir = apxm_home();
    let agents_path = apxm_dir.join("agents.toml");
    let tools_path = apxm_dir.join("tools.json");

    let (total_agents, total_tools) = tokio::join!(
        async {
            if !agents_path.exists() {
                return 0;
            }
            tokio::fs::read_to_string(&agents_path)
                .await
                .ok()
                .and_then(|content| content.parse::<toml::Value>().ok())
                .and_then(|v| v.get("agents").and_then(|a| a.as_array()).map(|a| a.len()))
                .unwrap_or(0)
        },
        async {
            if !tools_path.exists() {
                return 0;
            }
            tokio::fs::read_to_string(&tools_path)
                .await
                .ok()
                .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
                .and_then(|v| v.get("tools").and_then(|t| t.as_array()).map(|a| a.len()))
                .unwrap_or(0)
        }
    );

    let config_source = if std::env::current_dir()
        .map(|cwd| cwd.join(".apxm").join("config.toml").exists())
        .unwrap_or(false)
    {
        "project"
    } else {
        "global"
    };

    Ok(Json(serde_json::json!({
        "backends": backends_json,
        "total_models": total_models,
        "total_agents": total_agents,
        "total_tools": total_tools,
        "config_source": config_source,
    })))
}
