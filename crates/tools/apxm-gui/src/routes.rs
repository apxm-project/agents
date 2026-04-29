//! Router construction and route-path constants.

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post};
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use crate::api;
use crate::state::AppState;

/// API route path constants.
pub mod paths {
    // Pages
    pub const ROOT: &str = "/";

    // Graph
    pub const GRAPH: &str = "/api/graph";
    pub const GRAPH_ANALYZE: &str = "/api/graph/analyze";
    pub const GRAPH_SAVE: &str = "/api/graph/save";
    pub const OPS: &str = "/api/ops";
    pub const PASSES: &str = "/api/passes";

    // Sessions
    pub const SESSION: &str = "/api/session";
    pub const SESSION_NODE: &str = "/api/session/node/{id}";
    pub const SESSIONS: &str = "/api/sessions";

    // Files
    pub const CONFIG: &str = "/api/config";
    pub const CONFIG_UPDATE: &str = "/api/config/update";
    pub const WORKFLOWS: &str = "/api/workflows";
    pub const FILE: &str = "/api/file";
    pub const FILETREE: &str = "/api/filetree";
    pub const STARTUP: &str = "/api/startup";
    pub const EXAMPLES: &str = "/api/examples";

    // Health & backends
    pub const HEALTH: &str = "/api/health";
    pub const BACKENDS: &str = "/api/backends";

    // Compile / execute / validate / decompile / explain
    pub const COMPILE: &str = "/api/compile";
    pub const EXECUTE: &str = "/api/execute";
    pub const VALIDATE: &str = "/api/validate";
    pub const DECOMPILE: &str = "/api/decompile";
    pub const EXPLAIN: &str = "/api/explain";

    // Agents
    pub const AGENTS: &str = "/api/agents";

    // Chat
    pub const CHAT: &str = "/api/chat";
    pub const CHAT_MODELS: &str = "/api/chat/models";

    // Agent (ACP)
    pub const AGENT_CHAT: &str = "/api/agent/chat";
    pub const AGENT_PROFILES: &str = "/api/agent/profiles";
    pub const AGENT_SESSIONS: &str = "/api/agent/sessions";
    pub const AGENT_SESSION_BY_ID: &str = "/api/agent/sessions/{id}";

    // Skills
    pub const SKILLS: &str = "/api/skills";
    pub const SKILL_BY_NAME: &str = "/api/skills/{name}";

    // Live SSE
    pub const LIVE_SESSION: &str = "/api/live/session";
    pub const LIVE_NODE: &str = "/api/live/node/{id}";

    // Static assets
    pub const ASSETS: &str = "/assets";
    pub const ASSETS_DIR: &str = "assets";
}

/// Build the axum router with all routes wired up.
pub fn build_router(state: Arc<AppState>, static_dir: &Path) -> Router {
    Router::new()
        .route(paths::ROOT, get(api::pages::index_handler))
        .route(paths::GRAPH, get(api::graph::graph_handler))
        .route(paths::GRAPH_ANALYZE, get(api::graph::graph_analyze_handler))
        .route(paths::OPS, get(api::graph::ops_handler))
        .route(paths::PASSES, get(api::graph::passes_handler))
        .route(paths::SESSION, get(api::session::session_handler))
        .route(paths::SESSION_NODE, get(api::session::session_node_handler))
        .route(paths::CONFIG, get(api::config::config_handler))
        .route(paths::WORKFLOWS, get(api::files::workflows_handler))
        .route(paths::FILE, get(api::files::file_handler))
        .route(paths::HEALTH, get(api::health::health_handler))
        .route(paths::BACKENDS, get(api::backend::backends_handler))
        .route(paths::FILETREE, get(api::files::filetree_handler))
        .route(paths::STARTUP, get(api::files::startup_handler))
        .route(paths::EXAMPLES, get(api::files::examples_handler))
        .route(paths::COMPILE, post(api::compile::compile_handler))
        .route(paths::EXECUTE, post(api::execute::execute_handler))
        .route(paths::GRAPH_SAVE, post(api::graph::save_graph_handler))
        .route(paths::VALIDATE, post(api::execute::validate_handler))
        .route(paths::DECOMPILE, post(api::execute::decompile_handler))
        .route(paths::EXPLAIN, get(api::execute::explain_handler))
        .route(paths::AGENTS, get(api::agents::agents_handler))
        .route(
            paths::CONFIG_UPDATE,
            post(api::config::config_update_handler),
        )
        .route(paths::CHAT, post(api::chat::chat_handler))
        .route(paths::CHAT_MODELS, get(api::chat::models_handler))
        .route(paths::AGENT_CHAT, post(api::agent::agent_chat))
        .route(paths::AGENT_PROFILES, get(api::agent::list_agent_profiles))
        .route(paths::AGENT_SESSIONS, get(api::agent::list_agent_sessions))
        .route(
            paths::AGENT_SESSION_BY_ID,
            delete(api::agent::delete_agent_session),
        )
        .route(paths::SKILLS, get(api::skills::skills_handler))
        .route(paths::SKILL_BY_NAME, get(api::skills::skill_detail_handler))
        .route(paths::LIVE_SESSION, get(api::live::sse_session_stream))
        .route(paths::LIVE_NODE, get(api::live::sse_node_output))
        .route(paths::SESSIONS, get(api::live::list_sessions))
        .nest_service(
            paths::ASSETS,
            ServeDir::new(static_dir.join(paths::ASSETS_DIR)),
        )
        .fallback(get(api::pages::spa_fallback))
        .layer(CorsLayer::permissive())
        .with_state(state)
}
