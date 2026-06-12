//! Per-session execution lane guard.
//!
//! Ensures requests for the same session execute serially while allowing
//! different sessions to run concurrently.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Session lane coordinator.
#[derive(Default)]
pub struct SessionLaneGuard {
    lanes: DashMap<String, Arc<Mutex<()>>>,
}

impl SessionLaneGuard {
    /// Create an empty lane guard.
    pub fn new() -> Self {
        Self {
            lanes: DashMap::new(),
        }
    }

    /// Acquire the lane for a session.
    pub async fn acquire(&self, session_id: &str) -> SessionLanePermit {
        let lane = self
            .lanes
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let guard = lane.lock_owned().await;
        SessionLanePermit {
            _guard: guard,
            session_id: session_id.to_string(),
        }
    }
}

/// Held while a session lane lock is active.
pub struct SessionLanePermit {
    _guard: OwnedMutexGuard<()>,
    #[allow(dead_code)]
    session_id: String,
}

