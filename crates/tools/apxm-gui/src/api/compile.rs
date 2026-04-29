//! Compile endpoint.

use axum::response::{IntoResponse, Json};
use serde::{Deserialize, Serialize};

use apxm_core::types::compiler::find_pass_metadata;
use apxm_core::types::{OptimizationLevel, OptimizationTarget};

use crate::air_parse;
use crate::error::{ApiResult, AppError};
use crate::paths::{SourceKind, validate_path};
use crate::process::{apxm_command, find_apxm_cli};

fn default_opt_level() -> u8 {
    1
}

#[derive(Deserialize)]
pub struct CompileRequest {
    pub path: String,
    #[serde(default = "default_opt_level")]
    pub opt_level: u8,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub no_cse_llm: bool,
}

#[derive(Serialize, Clone)]
pub struct PassMetricEntry {
    pub pass_name: String,
    pub category: String,
    pub summary: String,
    pub description: String,
    pub duration_ms: Option<f64>,
    pub ops_before: Option<usize>,
    pub ops_after: Option<usize>,
    pub ops_delta: Option<isize>,
}

#[derive(Serialize, Clone)]
pub struct PassSummary {
    pub total_passes: usize,
    pub initial_ops: usize,
    pub final_ops: usize,
    pub total_ops_eliminated: usize,
    pub active_passes: Vec<String>,
    pub total_duration_ms: f64,
}

#[derive(Serialize)]
pub struct CompileResponse {
    pub success: bool,
    pub artifact_path: Option<String>,
    pub passes: Vec<String>,
    pub pass_metrics: Vec<PassMetricEntry>,
    pub pass_summary: Option<PassSummary>,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub node_count_before: Option<usize>,
    pub node_count_after: Option<usize>,
    pub diagnostics: Option<serde_json::Value>,
}

pub fn parse_node_count(content: &str) -> Option<usize> {
    let trimmed = content.trim_start();
    if trimmed.starts_with("module") || trimmed.starts_with("func.func") {
        air_parse::parse_air_text(content)
            .ok()
            .and_then(|v| v.get("nodes").and_then(|n| n.as_array()).map(|a| a.len()))
    } else {
        serde_json::from_str::<serde_json::Value>(content)
            .ok()
            .and_then(|v| v.get("nodes").and_then(|n| n.as_array()).map(|a| a.len()))
    }
}

pub fn parse_compile_opt_level(level: u8) -> OptimizationLevel {
    match level {
        0 => OptimizationLevel::O0,
        1 => OptimizationLevel::O1,
        2 => OptimizationLevel::O2,
        _ => OptimizationLevel::O3,
    }
}

/// POST /api/compile
pub async fn compile_handler(Json(req): Json<CompileRequest>) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&req.path)?;

    let node_count_before = if let Ok(content) = tokio::fs::read_to_string(&path).await {
        parse_node_count(&content)
    } else {
        None
    };

    let cli_bin = find_apxm_cli();

    let opt_level = parse_compile_opt_level(req.opt_level);
    let opt_target: OptimizationTarget = req
        .target
        .as_deref()
        .and_then(|t| t.parse().ok())
        .unwrap_or(OptimizationTarget::Balanced);
    let pass_names = apxm_compiler::passes::build_pass_list(opt_level, req.no_cse_llm, opt_target);

    let mut pass_metrics: Vec<PassMetricEntry> = pass_names
        .iter()
        .map(|name| {
            let spec = find_pass_metadata(name);
            PassMetricEntry {
                pass_name: name.clone(),
                category: spec
                    .map(|s| format!("{:?}", s.category))
                    .unwrap_or_default(),
                summary: spec.map(|s| s.summary.to_string()).unwrap_or_default(),
                description: spec.map(|s| s.description.to_string()).unwrap_or_default(),
                duration_ms: None,
                ops_before: None,
                ops_after: None,
                ops_delta: None,
            }
        })
        .collect();

    let diag_path = std::env::temp_dir().join(format!(
        "apxm-diag-{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    let start = std::time::Instant::now();
    let mut cmd = apxm_command(&cli_bin);
    cmd.arg("compile")
        .arg(&path)
        .arg("--opt-level")
        .arg(req.opt_level.to_string())
        .arg("--emit-diagnostics")
        .arg(&diag_path);

    if let Some(ref target) = req.target {
        cmd.arg("--target").arg(target);
    }
    if req.no_cse_llm {
        cmd.arg("--no-cse-llm");
    }

    let output = cmd
        .current_dir(std::env::current_dir().unwrap_or_default())
        .output()
        .await
        .map_err(|e| AppError::internal(format!("failed to spawn compile: {e}")))?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let artifact_path = path.with_extension("apxmobj");
    let artifact_exists = artifact_path.exists();

    let node_count_after = if artifact_exists {
        if let Ok(out) = apxm_command(&cli_bin)
            .arg("decompile")
            .arg(&artifact_path)
            .output()
            .await
        {
            let decomp = String::from_utf8_lossy(&out.stdout);
            decomp
                .lines()
                .find(|l| l.contains("\"nodes\""))
                .and_then(|_| {
                    serde_json::from_str::<serde_json::Value>(&decomp)
                        .ok()
                        .and_then(|v| v.get("nodes").and_then(|n| n.as_array()).map(|a| a.len()))
                })
        } else {
            None
        }
    } else {
        None
    };

    let mut pass_summary: Option<PassSummary> = None;
    let diagnostics = if let Ok(diag_content) = tokio::fs::read_to_string(&diag_path).await {
        let _ = tokio::fs::remove_file(&diag_path).await;
        if let Ok(diag_val) = serde_json::from_str::<serde_json::Value>(&diag_content) {
            if let Some(metrics_arr) = diag_val.get("pass_metrics").and_then(|v| v.as_array()) {
                for m in metrics_arr {
                    let pname = m.get("pass_name").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(entry) = pass_metrics.iter_mut().find(|e| e.pass_name == pname) {
                        entry.duration_ms = m.get("duration_ms").and_then(|v| v.as_f64());
                        entry.ops_before = m
                            .get("ops_before")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize);
                        entry.ops_after = m
                            .get("ops_after")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize);
                        entry.ops_delta = m
                            .get("ops_delta")
                            .and_then(|v| v.as_i64())
                            .map(|v| v as isize);
                    }
                }
            }
            if let Some(summary) = diag_val.get("pass_summary") {
                pass_summary = Some(PassSummary {
                    total_passes: summary
                        .get("total_passes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize,
                    initial_ops: summary
                        .get("initial_ops")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize,
                    final_ops: summary
                        .get("final_ops")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize,
                    total_ops_eliminated: summary
                        .get("total_ops_eliminated")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize,
                    active_passes: summary
                        .get("active_passes")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    total_duration_ms: summary
                        .get("total_duration_ms")
                        .and_then(|v| v.as_f64())
                        .or_else(|| {
                            diag_val
                                .get("compilation_phases")
                                .and_then(|p| p.get("passes_ms"))
                                .and_then(|v| v.as_f64())
                        })
                        .unwrap_or(0.0),
                });
            }
            Some(diag_val)
        } else {
            None
        }
    } else {
        let _ = tokio::fs::remove_file(&diag_path).await;
        None
    };

    Ok(Json(CompileResponse {
        success: output.status.success(),
        artifact_path: if artifact_exists {
            Some(artifact_path.to_string_lossy().to_string())
        } else {
            None
        },
        passes: pass_names,
        pass_metrics,
        pass_summary,
        duration_ms,
        stdout,
        stderr,
        node_count_before,
        node_count_after,
        diagnostics,
    }))
}

// Touch SourceKind import to avoid unused warning when this module references it indirectly.
#[allow(dead_code)]
const _SOURCE_KIND_HINT: &[&str] = &[SourceKind::Air.as_str(), SourceKind::Py.as_str()];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opt_level_maps_to_enum() {
        assert!(matches!(parse_compile_opt_level(0), OptimizationLevel::O0));
        assert!(matches!(parse_compile_opt_level(1), OptimizationLevel::O1));
        assert!(matches!(parse_compile_opt_level(2), OptimizationLevel::O2));
        assert!(matches!(parse_compile_opt_level(3), OptimizationLevel::O3));
        assert!(matches!(parse_compile_opt_level(99), OptimizationLevel::O3));
    }

    #[test]
    fn parse_node_count_returns_none_on_garbage() {
        assert_eq!(parse_node_count("not json"), None);
    }

    #[test]
    fn parse_node_count_from_simple_json() {
        let json = r#"{"nodes": [{"id":1},{"id":2},{"id":3}], "edges":[]}"#;
        assert_eq!(parse_node_count(json), Some(3));
    }
}
