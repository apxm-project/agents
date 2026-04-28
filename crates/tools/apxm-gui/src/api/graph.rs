//! Graph endpoints: read, analyze, save, ops, passes.

use std::path::Path;

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use serde::Deserialize;
use tracing::{error, info};

use apxm_compiler::AirModule;
use apxm_core::types::compiler::list_pass_metadata;
use apxm_core::types::operations::metadata::get_all_operations;
use apxm_core::types::OperationSpec;

use crate::air_parse;
use crate::analysis::{analyze_graph, normalize_ops_for_air_module};
use crate::error::{ApiResult, AppError};
use crate::paths::{validate_path, SourceKind};

#[derive(Deserialize)]
pub struct PathParam {
    pub path: String,
}

/// Read a graph source file - handles `.air` (MLIR text or JSON) and `.py`
/// (compile to AIR by running the Python file).
pub async fn read_graph_content(path: &Path) -> Result<String, AppError> {
    if matches!(SourceKind::from_path(path), Some(SourceKind::Py)) {
        let repo_root = std::env::current_dir().unwrap_or_default();
        let python_frontend = repo_root.join("crates/compiler/apxm-frontend/python");
        let mut pythonpath_entries = vec![python_frontend, repo_root];
        if let Some(parent) = path.parent() {
            pythonpath_entries.push(parent.to_path_buf());
        }
        if let Some(existing) = std::env::var_os(apxm_core::constants::env::PYTHONPATH) {
            pythonpath_entries.extend(std::env::split_paths(&existing));
        }
        let pythonpath = std::env::join_paths(pythonpath_entries)
            .map_err(|e| AppError::internal(format!("PYTHONPATH error: {e}")))?;

        let output = tokio::process::Command::new("python3")
            .arg(path)
            .env(apxm_core::constants::env::PYTHONPATH, &pythonpath)
            .output()
            .await
            .map_err(|e| AppError::internal(format!("failed to run Python: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::bad_request(format!(
                "Python error: {}",
                stderr.trim()
            )));
        }

        let air = String::from_utf8(output.stdout)
            .map_err(|_| AppError::bad_request("Python output is not valid UTF-8".to_string()))?;
        if air.trim().is_empty() {
            return Err(AppError::bad_request(
                "Python file produced no AIR output".to_string(),
            ));
        }
        Ok(air)
    } else {
        tokio::fs::read_to_string(path).await.map_err(|e| {
            error!(path = %path.display(), error = %e, "failed to read graph file");
            AppError::internal(format!("failed to read file: {e}"))
        })
    }
}

/// Parse file content into the GUI graph data shape.
pub fn parse_graph_content(content: &str) -> Result<serde_json::Value, AppError> {
    let trimmed = content.trim_start();
    let is_mlir = trimmed.starts_with("module") || trimmed.starts_with("func.func");

    if is_mlir {
        air_parse::parse_air_text(content)
            .map_err(|e| AppError::bad_request(format!("failed to parse AIR: {e}")))
    } else {
        if let Ok(graph) = serde_json::from_str::<AirModule>(content) {
            return Ok(serde_json::to_value(graph).unwrap());
        }
        let val: serde_json::Value = serde_json::from_str(content)
            .map_err(|e| AppError::bad_request(format!("failed to parse graph data: {e}")))?;
        if val.get("nodes").is_some() && val.get("edges").is_some() {
            Ok(val)
        } else {
            Err(AppError::bad_request(
                "JSON missing 'nodes' or 'edges' fields".to_string(),
            ))
        }
    }
}

/// GET /api/graph?path=<file>
pub async fn graph_handler(Query(params): Query<PathParam>) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&params.path)?;
    let content = read_graph_content(&path).await?;
    let graph = parse_graph_content(&content)?;
    Ok(Json(graph))
}

/// GET /api/graph/analyze?path=<file>
pub async fn graph_analyze_handler(
    Query(params): Query<PathParam>,
) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&params.path)?;
    let content = read_graph_content(&path).await?;
    let graph_json = parse_graph_content(&content)?;

    let normalized = normalize_ops_for_air_module(graph_json);
    let graph: AirModule = serde_json::from_value(normalized)
        .map_err(|e| AppError::bad_request(format!("failed to map graph: {e}")))?;

    let analysis = analyze_graph(&graph);
    Ok(Json(analysis))
}

/// GET /api/ops
pub async fn ops_handler() -> impl IntoResponse {
    let ops: Vec<&OperationSpec> = get_all_operations().collect();
    Json(ops)
}

/// GET /api/passes
pub async fn passes_handler() -> impl IntoResponse {
    let passes: Vec<serde_json::Value> = list_pass_metadata()
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "category": format!("{:?}", p.category),
                "summary": p.summary,
                "description": p.description,
            })
        })
        .collect();
    Json(passes)
}

/// POST /api/graph/save
pub async fn save_graph_handler(
    Json(payload): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let graph = payload
        .get("graph")
        .ok_or_else(|| AppError::bad_request("missing 'graph' field"))?;

    let graph_name = graph
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("untitled");

    let save_path = if let Some(p) = payload.get("path").and_then(|p| p.as_str()) {
        std::path::PathBuf::from(p)
    } else {
        let dir = std::path::PathBuf::from("workflows");
        if !dir.exists() {
            std::fs::create_dir_all(&dir)
                .map_err(|e| AppError::internal(format!("create dir: {e}")))?;
        }
        dir.join(format!("{}.{}", graph_name, SourceKind::Air.as_str()))
    };

    let content = serde_json::to_string_pretty(graph)
        .map_err(|e| AppError::internal(format!("serialize: {e}")))?;

    tokio::fs::write(&save_path, &content)
        .await
        .map_err(|e| AppError::internal(format!("write file: {e}")))?;

    let abs_path = save_path
        .canonicalize()
        .unwrap_or_else(|_| save_path.clone())
        .display()
        .to_string();

    info!("Graph saved: {} ({} bytes)", abs_path, content.len());

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "path": abs_path })),
    ))
}
