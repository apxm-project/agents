//! File-system endpoints: workflows, examples, file content, file tree.

use std::path::Path;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json};
use serde::{Deserialize, Serialize};

use crate::air_parse;
use crate::error::{ApiResult, AppError};
use crate::paths::{SourceKind, is_skip_dir, validate_path};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct PathParam {
    pub path: String,
}

#[derive(Serialize)]
struct StartupResponse {
    initial_file: Option<String>,
}

/// GET /api/file?path=<file>
pub async fn file_handler(Query(params): Query<PathParam>) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&params.path)?;
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| AppError::not_found(format!("failed to read file: {e}")))?;
    Ok(content)
}

/// GET /api/startup
pub async fn startup_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(StartupResponse {
        initial_file: state.initial_file.clone(),
    })
}

/// GET /api/examples
pub async fn examples_handler(State(state): State<Arc<AppState>>) -> ApiResult<impl IntoResponse> {
    let examples_dir = match &state.examples_dir {
        Some(d) if d.is_dir() => d.clone(),
        _ => {
            return Ok(Json(serde_json::json!({ "examples": [] })));
        }
    };

    let mut examples: Vec<serde_json::Value> = Vec::new();
    collect_graphs(&examples_dir, &examples_dir, 0, &mut examples).await;

    examples.sort_by(|a, b| {
        let pa = a
            .get("relative_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let pb = b
            .get("relative_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        pa.cmp(pb)
    });

    Ok(Json(serde_json::json!({ "examples": examples })))
}

/// GET /api/workflows
pub async fn workflows_handler() -> ApiResult<impl IntoResponse> {
    let cwd = std::env::current_dir()
        .map_err(|e| AppError::internal(format!("failed to get current directory: {e}")))?;

    let mut raw: Vec<serde_json::Value> = Vec::new();
    collect_graphs(&cwd, &cwd, 0, &mut raw).await;

    let mut by_dir_stem: std::collections::HashMap<(String, String), Vec<serde_json::Value>> =
        std::collections::HashMap::new();
    for entry in raw {
        let rel = entry
            .get("relative_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let p = std::path::Path::new(rel);
        let dir = p
            .parent()
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_default();
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        by_dir_stem.entry((dir, stem)).or_default().push(entry);
    }

    let mut workflows: Vec<serde_json::Value> = Vec::new();
    for (_, mut group) in by_dir_stem {
        if group.len() == 1 {
            workflows.push(group.remove(0));
        } else {
            let py_idx = group
                .iter()
                .position(|e| e.get("source_type").and_then(|v| v.as_str()) == Some("py"));
            if let Some(idx) = py_idx {
                let mut primary = group.remove(idx);
                for companion in &group {
                    if primary.get("node_count").and_then(|v| v.as_u64()).is_none() {
                        if let Some(count) = companion.get("node_count") {
                            primary["node_count"] = count.clone();
                        }
                    }
                    if primary.get("parameters").is_none() {
                        if let Some(params) = companion.get("parameters") {
                            primary["parameters"] = params.clone();
                        }
                    }
                    primary["air_path"] = companion
                        .get("path")
                        .cloned()
                        .unwrap_or(serde_json::json!(null));
                }
                workflows.push(primary);
            } else {
                workflows.extend(group);
            }
        }
    }

    workflows.sort_by(|a, b| {
        let pa = a
            .get("relative_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let pb = b
            .get("relative_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        pa.cmp(pb)
    });

    Ok(Json(serde_json::json!({ "workflows": workflows })))
}

/// GET /api/filetree
pub async fn filetree_handler() -> ApiResult<impl IntoResponse> {
    let cwd = std::env::current_dir()
        .map_err(|e| AppError::internal(format!("failed to get current directory: {e}")))?;

    let tree = build_tree(&cwd, &cwd, 0).await;
    let cwd_name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());

    Ok(Json(serde_json::json!({
        "root": cwd_name,
        "cwd": cwd.to_string_lossy(),
        "tree": tree,
    })))
}

async fn build_tree(dir: &Path, base: &Path, depth: usize) -> Vec<serde_json::Value> {
    if depth > 4 {
        return vec![];
    }
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return vec![],
    };

    let mut items: Vec<serde_json::Value> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if is_skip_dir(&name) {
            continue;
        }

        let rel = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        if path.is_dir() {
            let children = Box::pin(build_tree(&path, base, depth + 1)).await;
            if !children.is_empty() {
                items.push(serde_json::json!({
                    "name": name,
                    "path": rel,
                    "is_dir": true,
                    "children": children,
                }));
            }
        } else {
            let kind = SourceKind::from_path(&path);
            if matches!(
                kind,
                Some(SourceKind::Air) | Some(SourceKind::Py) | Some(SourceKind::Toml)
            ) {
                let mut entry_json = serde_json::json!({
                    "name": name,
                    "path": rel,
                    "is_dir": false,
                });

                if matches!(kind, Some(SourceKind::Air)) {
                    if let Ok(content) = tokio::fs::read_to_string(&path).await {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                            let graph_name = parsed.get("name").and_then(|n| n.as_str());
                            let node_count = parsed
                                .get("nodes")
                                .and_then(|n| n.as_array())
                                .map(|a| a.len());
                            entry_json["graph_meta"] = serde_json::json!({
                                "name": graph_name,
                                "node_count": node_count,
                            });
                        }
                    }
                }

                items.push(entry_json);
            }
        }
    }

    items.sort_by(|a, b| {
        let a_dir = a.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);
        let b_dir = b.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);
        b_dir.cmp(&a_dir).then_with(|| {
            let an = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let bn = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
            an.cmp(bn)
        })
    });

    items
}

/// Extract workflow metadata from a `.py` file by scanning for `@compile`
/// and a docstring.
pub fn extract_py_metadata(content: &str) -> (Option<String>, Option<String>) {
    let mut func_name = None;
    let mut description = None;
    let mut found_compile = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("@compile") {
            found_compile = true;
            continue;
        }
        if found_compile && trimmed.starts_with("def ") {
            let after_def = &trimmed[4..];
            func_name = after_def.split('(').next().map(|s| s.trim().to_string());
            found_compile = false;
            continue;
        }
        if found_compile {
            found_compile = false;
        }
        if func_name.is_some() && description.is_none() {
            if trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''") {
                let doc = trimmed
                    .trim_start_matches("\"\"\"")
                    .trim_start_matches("'''");
                let doc = doc
                    .trim_end_matches("\"\"\"")
                    .trim_end_matches("'''")
                    .trim();
                if !doc.is_empty() {
                    description = Some(doc.to_string());
                }
            }
            break;
        }
    }
    (func_name, description)
}

/// Recursively discover workflow files (.air, .py) under `dir`.
pub async fn collect_graphs(
    base: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<serde_json::Value>,
) {
    if depth > 8 {
        return;
    }
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let name_str = entry.file_name().to_string_lossy().to_string();

        if is_skip_dir(&name_str) {
            continue;
        }

        if path.is_dir() {
            Box::pin(collect_graphs(base, &path, depth + 1, out)).await;
            continue;
        }

        let kind = SourceKind::from_path(&path);
        let (source_type, is_py) = match kind {
            Some(SourceKind::Air) => (SourceKind::Air.as_str(), false),
            Some(SourceKind::Py) => (SourceKind::Py.as_str(), true),
            _ => continue,
        };

        if is_py && (name_str == "__init__.py" || name_str.starts_with("test_")) {
            continue;
        }

        let abs = tokio::fs::canonicalize(&path)
            .await
            .unwrap_or_else(|_| path.clone())
            .to_string_lossy()
            .to_string();
        let rel = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let category = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let content = tokio::fs::read_to_string(&path).await.ok();

        let (graph_name, node_count, description, parameters) = if is_py {
            let (func_name, desc) = content
                .as_deref()
                .map(extract_py_metadata)
                .unwrap_or((None, None));
            if func_name.is_none() {
                if content.as_deref().map_or(true, |c| {
                    !c.contains("@compile") && !c.contains("GraphRecorder")
                }) {
                    continue;
                }
            }
            (func_name, None, desc, None::<Vec<serde_json::Value>>)
        } else {
            // .air
            if let Some(ref text) = content {
                let trimmed = text.trim_start();
                if trimmed.starts_with("module") || trimmed.starts_with("func.func") {
                    match air_parse::parse_air_text(text) {
                        Ok(parsed) => {
                            let name = parsed
                                .get("name")
                                .and_then(|n| n.as_str())
                                .map(String::from);
                            let count = parsed
                                .get("nodes")
                                .and_then(|n| n.as_array())
                                .map(|a| a.len());
                            let params =
                                parsed.get("parameters").and_then(|p| p.as_array()).cloned();
                            (name, count, None, params)
                        }
                        Err(_) => (None, None, None, None),
                    }
                } else {
                    let parsed = serde_json::from_str::<serde_json::Value>(text).ok();
                    let name = parsed
                        .as_ref()
                        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from));
                    let count = parsed
                        .as_ref()
                        .and_then(|v| v.get("nodes").and_then(|n| n.as_array()).map(|a| a.len()));
                    let params = parsed
                        .as_ref()
                        .and_then(|v| v.get("parameters").and_then(|p| p.as_array()).cloned());
                    (name, count, None, params)
                }
            } else {
                (None, None, None, None)
            }
        };

        let mut entry = serde_json::json!({
            "path": abs,
            "relative_path": rel,
            "name": graph_name.unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| rel.clone())
            }),
            "category": category,
            "node_count": node_count,
            "source_type": source_type,
        });
        if let Some(desc) = description {
            entry["description"] = serde_json::json!(desc);
        }
        if let Some(params) = parameters {
            entry["parameters"] = serde_json::json!(params);
        }
        out.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_py_metadata_picks_up_compile_and_docstring() {
        let src = r#"
from apxm import compile

@compile
def my_workflow(x):
    """This is a workflow."""
    return x
"#;
        let (name, desc) = extract_py_metadata(src);
        assert_eq!(name.as_deref(), Some("my_workflow"));
        assert_eq!(desc.as_deref(), Some("This is a workflow."));
    }

    #[test]
    fn extract_py_metadata_no_compile() {
        let src = "def helper():\n    return 1\n";
        let (name, desc) = extract_py_metadata(src);
        assert!(name.is_none());
        assert!(desc.is_none());
    }
}
