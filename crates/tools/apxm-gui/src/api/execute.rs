//! Execute, validate, decompile, explain endpoints.

use std::collections::HashMap;

use axum::extract::Query;
use axum::response::{IntoResponse, Json};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::{ApiResult, AppError};
use crate::paths::{sessions_dir, validate_path};
use crate::process::{apxm_command, find_apxm_cli};

fn default_opt_level() -> u8 {
    1
}

#[derive(Deserialize)]
pub struct ExecuteRequest {
    pub path: String,
    #[serde(default)]
    pub params: HashMap<String, String>,
    #[serde(default = "default_opt_level")]
    pub opt_level: u8,
}

#[derive(Serialize)]
pub struct ExecuteResponse {
    pub session_path: String,
    pub execution_id: String,
}

#[derive(Deserialize)]
pub struct PathParam {
    pub path: String,
}

/// POST /api/execute
pub async fn execute_handler(Json(req): Json<ExecuteRequest>) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&req.path)?;
    let cli_bin = find_apxm_cli();

    let mut cmd = apxm_command(&cli_bin);
    cmd.arg("execute")
        .arg(&path)
        .arg("--emit-session")
        .arg("--opt-level")
        .arg(req.opt_level.to_string());

    for (key, value) in &req.params {
        cmd.arg("--param");
        cmd.arg(format!("{key}={value}"));
    }

    cmd.current_dir(std::env::current_dir().unwrap_or_default());

    let child = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| AppError::internal(format!("failed to spawn execute: {e}")))?;

    let pid = child.id().unwrap_or(0);

    let sessions = sessions_dir();

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("workflow");

    let mut session_path = String::new();
    let mut execution_id = String::new();
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Ok(mut entries) = tokio::fs::read_dir(&sessions).await {
            let mut newest: Option<(String, std::time::SystemTime)> = None;
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(stem) {
                    if let Ok(meta) = entry.metadata().await {
                        if let Ok(modified) = meta.modified() {
                            if newest.as_ref().map_or(true, |(_, t)| modified > *t) {
                                newest = Some((name.clone(), modified));
                            }
                        }
                    }
                }
            }
            if let Some((name, modified)) = newest {
                if modified.elapsed().map_or(false, |d| d.as_secs() < 10) {
                    let dir = sessions.join(&name);
                    if dir.join("manifest.json").exists() {
                        session_path = dir.to_string_lossy().to_string();
                        execution_id = name;
                        break;
                    }
                }
            }
        }
    }

    if session_path.is_empty() {
        return Err(AppError::internal(format!(
            "execution started (pid {pid}) but no session directory found - is --emit-session supported for this workflow?"
        )));
    }

    info!("Execution started: session={execution_id} pid={pid}");

    Ok(Json(ExecuteResponse {
        session_path,
        execution_id,
    }))
}

/// POST /api/validate
pub async fn validate_handler(Json(req): Json<serde_json::Value>) -> ApiResult<impl IntoResponse> {
    let path_str = req
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::bad_request("missing 'path' field"))?;
    let path = validate_path(path_str)?;
    let cli_bin = find_apxm_cli();

    let output = apxm_command(&cli_bin)
        .arg("validate")
        .arg(&path)
        .current_dir(std::env::current_dir().unwrap_or_default())
        .output()
        .await
        .map_err(|e| AppError::internal(format!("failed to spawn validate: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let details = serde_json::from_str::<serde_json::Value>(&stdout).ok();

    Ok(Json(serde_json::json!({
        "valid": output.status.success(),
        "stdout": stdout,
        "stderr": stderr,
        "details": details,
    })))
}

/// POST /api/decompile
pub async fn decompile_handler(Json(req): Json<serde_json::Value>) -> ApiResult<impl IntoResponse> {
    let path_str = req
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::bad_request("missing 'path' field"))?;
    let path = validate_path(path_str)?;
    let cli_bin = find_apxm_cli();

    let output = apxm_command(&cli_bin)
        .arg("decompile")
        .arg(&path)
        .current_dir(std::env::current_dir().unwrap_or_default())
        .output()
        .await
        .map_err(|e| AppError::internal(format!("failed to spawn decompile: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let graph = serde_json::from_str::<serde_json::Value>(&stdout).ok();

    Ok(Json(serde_json::json!({
        "success": output.status.success(),
        "graph": graph,
        "stderr": stderr,
    })))
}

/// GET /api/explain?path=<file>
pub async fn explain_handler(Query(params): Query<PathParam>) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&params.path)?;
    let cli_bin = find_apxm_cli();

    let output = apxm_command(&cli_bin)
        .arg("explain")
        .arg(&path)
        .current_dir(std::env::current_dir().unwrap_or_default())
        .output()
        .await
        .map_err(|e| AppError::internal(format!("failed to spawn explain: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok(Json(serde_json::json!({
        "success": output.status.success(),
        "explanation": stdout,
        "stderr": stderr,
    })))
}
