//! APXM GUI — Axum web server for visualizing agent workflow graphs,
//! compiler optimizations, and session traces.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Json, Response};
use axum::Router;
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing::{error, info};

use apxm_compiler::AirModule;

mod api;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

/// Shared application state.
#[derive(Clone)]
struct AppState {
    /// Path to the directory containing static assets (CSS, JS, images).
    #[allow(dead_code)]
    static_dir: PathBuf,
    /// Initial graph file to load on startup (from `--file` arg).
    initial_file: Option<String>,
    /// Directory to scan for example `.apxm` files.
    examples_dir: Option<PathBuf>,
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
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".into()));
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
        Err(AppError(StatusCode::FORBIDDEN, "path outside allowed directories".into()))
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

/// GET /api/graph?path=<file> — read an `.apxm` file and return parsed graph JSON.
async fn graph_handler(Query(params): Query<PathParam>) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&params.path)?;

    let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
        error!(path = %params.path, error = %e, "failed to read graph file");
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read file: {e}"),
        )
    })?;

    let graph: AirModule = serde_json::from_str(&content).map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("failed to parse graph: {e}"),
        )
    })?;

    Ok(Json(graph))
}

/// GET /api/graph/analyze?path=<file> — return graph analysis JSON.
async fn graph_analyze_handler(
    Query(params): Query<PathParam>,
) -> ApiResult<impl IntoResponse> {
    let path = validate_path(&params.path)?;
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| {
            AppError(
                StatusCode::NOT_FOUND,
                format!("failed to read file: {e}"),
            )
        })?;

    let graph: AirModule = serde_json::from_str(&content).map_err(|e| {
        AppError(
            StatusCode::BAD_REQUEST,
            format!("failed to parse graph: {e}"),
        )
    })?;

    let analysis = analyze_graph(&graph);
    Ok(Json(analysis))
}

/// GET /api/ops — return the AIS operation catalog.
async fn ops_handler() -> impl IntoResponse {
    let ops: Vec<&apxm_ais::OperationSpec> = apxm_ais::get_all_operations().collect();
    Json(ops)
}

/// GET /api/passes — return the compiler pass pipeline metadata.
async fn passes_handler() -> impl IntoResponse {
    let passes: Vec<serde_json::Value> = apxm_ais::passes::get_all_passes()
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

    Ok(Json(serde_json::json!({
        "manifest": manifest,
        "trace": trace,
        "results": results,
        "metrics": metrics,
        "node_statuses": node_statuses,
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

    let files = &[
        "node.json",
        "output.json",
        "status.json",
        "live.json",
    ];

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
        let pa = a.get("relative_path").and_then(|v| v.as_str()).unwrap_or("");
        let pb = b.get("relative_path").and_then(|v| v.as_str()).unwrap_or("");
        pa.cmp(pb)
    });

    Ok(Json(serde_json::json!({ "examples": examples })))
}

/// Whether a directory name represents build artifacts, caches, or generated output
/// that should be skipped when scanning for workflow files.
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

/// Recursively find all `.apxm` graph files under `dir`.
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
        } else if path.extension().map_or(false, |e| e == "apxm") {
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

            let (graph_name, node_count) = match tokio::fs::read_to_string(&path).await {
                Ok(content) => {
                    let parsed = serde_json::from_str::<serde_json::Value>(&content).ok();
                    let name = parsed
                        .as_ref()
                        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from));
                    let count = parsed
                        .as_ref()
                        .and_then(|v| v.get("nodes").and_then(|n| n.as_array()).map(|a| a.len()));
                    (name, count)
                }
                Err(_) => (None, None),
            };

            out.push(serde_json::json!({
                "path": abs,
                "relative_path": rel,
                "name": graph_name.unwrap_or_else(|| rel.clone()),
                "category": category,
                "node_count": node_count,
            }));
        }
    }
}

/// GET /api/workflows — scan cwd recursively for `.apxm` graph files.
async fn workflows_handler() -> ApiResult<impl IntoResponse> {
    let cwd = std::env::current_dir().map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to get current directory: {e}"),
        )
    })?;

    let mut workflows: Vec<serde_json::Value> = Vec::new();
    collect_graphs(&cwd, &cwd, 0, &mut workflows).await;

    // Sort by relative path.
    workflows.sort_by(|a, b| {
        let pa = a.get("relative_path").and_then(|v| v.as_str()).unwrap_or("");
        let pb = b.get("relative_path").and_then(|v| v.as_str()).unwrap_or("");
        pa.cmp(pb)
    });

    Ok(Json(serde_json::json!({ "workflows": workflows })))
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
                if matches!(ext, "apxm" | "py" | "toml") {
                    let mut entry_json = serde_json::json!({
                        "name": name,
                        "path": rel,
                        "is_dir": false,
                    });

                    // For .apxm files, extract graph metadata
                    if ext == "apxm" {
                        if let Ok(content) = tokio::fs::read_to_string(&path).await {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                                let graph_name = parsed.get("name").and_then(|n| n.as_str());
                                let node_count = parsed.get("nodes").and_then(|n| n.as_array()).map(|a| a.len());
                                entry_json["apxm_meta"] = serde_json::json!({
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
async fn health_handler(
    Query(params): Query<HealthParam>,
) -> ApiResult<impl IntoResponse> {
    let do_probe = params.probe.unwrap_or(false);
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let apxm_dir = PathBuf::from(&home).join(".apxm");
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
                            let name = backend.get("name").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                            let endpoint = backend.get("endpoint").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let protocol = backend.get("protocol").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                            let model_count = backend.get("models").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                            BackendInfo { name, endpoint, protocol, model_count }
                        })
                        .collect();

                    // Probe all backends concurrently
                    let statuses: Vec<String> = if do_probe {
                        let futs: Vec<_> = infos.iter().map(|b| {
                            let ep = b.endpoint.clone();
                            async move {
                                if ep.is_empty() { "unknown".to_string() } else { probe_backend_status(&ep).await }
                            }
                        }).collect();
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
            if !agents_path.exists() { return 0; }
            tokio::fs::read_to_string(&agents_path)
                .await
                .ok()
                .and_then(|content| content.parse::<toml::Value>().ok())
                .and_then(|v| v.get("agents").and_then(|a| a.as_array()).map(|a| a.len()))
                .unwrap_or(0)
        },
        async {
            if !tools_path.exists() { return 0; }
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

/// GET /api/config — read ~/.apxm/config.toml and return as text.
async fn config_handler() -> ApiResult<impl IntoResponse> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let config_path = PathBuf::from(home).join(".apxm").join("config.toml");

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
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("frontend-dist")
        });

    // --file <path>: initial graph file to load on startup.
    let initial_file = args
        .iter()
        .position(|a| a == "--file")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .or_else(|| {
            // Also accept a bare positional .apxm arg (last arg not starting with --)
            args.last()
                .filter(|a| a.ends_with(".apxm") && !a.starts_with("--"))
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
        static_dir: static_dir.clone(),
        initial_file,
        examples_dir,
    });

    // Build router
    let app = Router::new()
        // Page routes
        .route("/", axum::routing::get(index_handler))
        // API routes
        .route("/api/graph", axum::routing::get(graph_handler))
        .route("/api/graph/analyze", axum::routing::get(graph_analyze_handler))
        .route("/api/ops", axum::routing::get(ops_handler))
        .route("/api/passes", axum::routing::get(passes_handler))
        .route("/api/session", axum::routing::get(session_handler))
        .route(
            "/api/session/node/{id}",
            axum::routing::get(session_node_handler),
        )
        .route("/api/config", axum::routing::get(config_handler))
        .route("/api/workflows", axum::routing::get(workflows_handler))
        .route("/api/health", axum::routing::get(health_handler))
        .route("/api/filetree", axum::routing::get(filetree_handler))
        // Startup & examples
        .route("/api/startup", axum::routing::get(startup_handler))
        .route("/api/examples", axum::routing::get(examples_handler))
        // Live SSE endpoints
        .route("/api/live/session", axum::routing::get(api::live::sse_session_stream))
        .route("/api/live/node/{id}", axum::routing::get(api::live::sse_node_output))
        .route("/api/sessions", axum::routing::get(api::live::list_sessions))
        // Serve frontend assets (JS, CSS)
        .nest_service("/assets", ServeDir::new(static_dir.join("assets")))
        // SPA fallback: all non-/api/ routes serve index.html for client-side routing
        .fallback(axum::routing::get(spa_fallback))
        // CORS for local dev
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    info!("APXM GUI listening on http://localhost:{port}");
    info!("Static files: {}", static_dir.display());

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("server error");
}
