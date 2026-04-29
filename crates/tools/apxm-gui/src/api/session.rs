//! Session inspection endpoints.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use axum::extract::{Path as AxumPath, Query};
use axum::response::{IntoResponse, Json};
use serde::Deserialize;

use crate::api::files::collect_graphs;
use crate::error::{ApiResult, AppError};
use crate::paths::validate_path;

#[derive(Deserialize)]
pub struct PathParam {
    pub path: String,
}

#[derive(Deserialize)]
pub struct SessionNodeParam {
    pub session: String,
}

/// Read a JSON file and return it as a serde_json::Value.
pub async fn read_json_file(path: &Path) -> Result<serde_json::Value, AppError> {
    if !path.exists() {
        return Ok(serde_json::Value::Null);
    }
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| AppError::internal(format!("failed to read {}: {e}", path.display())))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| AppError::bad_request(format!("invalid JSON in {}: {e}", path.display())))?;
    Ok(value)
}

/// Read an NDJSON file (one JSON object per line) and return as a JSON array.
pub async fn read_ndjson_file(path: &Path) -> Result<serde_json::Value, AppError> {
    if !path.exists() {
        return Ok(serde_json::Value::Array(vec![]));
    }
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| AppError::internal(format!("failed to read {}: {e}", path.display())))?;

    let events: Vec<serde_json::Value> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    Ok(serde_json::Value::Array(events))
}

/// GET /api/session?path=<dir>
pub async fn session_handler(Query(params): Query<PathParam>) -> ApiResult<impl IntoResponse> {
    let dir = validate_path(&params.path)?;
    if !dir.is_dir() {
        return Err(AppError::not_found(format!(
            "session directory not found: {}",
            params.path
        )));
    }

    let manifest = read_json_file(&dir.join("manifest.json")).await?;
    let trace = read_ndjson_file(&dir.join("trace.ndjson")).await?;
    let results = read_json_file(&dir.join("results.json")).await?;
    let metrics = read_json_file(&dir.join("metrics.json")).await?;
    let node_statuses = read_json_file(&dir.join("node_statuses.json")).await?;

    let mut node_names: HashMap<String, String> = HashMap::new();
    let nodes_dir = dir.join("nodes");
    if nodes_dir.is_dir() {
        if let Ok(mut entries) = tokio::fs::read_dir(&nodes_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(idx) = name.find('_') {
                    let id_str = name[..idx].trim_start_matches('0');
                    let id = if id_str.is_empty() { "0" } else { id_str };
                    let display = &name[idx + 1..];
                    node_names.insert(id.to_string(), display.to_string());
                }
            }
        }
    }

    let graph_name = manifest
        .get("graph_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut source_path: Option<String> = None;
    if !graph_name.is_empty() {
        let cwd = std::env::current_dir().unwrap_or_default();
        for ext in &["py", "air"] {
            let candidate = cwd
                .join("examples")
                .join("python")
                .join(format!("{graph_name}.{ext}"));
            if candidate.exists() {
                source_path = Some(candidate.to_string_lossy().to_string());
                break;
            }
            let candidate2 = cwd.join(format!("{graph_name}.{ext}"));
            if candidate2.exists() {
                source_path = Some(candidate2.to_string_lossy().to_string());
                break;
            }
        }
        if source_path.is_none() {
            let mut scan: Vec<serde_json::Value> = Vec::new();
            collect_graphs(&cwd, &cwd, 0, &mut scan).await;
            for entry in &scan {
                let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if name == graph_name {
                    source_path = entry.get("path").and_then(|v| v.as_str()).map(String::from);
                    break;
                }
            }
        }
    }

    Ok(Json(serde_json::json!({
        "manifest": manifest,
        "trace": trace,
        "results": results,
        "metrics": metrics,
        "node_statuses": node_statuses,
        "node_names": node_names,
        "source_path": source_path,
    })))
}

/// GET /api/session/node/{id}?session=<dir>
pub async fn session_node_handler(
    AxumPath(node_id): AxumPath<String>,
    Query(params): Query<SessionNodeParam>,
) -> ApiResult<impl IntoResponse> {
    let session_dir = validate_path(&params.session)?;
    if !session_dir.is_dir() {
        return Err(AppError::not_found(format!(
            "session directory not found: {}",
            params.session
        )));
    }

    let nodes_dir = session_dir.join("nodes");
    if !nodes_dir.is_dir() {
        return Err(AppError::not_found(
            "nodes directory not found in session".to_string(),
        ));
    }

    let mut target_dir: Option<PathBuf> = None;
    if let Ok(mut entries) = tokio::fs::read_dir(&nodes_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&format!("{}_", node_id))
                || name_str.starts_with(&format!("{:02}_", node_id.parse::<u64>().unwrap_or(0)))
            {
                target_dir = Some(entry.path());
                break;
            }
        }
    }

    let target_dir = target_dir.ok_or_else(|| {
        AppError::not_found(format!("node directory not found for id: {node_id}"))
    })?;

    let mut result = serde_json::Map::new();
    let files = &["node.json", "output.json", "status.json", "live.json"];

    for filename in files {
        let file_path = target_dir.join(filename);
        if file_path.exists() {
            if let Ok(value) = read_json_file(&file_path).await {
                let key = filename.trim_end_matches(".json").to_string();
                result.insert(key, value);
            }
        }
    }

    let trace_path = target_dir.join("trace.ndjson");
    if trace_path.exists() {
        if let Ok(trace) = read_ndjson_file(&trace_path).await {
            result.insert("trace".to_string(), trace);
        }
    }

    let text_files = &["prompt.txt", "response.txt"];
    for filename in text_files {
        let file_path = target_dir.join(filename);
        if file_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&file_path).await {
                let key = filename.trim_end_matches(".txt").to_string();
                result.insert(key, serde_json::Value::String(content));
            }
        }
    }

    result.insert(
        "directory".to_string(),
        serde_json::Value::String(target_dir.to_string_lossy().to_string()),
    );

    Ok(Json(serde_json::Value::Object(result)))
}
