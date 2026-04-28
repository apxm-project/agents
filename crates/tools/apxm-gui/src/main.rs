//! APXM GUI — Axum web server for visualizing agent workflow graphs,
//! compiler optimizations, and session traces.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Json, Response};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing::{error, info};

use apxm_compiler::AirModule;
use apxm_core::types::compiler::{find_pass_metadata, list_pass_metadata};
use apxm_core::types::operations::metadata::get_all_operations;
use apxm_core::types::{OperationSpec, OptimizationLevel, OptimizationTarget};

mod acp_client;
mod air_parse;
mod api;
mod events;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

/// Shared application state.
struct AppState {
    /// Initial graph file to load on startup (from `--file` arg).
    initial_file: Option<String>,
    /// Directory to scan for example workflow files.
    examples_dir: Option<PathBuf>,
    /// Active ACP agent sessions, keyed by session ID.
    agent_sessions: DashMap<String, Arc<Mutex<acp_client::AgentSession>>>,
}

// ---------------------------------------------------------------------------
// Error helper
// ---------------------------------------------------------------------------

struct AppError(StatusCode, String);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.1 });
        (self.0, Json(body)).into_response()
    }
}

impl<E: std::fmt::Display> From<E> for AppError {
    fn from(err: E) -> Self {
        AppError(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
    }
}

type ApiResult<T> = Result<T, AppError>;

/// Validate that a user-provided path resolves to within cwd or `~/.apxm/sessions/`.
fn validate_path(path: &str) -> Result<PathBuf, AppError> {
    let home = apxm_core::env::home_dir();
    // Expand leading ~/ to $HOME/
    let expanded = if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(path)
    };
    let canonical = std::fs::canonicalize(&expanded)
        .map_err(|_| AppError(StatusCode::NOT_FOUND, format!("path not found: {path}")))?;
    let cwd = std::env::current_dir().unwrap_or_default();
    let sessions = home.join(".apxm").join("sessions");
    if canonical.starts_with(&cwd) || canonical.starts_with(&sessions) {
        Ok(canonical)
    } else {
        Err(AppError(
            StatusCode::FORBIDDEN,
            "path outside allowed directories".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Query parameter structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PathParam {
    path: String,
}

#[derive(Deserialize)]
struct HealthParam {
    #[serde(default)]
    probe: Option<bool>,
}

#[derive(Deserialize)]
struct SessionNodeParam {
    session: String,
}

// OperationSpec derives Serialize directly — no mirror types needed.

// ---------------------------------------------------------------------------
// Graph analysis types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct GraphAnalysis {
    total_nodes: usize,
    total_edges: usize,
    entry_nodes: Vec<u64>,
    exit_nodes: Vec<u64>,
    critical_path_length: usize,
    critical_path: Vec<u64>,
    max_parallelism: usize,
    op_histogram: HashMap<String, usize>,
}

fn analyze_graph(graph: &AirModule) -> GraphAnalysis {
    use std::collections::{HashSet, VecDeque};

    let node_ids: HashSet<u64> = graph.nodes.iter().map(|n| n.id).collect();

    // Build adjacency: from -> [to]
    let mut successors: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut predecessors: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut targets: HashSet<u64> = HashSet::new();
    let mut sources: HashSet<u64> = HashSet::new();

    for edge in &graph.edges {
        successors.entry(edge.from).or_default().push(edge.to);
        predecessors.entry(edge.to).or_default().push(edge.from);
        targets.insert(edge.to);
        sources.insert(edge.from);
    }

    // Entry nodes: no incoming edges
    let entry_nodes: Vec<u64> = graph
        .nodes
        .iter()
        .filter(|n| !targets.contains(&n.id))
        .map(|n| n.id)
        .collect();

    // Exit nodes: no outgoing edges
    let exit_nodes: Vec<u64> = graph
        .nodes
        .iter()
        .filter(|n| !sources.contains(&n.id))
        .map(|n| n.id)
        .collect();

    // Topological levels via BFS from entry nodes (for max parallelism).
    // Also compute longest path (critical path) via dynamic programming.
    let mut in_degree: HashMap<u64, usize> = HashMap::new();
    for &id in &node_ids {
        in_degree.insert(id, predecessors.get(&id).map_or(0, |p| p.len()));
    }

    let mut queue: VecDeque<u64> = VecDeque::new();
    let mut level: HashMap<u64, usize> = HashMap::new();
    for &id in &entry_nodes {
        queue.push_back(id);
        level.insert(id, 0);
    }

    let mut level_counts: HashMap<usize, usize> = HashMap::new();
    let mut dist: HashMap<u64, usize> = HashMap::new(); // longest path to node
    let mut prev: HashMap<u64, u64> = HashMap::new(); // predecessor on longest path

    for &id in &entry_nodes {
        dist.insert(id, 1);
    }

    // Kahn's algorithm for topological order + level assignment
    let mut topo_order: Vec<u64> = Vec::new();
    let mut in_deg = in_degree.clone();

    for &id in &entry_nodes {
        queue.push_back(id);
    }
    // Reset queue (was used for levels above, reuse)
    queue.clear();
    for &id in &entry_nodes {
        queue.push_back(id);
    }

    while let Some(u) = queue.pop_front() {
        topo_order.push(u);
        let lvl = *level.get(&u).unwrap_or(&0);
        *level_counts.entry(lvl).or_insert(0) += 1;

        if let Some(succs) = successors.get(&u) {
            for &v in succs {
                // Update longest path
                let new_dist = dist.get(&u).copied().unwrap_or(1) + 1;
                if new_dist > dist.get(&v).copied().unwrap_or(0) {
                    dist.insert(v, new_dist);
                    prev.insert(v, u);
                }

                // Update level
                let new_level = lvl + 1;
                let cur_level = level.entry(v).or_insert(0);
                if new_level > *cur_level {
                    *cur_level = new_level;
                }

                if let Some(d) = in_deg.get_mut(&v) {
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(v);
                    }
                }
            }
        }
    }

    // Recount level_counts after all levels are finalized
    level_counts.clear();
    for &lvl in level.values() {
        *level_counts.entry(lvl).or_insert(0) += 1;
    }

    let max_parallelism = level_counts.values().copied().max().unwrap_or(1);

    // Reconstruct critical path: find the node with the longest dist
    let critical_end = dist.iter().max_by_key(|&(_, d)| *d).map(|(&id, _)| id);
    let critical_path_length = critical_end
        .and_then(|id| dist.get(&id).copied())
        .unwrap_or(0);

    let mut critical_path = Vec::new();
    if let Some(mut current) = critical_end {
        critical_path.push(current);
        while let Some(&p) = prev.get(&current) {
            critical_path.push(p);
            current = p;
        }
        critical_path.reverse();
    }

    // Op type histogram
    let mut op_histogram: HashMap<String, usize> = HashMap::new();
    for node in &graph.nodes {
        *op_histogram.entry(format!("{:?}", node.op)).or_insert(0) += 1;
    }

    GraphAnalysis {
        total_nodes: graph.nodes.len(),
        total_edges: graph.edges.len(),
        entry_nodes,
        exit_nodes,
        critical_path_length,
        critical_path,
        max_parallelism,
        op_histogram,
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET / — serve the embedded SPA index.html.
async fn index_handler() -> Html<&'static str> {
    Html(include_str!("frontend-dist/index.html"))
}

/// SPA fallback — serve index.html for all non-API routes (client-side routing).
async fn spa_fallback() -> Html<&'static str> {
    Html(include_str!("frontend-dist/index.html"))
}

/// Read a graph source file — handles .air (MLIR text or JSON) and .py (compile to .air).
async fn read_graph_content(path: &std::path::Path) -> Result<String, AppError> {
    if path.extension().and_then(|e| e.to_str()) == Some("py") {
        // Run Python file to emit AIR text
        let repo_root = std::env::current_dir().unwrap_or_default();
        let python_frontend = repo_root.join("crates/compiler/apxm-frontend/python");
        let mut pythonpath_entries = vec![python_frontend, repo_root];
        if let Some(parent) = path.parent() {
            pythonpath_entries.push(parent.to_path_buf());
        }
        if let Some(existing) = std::env::var_os(apxm_core::constants::env::PYTHONPATH) {
            pythonpath_entries.extend(std::env::split_paths(&existing));
        }
        let pythonpath = std::env::join_paths(pythonpath_entries).map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("PYTHONPATH error: {e}"),
            )
        })?;

        let output = tokio::process::Command::new("python3")
            .arg(path)
            .env(apxm_core::constants::env::PYTHONPATH, &pythonpath)
            .output()
            .await
            .map_err(|e| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to run Python: {e}"),
                )
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                format!("Python error: {}", stderr.trim()),
            ));
        }

        let air = String::from_utf8(output.stdout).map_err(|_| {
            AppError(
                StatusCode::BAD_REQUEST,
                "Python output is not valid UTF-8".into(),
            )
        })?;
        if air.trim().is_empty() {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                "Python file produced no AIR output".into(),
            ));
        }
        Ok(air)
    } else {
        tokio::fs::read_to_string(path).await.map_err(|e| {
            error!(path = %path.display(), error = %e, "failed to read graph file");
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to read file: {e}"),
            )
        })
    }
}

/// Parse file content into the GUI graph data shape.
fn parse_graph_content(content: &str) -> Result<serde_json::Value, AppError> {
    let trimmed = content.trim_start();
    let is_mlir = trimmed.starts_with("module") || trimmed.starts_with("func.func");

    if is_mlir {
        air_parse::parse_air_text(content)
            .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("failed to parse AIR: {e}")))
    } else {
        // Try strict AirModule deserialization first (expects SCREAMING_SNAKE_CASE op types)
        if let Ok(graph) = serde_json::from_str::<AirModule>(content) {
            return Ok(serde_json::to_value(graph).unwrap());
        }
        // Fallback: pass through raw JSON for studio-created graphs that use op names
        let val: serde_json::Value = serde_json::from_str(content).map_err(|e| {
            AppError(
                StatusCode::BAD_REQUEST,
                format!("failed to parse graph data: {e}"),
            )
        })?;
        // Validate it has the expected shape
        if val.get("nodes").is_some() && val.get("edges").is_some() {
            Ok(val)
        } else {
            Err(AppError(
                StatusCode::BAD_REQUEST,
                "JSON missing 'nodes' or 'edges' fields".into(),
            ))
        }
    }
}

/// GET /api/graph?path=<file> — read a graph file (.air MLIR text, .py, or JSON).
async fn graph_handler(Query(params): Query<PathParam>) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&params.path)?;
    let content = read_graph_content(&path).await?;
    let graph = parse_graph_content(&content)?;
    Ok(Json(graph))
}

/// GET /api/graph/analyze?path=<file> — return graph analysis JSON.
async fn graph_analyze_handler(Query(params): Query<PathParam>) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&params.path)?;
    let content = read_graph_content(&path).await?;
    let graph_json = parse_graph_content(&content)?;

    // The AIR parser outputs PascalCase op names (matching ops catalog) but
    // AirModule expects SCREAMING_SNAKE_CASE. Normalize before deserializing.
    let normalized = normalize_ops_for_air_module(graph_json);

    let graph: AirModule = serde_json::from_value(normalized)
        .map_err(|e| AppError(StatusCode::BAD_REQUEST, format!("failed to map graph: {e}")))?;

    let analysis = analyze_graph(&graph);
    Ok(Json(analysis))
}

/// Convert PascalCase op names to SCREAMING_SNAKE_CASE for AirModule deserialization.
fn normalize_ops_for_air_module(mut graph: serde_json::Value) -> serde_json::Value {
    if let Some(nodes) = graph.get_mut("nodes").and_then(|n| n.as_array_mut()) {
        for node in nodes {
            if let Some(op) = node.get("op").and_then(|o| o.as_str()) {
                let screaming = display_name_to_serde_variant(op);
                node.as_object_mut()
                    .unwrap()
                    .insert("op".into(), serde_json::Value::String(screaming));
            }
        }
    }
    graph
}

/// Map ops catalog display name → AISOperationType serde variant.
/// Handles cases where the display name differs from the enum variant.
fn display_name_to_serde_variant(display: &str) -> String {
    match display {
        "QueryMemory" => "QMEM".to_string(),
        "UpdateMemory" => "UMEM".to_string(),
        "InvokeTool" => "INV_TOOL".to_string(),
        "ExecuteCode" => "EXC".to_string(),
        "PrintOutput" => "PRINT".to_string(),
        "HandleError" => "ERR".to_string(),
        s if s.chars().all(|c| c.is_uppercase() || c == '_') => s.to_string(),
        s => {
            let mut result = String::with_capacity(s.len() + 4);
            for (i, c) in s.chars().enumerate() {
                if c.is_uppercase() && i > 0 {
                    result.push('_');
                }
                result.push(c.to_ascii_uppercase());
            }
            result
        }
    }
}

/// GET /api/ops — return the AIS operation catalog.
async fn ops_handler() -> impl IntoResponse {
    let ops: Vec<&OperationSpec> = get_all_operations().collect();
    Json(ops)
}

/// GET /api/passes — return the compiler pass pipeline metadata.
async fn passes_handler() -> impl IntoResponse {
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

/// GET /api/session?path=<dir> — read session directory, return combined JSON.
async fn session_handler(Query(params): Query<PathParam>) -> ApiResult<impl IntoResponse> {
    let dir = validate_path(&params.path)?;
    if !dir.is_dir() {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            format!("session directory not found: {}", params.path),
        ));
    }

    // Read manifest.json
    let manifest = read_json_file(&dir.join("manifest.json")).await?;

    // Read trace.ndjson (line-by-line JSON)
    let trace = read_ndjson_file(&dir.join("trace.ndjson")).await?;

    // Read results.json
    let results = read_json_file(&dir.join("results.json")).await?;

    // Read metrics.json
    let metrics = read_json_file(&dir.join("metrics.json")).await?;

    // Read node_statuses.json
    let node_statuses = read_json_file(&dir.join("node_statuses.json")).await?;

    // Build a node_id → directory name mapping from the nodes/ subdirectory
    let mut node_names: HashMap<String, String> = HashMap::new();
    let nodes_dir = dir.join("nodes");
    if nodes_dir.is_dir() {
        if let Ok(mut entries) = tokio::fs::read_dir(&nodes_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                // Parse leading digits: "01_coder" → id="1", display="coder"
                if let Some(idx) = name.find('_') {
                    let id_str = name[..idx].trim_start_matches('0');
                    let id = if id_str.is_empty() { "0" } else { id_str };
                    let display = &name[idx + 1..];
                    node_names.insert(id.to_string(), display.to_string());
                }
            }
        }
    }

    // Detect source workflow path by scanning for matching graph_name
    let graph_name = manifest
        .get("graph_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut source_path: Option<String> = None;
    if !graph_name.is_empty() {
        let cwd = std::env::current_dir().unwrap_or_default();
        // Check common locations for the source workflow
        for ext in &["py", "air"] {
            let candidate = cwd
                .join("examples")
                .join("python")
                .join(format!("{graph_name}.{ext}"));
            if candidate.exists() {
                source_path = Some(candidate.to_string_lossy().to_string());
                break;
            }
            // Also check direct cwd
            let candidate2 = cwd.join(format!("{graph_name}.{ext}"));
            if candidate2.exists() {
                source_path = Some(candidate2.to_string_lossy().to_string());
                break;
            }
        }
        // Broader scan if not found
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

/// GET /api/session/node/:id?session=<dir> — return per-node output.
async fn session_node_handler(
    Path(node_id): Path<String>,
    Query(params): Query<SessionNodeParam>,
) -> ApiResult<impl IntoResponse> {
    let session_dir = validate_path(&params.session)?;
    if !session_dir.is_dir() {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            format!("session directory not found: {}", params.session),
        ));
    }

    let nodes_dir = session_dir.join("nodes");
    if !nodes_dir.is_dir() {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            "nodes directory not found in session".to_string(),
        ));
    }

    // Node directories are named like "01_spawn_architect" — match by leading ID.
    let mut target_dir: Option<PathBuf> = None;
    if let Ok(mut entries) = tokio::fs::read_dir(&nodes_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Match prefix: "01_" or just "1_"
            if name_str.starts_with(&format!("{}_", node_id))
                || name_str.starts_with(&format!("{:02}_", node_id.parse::<u64>().unwrap_or(0)))
            {
                target_dir = Some(entry.path());
                break;
            }
        }
    }

    let target_dir = target_dir.ok_or_else(|| {
        AppError(
            StatusCode::NOT_FOUND,
            format!("node directory not found for id: {node_id}"),
        )
    })?;

    // Gather all JSON files in the node directory
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

    // Also read trace.ndjson if present
    let trace_path = target_dir.join("trace.ndjson");
    if trace_path.exists() {
        if let Ok(trace) = read_ndjson_file(&trace_path).await {
            result.insert("trace".to_string(), trace);
        }
    }

    // Read text files (prompt.txt, response.txt)
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

/// GET /api/startup — return the initial file path and available examples.
async fn startup_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl IntoResponse {
    Json(serde_json::json!({
        "initial_file": state.initial_file,
    }))
}

/// GET /api/examples — recursively list workflow files under the examples directory.
async fn examples_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> ApiResult<impl IntoResponse> {
    let examples_dir = match &state.examples_dir {
        Some(d) if d.is_dir() => d.clone(),
        _ => {
            return Ok(Json(serde_json::json!({ "examples": [] })));
        }
    };

    let mut examples: Vec<serde_json::Value> = Vec::new();
    collect_graphs(&examples_dir, &examples_dir, 0, &mut examples).await;

    // Sort by relative path.
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

/// Whether a directory name represents build artifacts or caches that should be
/// skipped during filesystem scans.
fn is_skip_dir(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules"
                | "target"
                | "__pycache__"
                | "frontend-dist"
                | "dist"
                | "build"
                | "venv"
                | "site-packages"
        )
}

/// GET /api/file?path=<file> — return raw file content as plain text.
async fn file_handler(Query(params): Query<PathParam>) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&params.path)?;
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| AppError(StatusCode::NOT_FOUND, format!("failed to read file: {e}")))?;
    Ok(content)
}

/// Extract workflow metadata from a `.py` file by scanning for `@compile` and docstrings.
fn extract_py_metadata(content: &str) -> (Option<String>, Option<String>) {
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
async fn collect_graphs(
    base: &std::path::Path,
    dir: &std::path::Path,
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

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "air" | "py") {
            continue;
        }

        // Skip __init__.py files and test files
        if ext == "py" && (name_str == "__init__.py" || name_str.starts_with("test_")) {
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

        let (graph_name, node_count, description, parameters, source_type) = match ext {
            "py" => {
                let (func_name, desc) = content
                    .as_deref()
                    .map(extract_py_metadata)
                    .unwrap_or((None, None));
                // Only include .py files that use @compile (are actual workflows)
                if func_name.is_none() {
                    if content.as_deref().map_or(true, |c| {
                        !c.contains("@compile") && !c.contains("GraphRecorder")
                    }) {
                        continue;
                    }
                }
                (func_name, None, desc, None::<Vec<serde_json::Value>>, "py")
            }
            "air" => {
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
                                (name, count, None, params, "air")
                            }
                            Err(_) => (None, None, None, None, "air"),
                        }
                    } else {
                        let parsed = serde_json::from_str::<serde_json::Value>(text).ok();
                        let name = parsed
                            .as_ref()
                            .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from));
                        let count = parsed.as_ref().and_then(|v| {
                            v.get("nodes").and_then(|n| n.as_array()).map(|a| a.len())
                        });
                        let params = parsed
                            .as_ref()
                            .and_then(|v| v.get("parameters").and_then(|p| p.as_array()).cloned());
                        (name, count, None, params, "air")
                    }
                } else {
                    (None, None, None, None, "air")
                }
            }
            _ => {
                continue;
            }
        };

        let mut entry = serde_json::json!({
            "path": abs,
            "relative_path": rel,
            "name": graph_name.unwrap_or_else(|| {
                // Use filename stem as fallback name
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

/// GET /api/workflows — scan cwd recursively for workflow files.
/// Pairs `.py` + `.air` by basename in the same directory.
async fn workflows_handler() -> ApiResult<impl IntoResponse> {
    let cwd = std::env::current_dir().map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to get current directory: {e}"),
        )
    })?;

    let mut raw: Vec<serde_json::Value> = Vec::new();
    collect_graphs(&cwd, &cwd, 0, &mut raw).await;

    // Pair .py + .air by (directory, stem): when both exist, keep one entry
    // with source_type "py" and propagate the .air's node_count + parameters.
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
            // Find the .py entry (primary) and .air (secondary)
            let py_idx = group
                .iter()
                .position(|e| e.get("source_type").and_then(|v| v.as_str()) == Some("py"));
            if let Some(idx) = py_idx {
                let mut primary = group.remove(idx);
                // Merge metadata from the companion .air
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
                    // Store companion path for graph visualization
                    primary["air_path"] = companion
                        .get("path")
                        .cloned()
                        .unwrap_or(serde_json::json!(null));
                }
                workflows.push(primary);
            } else {
                // No .py — just emit all entries
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

// ---------------------------------------------------------------------------
// Compile & Execute endpoints
// ---------------------------------------------------------------------------

/// Request body for POST /api/compile.
#[derive(Deserialize)]
struct CompileRequest {
    path: String,
    #[serde(default = "default_opt_level")]
    opt_level: u8,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    no_cse_llm: bool,
}

fn default_opt_level() -> u8 {
    1
}

/// Per-pass metrics + metadata for the pipeline visualization.
#[derive(Serialize, Clone)]
struct PassMetricEntry {
    pass_name: String,
    category: String,
    summary: String,
    description: String,
    duration_ms: Option<f64>,
    ops_before: Option<usize>,
    ops_after: Option<usize>,
    ops_delta: Option<isize>,
}

/// Aggregate pass-pipeline statistics.
#[derive(Serialize, Clone)]
struct PassSummary {
    total_passes: usize,
    initial_ops: usize,
    final_ops: usize,
    total_ops_eliminated: usize,
    active_passes: Vec<String>,
    total_duration_ms: f64,
}

/// Response for POST /api/compile.
#[derive(Serialize)]
struct CompileResponse {
    success: bool,
    artifact_path: Option<String>,
    passes: Vec<String>,
    pass_metrics: Vec<PassMetricEntry>,
    pass_summary: Option<PassSummary>,
    duration_ms: u64,
    stdout: String,
    stderr: String,
    node_count_before: Option<usize>,
    node_count_after: Option<usize>,
    diagnostics: Option<serde_json::Value>,
}

/// POST /api/compile — compile a workflow and return results.
async fn compile_handler(
    axum::extract::Json(req): axum::extract::Json<CompileRequest>,
) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&req.path)?;

    // Read source to get before-node-count
    let node_count_before = if let Ok(content) = tokio::fs::read_to_string(&path).await {
        parse_node_count(&content)
    } else {
        None
    };

    let cli_bin = find_apxm_cli();

    // Build the enriched pass list using the compiler's real pass ordering + metadata
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

    // Write diagnostics to a temp file so we can get per-pass metrics
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
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to spawn compile: {e}"),
            )
        })?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Determine artifact path (same stem + .apxmobj)
    let artifact_path = path.with_extension("apxmobj");
    let artifact_exists = artifact_path.exists();

    // Get after-node-count from the artifact by decompiling
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

    // Parse the diagnostics file for per-pass metrics and merge into pass_metrics
    let mut pass_summary: Option<PassSummary> = None;
    let diagnostics = if let Ok(diag_content) = tokio::fs::read_to_string(&diag_path).await {
        let _ = tokio::fs::remove_file(&diag_path).await;
        if let Ok(diag_val) = serde_json::from_str::<serde_json::Value>(&diag_content) {
            // Merge per-pass timing/ops into our enriched pass_metrics
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
            // Build aggregate summary from diagnostics
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

    let passes: Vec<String> = pass_names;

    Ok(Json(CompileResponse {
        success: output.status.success(),
        artifact_path: if artifact_exists {
            Some(artifact_path.to_string_lossy().to_string())
        } else {
            None
        },
        passes,
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

fn parse_node_count(content: &str) -> Option<usize> {
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

fn parse_compile_opt_level(level: u8) -> OptimizationLevel {
    match level {
        0 => OptimizationLevel::O0,
        1 => OptimizationLevel::O1,
        2 => OptimizationLevel::O2,
        _ => OptimizationLevel::O3,
    }
}

fn find_apxm_cli() -> PathBuf {
    // Try the release binary next to this binary
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap_or(std::path::Path::new("."));
        let sibling = dir.join("apxm");
        if sibling.exists() {
            return sibling;
        }
    }
    // Try target/release/apxm relative to cwd
    let cwd_release = PathBuf::from("target/release/apxm");
    if cwd_release.exists() {
        return cwd_release;
    }
    // Fallback: hope it's on PATH
    PathBuf::from("apxm")
}

/// Create a `tokio::process::Command` for the CLI binary with
/// `LD_LIBRARY_PATH` set so it can find `libapxm_compiler_c.so`
/// and MLIR/LLVM shared libraries.  Mirrors the `[env]` section
/// that dekk sets from `.dekk.toml`.
fn apxm_command(cli_bin: &std::path::Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(cli_bin);

    let mut dirs: Vec<String> = Vec::new();
    if let Some(bin_dir) = cli_bin.parent().and_then(|p| std::fs::canonicalize(p).ok()) {
        let lib_dir = bin_dir.join("lib");
        if lib_dir.is_dir() {
            dirs.push(lib_dir.to_string_lossy().into_owned());
        }
        dirs.push(bin_dir.to_string_lossy().into_owned());
    }
    if let Ok(existing) = std::env::var("LD_LIBRARY_PATH") {
        if !existing.is_empty() {
            dirs.push(existing);
        }
    }
    if !dirs.is_empty() {
        cmd.env("LD_LIBRARY_PATH", dirs.join(":"));
    }

    cmd
}

/// Request body for POST /api/execute.
#[derive(Deserialize)]
struct ExecuteRequest {
    path: String,
    #[serde(default)]
    params: HashMap<String, String>,
    #[serde(default = "default_opt_level")]
    opt_level: u8,
}

/// Response for POST /api/execute.
#[derive(Serialize)]
struct ExecuteResponse {
    session_path: String,
    execution_id: String,
}

/// POST /api/execute — execute a workflow with --emit-session and return the session path.
async fn execute_handler(
    axum::extract::Json(req): axum::extract::Json<ExecuteRequest>,
) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&req.path)?;
    let cli_bin = find_apxm_cli();

    let mut cmd = apxm_command(&cli_bin);
    cmd.arg("execute")
        .arg(&path)
        .arg("--emit-session")
        .arg("--opt-level")
        .arg(req.opt_level.to_string());

    // Add parameters as positional args
    for (key, value) in &req.params {
        cmd.arg(format!("--param"));
        cmd.arg(format!("{key}={value}"));
    }

    cmd.current_dir(std::env::current_dir().unwrap_or_default());

    // Spawn the process in the background — don't wait for completion
    let child = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to spawn execute: {e}"),
            )
        })?;

    let pid = child.id().unwrap_or(0);

    // Wait briefly for the session directory to appear
    let sessions_dir = apxm_core::env::apxm_home().join("sessions");

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("workflow");

    // Poll for the session directory (up to 5 seconds)
    let mut session_path = String::new();
    let mut execution_id = String::new();
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Ok(mut entries) = tokio::fs::read_dir(&sessions_dir).await {
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
                // Check if this session was created very recently (within last 10 seconds)
                if modified.elapsed().map_or(false, |d| d.as_secs() < 10) {
                    let dir = sessions_dir.join(&name);
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
        // Could not find session — return the PID at least
        return Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "execution started (pid {pid}) but no session directory found — is --emit-session supported for this workflow?"
            ),
        ));
    }

    info!("Execution started: session={execution_id} pid={pid}");

    Ok(Json(ExecuteResponse {
        session_path,
        execution_id,
    }))
}

/// POST /api/graph/save — persist an ApxmGraph to a JSON file.
async fn save_graph_handler(
    Json(payload): Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let graph = payload
        .get("graph")
        .ok_or_else(|| AppError(StatusCode::BAD_REQUEST, "missing 'graph' field".into()))?;

    // Validate the graph has at least a name
    let graph_name = graph
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("untitled");

    // Determine output path
    let save_path = if let Some(p) = payload.get("path").and_then(|p| p.as_str()) {
        PathBuf::from(p)
    } else {
        // Default: save to CWD/workflows/<name>.apxm
        let dir = PathBuf::from("workflows");
        if !dir.exists() {
            std::fs::create_dir_all(&dir).map_err(|e| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("create dir: {e}"),
                )
            })?;
        }
        dir.join(format!("{}.air", graph_name))
    };

    // Write pretty-printed JSON
    let content = serde_json::to_string_pretty(graph)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("serialize: {e}")))?;

    tokio::fs::write(&save_path, &content).await.map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write file: {e}"),
        )
    })?;

    let abs_path = save_path
        .canonicalize()
        .unwrap_or_else(|_| save_path.clone())
        .display()
        .to_string();

    info!("Graph saved: {} ({} bytes)", abs_path, content.len());

    Ok(Json(serde_json::json!({ "path": abs_path })))
}

/// GET /api/filetree — return the project directory tree for server-side browsing.
async fn filetree_handler() -> ApiResult<impl IntoResponse> {
    let cwd = std::env::current_dir().map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to get current directory: {e}"),
        )
    })?;

    async fn build_tree(
        dir: &std::path::Path,
        base: &std::path::Path,
        depth: usize,
    ) -> Vec<serde_json::Value> {
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
                // Only include directories that contain relevant files
                if !children.is_empty() {
                    items.push(serde_json::json!({
                        "name": name,
                        "path": rel,
                        "is_dir": true,
                        "children": children,
                    }));
                }
            } else {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(ext, "air" | "py" | "toml") {
                    let mut entry_json = serde_json::json!({
                        "name": name,
                        "path": rel,
                        "is_dir": false,
                    });

                    if ext == "air" {
                        if let Ok(content) = tokio::fs::read_to_string(&path).await {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content)
                            {
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

/// Probe a backend endpoint for TCP reachability (2s timeout).
async fn probe_backend_status(endpoint: &str) -> String {
    // Parse host:port from URL
    let addr = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .unwrap_or(endpoint);
    // Remove trailing path
    let host_port = addr.split('/').next().unwrap_or(addr);

    let timeout = std::time::Duration::from_secs(2);
    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(host_port)).await {
        Ok(Ok(_)) => "healthy".to_string(),
        _ => "unreachable".to_string(),
    }
}

/// GET /api/health — return structured summary of system configuration.
/// With `?probe=true`, actively probes backend endpoints for reachability.
async fn health_handler(Query(params): Query<HealthParam>) -> ApiResult<impl IntoResponse> {
    let do_probe = params.probe.unwrap_or(false);
    let apxm_dir = apxm_core::env::apxm_home();
    let config_path = apxm_dir.join("config.toml");

    let mut backends_json: Vec<serde_json::Value> = Vec::new();
    let mut total_models: usize = 0;

    if config_path.exists() {
        if let Ok(content) = tokio::fs::read_to_string(&config_path).await {
            if let Ok(config) = content.parse::<toml::Value>() {
                if let Some(backends) = config.get("backends").and_then(|b| b.as_array()) {
                    // Collect backend info first
                    struct BackendInfo {
                        name: String,
                        endpoint: String,
                        protocol: String,
                        model_count: usize,
                    }
                    let infos: Vec<BackendInfo> = backends
                        .iter()
                        .map(|backend| {
                            let name = backend
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            let endpoint = backend
                                .get("endpoint")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let protocol = backend
                                .get("protocol")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            let model_count = backend
                                .get("models")
                                .and_then(|v| v.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0);
                            BackendInfo {
                                name,
                                endpoint,
                                protocol,
                                model_count,
                            }
                        })
                        .collect();

                    // Probe all backends concurrently
                    let statuses: Vec<String> = if do_probe {
                        let futs: Vec<_> = infos
                            .iter()
                            .map(|b| {
                                let ep = b.endpoint.clone();
                                async move {
                                    if ep.is_empty() {
                                        "unknown".to_string()
                                    } else {
                                        probe_backend_status(&ep).await
                                    }
                                }
                            })
                            .collect();
                        futures::future::join_all(futs).await
                    } else {
                        infos.iter().map(|_| "unknown".to_string()).collect()
                    };

                    for (info, status) in infos.iter().zip(statuses) {
                        total_models += info.model_count;
                        backends_json.push(serde_json::json!({
                            "name": info.name,
                            "endpoint": info.endpoint,
                            "protocol": info.protocol,
                            "model_count": info.model_count,
                            "status": status,
                        }));
                    }
                }
            }
        }
    }

    // Count agents and tools concurrently
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

    // Determine config source: project-local or global.
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

// ---------------------------------------------------------------------------
// Validate handler
// ---------------------------------------------------------------------------

/// POST /api/validate — validate a workflow against the AIS contract.
async fn validate_handler(
    axum::extract::Json(req): axum::extract::Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let path_str = req
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError(StatusCode::BAD_REQUEST, "missing 'path' field".into()))?;
    let path = validate_path(path_str)?;
    let cli_bin = find_apxm_cli();

    let output = apxm_command(&cli_bin)
        .arg("validate")
        .arg(&path)
        .current_dir(std::env::current_dir().unwrap_or_default())
        .output()
        .await
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to spawn validate: {e}"),
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Try to parse structured output
    let details = serde_json::from_str::<serde_json::Value>(&stdout).ok();

    Ok(Json(serde_json::json!({
        "valid": output.status.success(),
        "stdout": stdout,
        "stderr": stderr,
        "details": details,
    })))
}

// ---------------------------------------------------------------------------
// Decompile handler
// ---------------------------------------------------------------------------

/// POST /api/decompile — reverse-map a compiled artifact back to AIR.
async fn decompile_handler(
    axum::extract::Json(req): axum::extract::Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let path_str = req
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError(StatusCode::BAD_REQUEST, "missing 'path' field".into()))?;
    let path = validate_path(path_str)?;
    let cli_bin = find_apxm_cli();

    let output = apxm_command(&cli_bin)
        .arg("decompile")
        .arg(&path)
        .current_dir(std::env::current_dir().unwrap_or_default())
        .output()
        .await
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to spawn decompile: {e}"),
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let graph = serde_json::from_str::<serde_json::Value>(&stdout).ok();

    Ok(Json(serde_json::json!({
        "success": output.status.success(),
        "graph": graph,
        "stderr": stderr,
    })))
}

// ---------------------------------------------------------------------------
// Explain handler
// ---------------------------------------------------------------------------

/// GET /api/explain?path=<file> — human-readable walkthrough of a workflow.
async fn explain_handler(Query(params): Query<PathParam>) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&params.path)?;
    let cli_bin = find_apxm_cli();

    let output = apxm_command(&cli_bin)
        .arg("explain")
        .arg(&path)
        .current_dir(std::env::current_dir().unwrap_or_default())
        .output()
        .await
        .map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to spawn explain: {e}"),
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok(Json(serde_json::json!({
        "success": output.status.success(),
        "explanation": stdout,
        "stderr": stderr,
    })))
}

// ---------------------------------------------------------------------------
// Agents handler
// ---------------------------------------------------------------------------

/// GET /api/agents — list builtin agent profiles from the AIS.
async fn agents_handler() -> impl IntoResponse {
    let profiles: Vec<serde_json::Value> = get_all_operations()
        .filter(|op| {
            matches!(
                format!("{:?}", op.category).as_str(),
                "Coordination" | "Communication"
            )
        })
        .flat_map(|op| {
            op.fields
                .iter()
                .filter(|f| f.ref_type.is_some())
                .map(move |f| {
                    serde_json::json!({
                        "op": op.name,
                        "field": f.name,
                        "ref_type": format!("{:?}", f.ref_type),
                    })
                })
        })
        .collect();

    // Return builtin agent definitions for the GUI.
    // These are presentation defaults, not the shared operation contract.
    let builtin_agents = vec![
        serde_json::json!({ "name": "architect", "description": "Designs system architecture and high-level solutions", "skills": ["design", "planning", "analysis"], "category": "engineering" }),
        serde_json::json!({ "name": "coder", "description": "Implements code based on specifications", "skills": ["coding", "implementation", "debugging"], "category": "engineering" }),
        serde_json::json!({ "name": "reviewer", "description": "Reviews code for quality, bugs, and best practices", "skills": ["review", "testing", "quality"], "category": "engineering" }),
        serde_json::json!({ "name": "tester", "description": "Writes and runs tests to validate implementations", "skills": ["testing", "validation", "qa"], "category": "engineering" }),
        serde_json::json!({ "name": "researcher", "description": "Researches topics and gathers information", "skills": ["research", "analysis", "summarization"], "category": "knowledge" }),
        serde_json::json!({ "name": "planner", "description": "Creates detailed plans and task breakdowns", "skills": ["planning", "decomposition", "scheduling"], "category": "management" }),
        serde_json::json!({ "name": "coordinator", "description": "Orchestrates multi-agent collaboration", "skills": ["coordination", "delegation", "monitoring"], "category": "management" }),
        serde_json::json!({ "name": "writer", "description": "Creates documentation and written content", "skills": ["writing", "documentation", "communication"], "category": "content" }),
        serde_json::json!({ "name": "analyst", "description": "Analyzes data and produces insights", "skills": ["analysis", "statistics", "visualization"], "category": "knowledge" }),
        serde_json::json!({ "name": "debugger", "description": "Diagnoses and fixes bugs systematically", "skills": ["debugging", "diagnostics", "root-cause-analysis"], "category": "engineering" }),
        serde_json::json!({ "name": "optimizer", "description": "Optimizes performance and resource usage", "skills": ["optimization", "profiling", "benchmarking"], "category": "engineering" }),
        serde_json::json!({ "name": "security", "description": "Audits code and systems for security vulnerabilities", "skills": ["security", "auditing", "compliance"], "category": "engineering" }),
        serde_json::json!({ "name": "devops", "description": "Manages deployment, CI/CD, and infrastructure", "skills": ["deployment", "ci-cd", "infrastructure"], "category": "operations" }),
        serde_json::json!({ "name": "data_engineer", "description": "Designs and manages data pipelines", "skills": ["data", "pipelines", "etl"], "category": "data" }),
        serde_json::json!({ "name": "ml_engineer", "description": "Builds and deploys machine learning models", "skills": ["ml", "training", "inference"], "category": "data" }),
        serde_json::json!({ "name": "ux_designer", "description": "Designs user interfaces and experiences", "skills": ["design", "ux", "accessibility"], "category": "design" }),
    ];

    Json(serde_json::json!({
        "agents": builtin_agents,
        "operation_refs": profiles,
    }))
}

// ---------------------------------------------------------------------------
// Config update handler
// ---------------------------------------------------------------------------

/// POST /api/config/update — update ~/.apxm/config.toml with structured changes.
async fn config_update_handler(
    axum::extract::Json(payload): axum::extract::Json<serde_json::Value>,
) -> ApiResult<impl IntoResponse> {
    let config_path = apxm_core::env::apxm_home().join("config.toml");

    if let Some(content) = payload.get("content").and_then(|v| v.as_str()) {
        // Direct content write
        tokio::fs::write(&config_path, content).await.map_err(|e| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to write config: {e}"),
            )
        })?;
        info!("Config updated: {}", config_path.display());
        return Ok(Json(serde_json::json!({ "success": true })));
    }

    Err(AppError(
        StatusCode::BAD_REQUEST,
        "missing 'content' field".into(),
    ))
}

// ---------------------------------------------------------------------------
// Skills handler
// ---------------------------------------------------------------------------

/// `GET /api/skills`
///
/// Returns no skills while generated agent skill packaging is disabled.
async fn skills_handler() -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "skills": [] })))
}

/// `GET /api/skills/:name` — return full skill content including body markdown.
async fn skill_detail_handler(
    axum::extract::Path(name): axum::extract::Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(AppError(
        StatusCode::NOT_FOUND,
        format!("skill '{name}' not found"),
    ))
}

/// Extract backend entries from config text, trying TOML parse first then regex fallback.
pub(crate) fn extract_backends_from_config(content: &str) -> Vec<serde_json::Value> {
    // Try toml parse first
    if let Ok(config) = content.parse::<toml::Value>() {
        if let Some(backends) = config.get("backends").and_then(|b| b.as_array()) {
            return backends
                .iter()
                .map(|b| {
                    let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let endpoint = b.get("endpoint").and_then(|v| v.as_str()).unwrap_or("");
                    let protocol = b
                        .get("protocol")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let backend_type = b.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let models: Vec<serde_json::Value> = b
                        .get("models")
                        .and_then(|m| m.as_array())
                        .unwrap_or(&Vec::new())
                        .iter()
                        .map(|m| toml_model_to_json(m))
                        .collect();
                    serde_json::json!({
                        "name": name,
                        "endpoint": endpoint,
                        "protocol": protocol,
                        "backend_type": backend_type,
                        "model_count": models.len(),
                        "models": models,
                    })
                })
                .collect();
        }
    }

    // Fallback: line-based extraction for configs that fail strict TOML parse
    extract_backends_line_based(content)
}

fn toml_model_to_json(m: &toml::Value) -> serde_json::Value {
    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let aliases: Vec<&str> = m
        .get("aliases")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let context_window = m
        .get("context_window")
        .and_then(|v| v.as_integer())
        .unwrap_or(0);
    let supports_vision = m
        .get("supports_vision")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let supports_functions = m
        .get("supports_functions")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let tags: Vec<&str> = m
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    serde_json::json!({
        "id": id,
        "aliases": aliases,
        "context_window": context_window,
        "supports_vision": supports_vision,
        "supports_functions": supports_functions,
        "tags": tags,
    })
}

/// Line-based fallback parser for [[backends]] and [[backends.models]] sections.
fn extract_backends_line_based(content: &str) -> Vec<serde_json::Value> {
    let mut backends: Vec<serde_json::Value> = Vec::new();
    let mut current_backend: Option<serde_json::Map<String, serde_json::Value>> = None;
    let mut current_model: Option<serde_json::Map<String, serde_json::Value>> = None;
    let mut in_models = false;
    let mut in_headers = false;

    fn flush_model(
        model: &mut Option<serde_json::Map<String, serde_json::Value>>,
        backend: &mut Option<serde_json::Map<String, serde_json::Value>>,
    ) {
        if let (Some(mut m), Some(b)) = (model.take(), backend.as_mut()) {
            // Ensure standard model fields exist with defaults
            m.entry("id").or_insert_with(|| serde_json::json!(""));
            m.entry("aliases").or_insert_with(|| serde_json::json!([]));
            m.entry("context_window")
                .or_insert_with(|| serde_json::json!(0));
            m.entry("supports_vision")
                .or_insert_with(|| serde_json::json!(false));
            m.entry("supports_functions")
                .or_insert_with(|| serde_json::json!(false));
            m.entry("tags").or_insert_with(|| serde_json::json!([]));
            let models = b.entry("models").or_insert_with(|| serde_json::json!([]));
            if let Some(arr) = models.as_array_mut() {
                arr.push(serde_json::Value::Object(m));
            }
        }
    }

    fn flush_backend(
        backend: &mut Option<serde_json::Map<String, serde_json::Value>>,
        list: &mut Vec<serde_json::Value>,
    ) {
        if let Some(mut b) = backend.take() {
            let models = b.entry("models").or_insert_with(|| serde_json::json!([]));
            let model_count = models.as_array().map(|a| a.len()).unwrap_or(0);
            b.insert("model_count".to_string(), serde_json::json!(model_count));
            if !b.contains_key("name") {
                b.insert("name".to_string(), serde_json::json!("unknown"));
            }
            if !b.contains_key("endpoint") {
                b.insert("endpoint".to_string(), serde_json::json!(""));
            }
            if !b.contains_key("protocol") {
                b.insert("protocol".to_string(), serde_json::json!("unknown"));
            }
            if !b.contains_key("backend_type") {
                b.insert("backend_type".to_string(), serde_json::json!("unknown"));
            }
            list.push(serde_json::Value::Object(b));
        }
    }

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed == "[[backends]]" {
            flush_model(&mut current_model, &mut current_backend);
            flush_backend(&mut current_backend, &mut backends);
            current_backend = Some(serde_json::Map::new());
            in_models = false;
            in_headers = false;
            continue;
        }

        if trimmed == "[[backends.models]]" {
            flush_model(&mut current_model, &mut current_backend);
            current_model = Some(serde_json::Map::new());
            in_models = true;
            in_headers = false;
            continue;
        }

        if trimmed == "[backends.headers]" {
            flush_model(&mut current_model, &mut current_backend);
            in_models = false;
            in_headers = true;
            continue;
        }

        // Any other section header
        if trimmed.starts_with('[') {
            flush_model(&mut current_model, &mut current_backend);
            in_models = false;
            in_headers = false;
            if !trimmed.starts_with("[[backends") && !trimmed.starts_with("[backends") {
                flush_backend(&mut current_backend, &mut backends);
            }
            continue;
        }

        // Skip header key-value pairs
        if in_headers {
            continue;
        }

        // Parse key = value
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim();
            let raw_val = trimmed[eq_pos + 1..].trim();

            let val = parse_toml_value(raw_val);

            if in_models {
                if let Some(ref mut m) = current_model {
                    m.insert(key.to_string(), val);
                }
            } else if let Some(ref mut b) = current_backend {
                // Skip sensitive fields
                if key == "api_key" {
                    continue;
                }
                let store_key = if key == "type" { "backend_type" } else { key };
                b.insert(store_key.to_string(), val);
            }
        }
    }

    flush_model(&mut current_model, &mut current_backend);
    flush_backend(&mut current_backend, &mut backends);

    backends
}

/// Parse a simple TOML value (string, number, bool, array of strings).
fn parse_toml_value(raw: &str) -> serde_json::Value {
    // Quoted string
    if raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2 {
        return serde_json::json!(raw[1..raw.len() - 1].replace("\\\"", "\""));
    }
    // Boolean
    if raw == "true" {
        return serde_json::json!(true);
    }
    if raw == "false" {
        return serde_json::json!(false);
    }
    // Integer
    if let Ok(n) = raw.parse::<i64>() {
        return serde_json::json!(n);
    }
    // Array of strings: ["a", "b", "c"]
    if raw.starts_with('[') && raw.ends_with(']') {
        let inner = &raw[1..raw.len() - 1];
        let items: Vec<serde_json::Value> = inner
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
                    serde_json::json!(&s[1..s.len() - 1])
                } else {
                    serde_json::json!(s)
                }
            })
            .collect();
        return serde_json::json!(items);
    }
    // env: reference or other bare string
    serde_json::json!(raw)
}

/// GET /api/backends — detailed backend and model listing from config.
async fn backends_handler(Query(params): Query<HealthParam>) -> ApiResult<impl IntoResponse> {
    let do_probe = params.probe.unwrap_or(false);
    let config_path = apxm_core::env::apxm_home().join("config.toml");

    if !config_path.exists() {
        return Ok(Json(serde_json::json!({ "backends": [] })));
    }

    let content = match tokio::fs::read_to_string(&config_path).await {
        Ok(c) => c,
        Err(_) => return Ok(Json(serde_json::json!({ "backends": [] }))),
    };

    // Parse config — extract backends as JSON values directly
    let backends_json_list = extract_backends_from_config(&content);

    // Probe backends concurrently if requested
    let statuses: Vec<String> = if do_probe {
        let futs: Vec<_> = backends_json_list
            .iter()
            .map(|b| {
                let ep = b
                    .get("endpoint")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                async move {
                    if ep.is_empty() {
                        "unknown".to_string()
                    } else {
                        probe_backend_status(&ep).await
                    }
                }
            })
            .collect();
        futures::future::join_all(futs).await
    } else {
        backends_json_list
            .iter()
            .map(|_| "unknown".to_string())
            .collect()
    };

    let backends_json: Vec<serde_json::Value> = backends_json_list
        .into_iter()
        .zip(statuses)
        .map(|(mut b, status)| {
            if let Some(obj) = b.as_object_mut() {
                obj.insert("status".to_string(), serde_json::json!(status));
            }
            b
        })
        .collect();

    Ok(Json(serde_json::json!({ "backends": backends_json })))
}

/// GET /api/config — read ~/.apxm/config.toml and return as text.
async fn config_handler() -> ApiResult<impl IntoResponse> {
    let config_path = apxm_core::env::apxm_home().join("config.toml");

    if !config_path.exists() {
        return Err(AppError(
            StatusCode::NOT_FOUND,
            "~/.apxm/config.toml not found".to_string(),
        ));
    }

    let content = tokio::fs::read_to_string(&config_path).await.map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read config: {e}"),
        )
    })?;

    Ok((
        StatusCode::OK,
        [("content-type", "text/plain; charset=utf-8")],
        content,
    ))
}

// ---------------------------------------------------------------------------
// File reading helpers
// ---------------------------------------------------------------------------

/// Read a JSON file and return it as a serde_json::Value.
/// Returns `null` if the file does not exist.
async fn read_json_file(path: &std::path::Path) -> Result<serde_json::Value, AppError> {
    if !path.exists() {
        return Ok(serde_json::Value::Null);
    }
    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read {}: {e}", path.display()),
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("invalid JSON in {}: {e}", path.display()),
        )
    })?;
    Ok(value)
}

/// Read an NDJSON file (one JSON object per line) and return as a JSON array.
/// Returns an empty array if the file does not exist.
async fn read_ndjson_file(path: &std::path::Path) -> Result<serde_json::Value, AppError> {
    if !path.exists() {
        return Ok(serde_json::Value::Array(vec![]));
    }
    let content = tokio::fs::read_to_string(path).await.map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read {}: {e}", path.display()),
        )
    })?;

    let events: Vec<serde_json::Value> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    Ok(serde_json::Value::Array(events))
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "apxm_gui=info,tower_http=info".parse().unwrap()),
        )
        .init();

    // Determine port: CLI arg > env var > default 18801
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = args
        .iter()
        .position(|a| a == "--port" || a == "-p")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .or_else(|| {
            std::env::var("APXM_GUI_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(18801);

    // Frontend dist directory: frontend-dist/ relative to the binary's manifest dir,
    // or overridden via --static-dir.
    let static_dir = args
        .iter()
        .position(|a| a == "--static-dir")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("frontend-dist")
        });

    // --file <path>: initial graph file to load on startup.
    let initial_file = args
        .iter()
        .position(|a| a == "--file")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .or_else(|| {
            args.last()
                .filter(|a| (a.ends_with(".air") || a.ends_with(".py")) && !a.starts_with("--"))
                .cloned()
        })
        .map(|f| {
            // Resolve to absolute path for the API.
            let p = PathBuf::from(&f);
            if p.is_absolute() {
                f
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(&p).to_string_lossy().to_string())
                    .unwrap_or(f)
            }
        });

    // --examples-dir <path>: directory to scan for example graphs.
    // Defaults to <project>/examples/ if it exists.
    let examples_dir = args
        .iter()
        .position(|a| a == "--examples-dir")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .or_else(|| {
            // Try APXM_HOME/examples, then relative examples/
            std::env::var("APXM_HOME")
                .ok()
                .map(|h| PathBuf::from(h).join("examples"))
                .filter(|p| p.is_dir())
                .or_else(|| {
                    let local = PathBuf::from("examples");
                    if local.is_dir() { Some(local) } else { None }
                })
        });

    if let Some(ref f) = initial_file {
        info!("Initial file: {f}");
    }
    if let Some(ref d) = examples_dir {
        info!("Examples dir: {}", d.display());
    }

    let state = Arc::new(AppState {
        initial_file,
        examples_dir,
        agent_sessions: DashMap::new(),
    });

    // Build router
    let app = Router::new()
        // Page routes
        .route("/", axum::routing::get(index_handler))
        // API routes
        .route("/api/graph", axum::routing::get(graph_handler))
        .route(
            "/api/graph/analyze",
            axum::routing::get(graph_analyze_handler),
        )
        .route("/api/ops", axum::routing::get(ops_handler))
        .route("/api/passes", axum::routing::get(passes_handler))
        .route("/api/session", axum::routing::get(session_handler))
        .route(
            "/api/session/node/{id}",
            axum::routing::get(session_node_handler),
        )
        .route("/api/config", axum::routing::get(config_handler))
        .route("/api/workflows", axum::routing::get(workflows_handler))
        .route("/api/file", axum::routing::get(file_handler))
        .route("/api/health", axum::routing::get(health_handler))
        .route("/api/backends", axum::routing::get(backends_handler))
        .route("/api/filetree", axum::routing::get(filetree_handler))
        // Startup & examples
        .route("/api/startup", axum::routing::get(startup_handler))
        .route("/api/examples", axum::routing::get(examples_handler))
        // Compile & Execute
        .route("/api/compile", axum::routing::post(compile_handler))
        .route("/api/execute", axum::routing::post(execute_handler))
        .route("/api/graph/save", axum::routing::post(save_graph_handler))
        // Validate, Decompile, Explain
        .route("/api/validate", axum::routing::post(validate_handler))
        .route("/api/decompile", axum::routing::post(decompile_handler))
        .route("/api/explain", axum::routing::get(explain_handler))
        // Agents
        .route("/api/agents", axum::routing::get(agents_handler))
        // Config update
        .route(
            "/api/config/update",
            axum::routing::post(config_update_handler),
        )
        // Chat endpoints
        .route("/api/chat", axum::routing::post(api::chat::chat_handler))
        .route(
            "/api/chat/models",
            axum::routing::get(api::chat::models_handler),
        )
        // Agent (ACP) endpoints
        .route(
            "/api/agent/chat",
            axum::routing::post(api::agent::agent_chat),
        )
        .route(
            "/api/agent/profiles",
            axum::routing::get(api::agent::list_agent_profiles),
        )
        .route(
            "/api/agent/sessions",
            axum::routing::get(api::agent::list_agent_sessions),
        )
        .route(
            "/api/agent/sessions/{id}",
            axum::routing::delete(api::agent::delete_agent_session),
        )
        // Skills
        .route("/api/skills", axum::routing::get(skills_handler))
        .route(
            "/api/skills/{name}",
            axum::routing::get(skill_detail_handler),
        )
        // Live SSE endpoints
        .route(
            "/api/live/session",
            axum::routing::get(api::live::sse_session_stream),
        )
        .route(
            "/api/live/node/{id}",
            axum::routing::get(api::live::sse_node_output),
        )
        .route(
            "/api/sessions",
            axum::routing::get(api::live::list_sessions),
        )
        // Serve frontend assets (JS, CSS)
        .nest_service("/assets", ServeDir::new(static_dir.join("assets")))
        // SPA fallback: all non-/api/ routes serve index.html for client-side routing
        .fallback(axum::routing::get(spa_fallback))
        // CORS for local dev
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    info!("APXM GUI listening on http://localhost:{port}");
    info!("Static files: {}", static_dir.display());

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind address");

    // Graceful shutdown on SIGINT/SIGTERM
    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("failed to install SIGTERM handler");
            tokio::select! {
                _ = ctrl_c => {},
                _ = sigterm.recv() => {},
            }
        }
        #[cfg(not(unix))]
        ctrl_c.await.ok();

        info!("shutdown signal received");
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .expect("server error");
}
