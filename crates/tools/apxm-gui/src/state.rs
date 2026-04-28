//! Shared application state for axum handlers.

use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Mutex;

use crate::acp_client;

pub struct AppState {
    pub initial_file: Option<String>,
    pub examples_dir: Option<PathBuf>,
    pub agent_sessions: DashMap<String, Arc<Mutex<acp_client::AgentSession>>>,
}

impl AppState {
    pub fn new(initial_file: Option<String>, examples_dir: Option<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            initial_file,
            examples_dir,
            agent_sessions: DashMap::new(),
        })
    }
}
