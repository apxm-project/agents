//! Per-session execution lane guard ([`SessionLaneGuard`]).
//!
//! Webhook/trigger connectivity relies on this for **same-chat serialization**:
//! apxm-os mints `session_id = <agent>-<conversation-subject>`, so every delivery
//! for one conversation queues on one lane while deliveries for other conversations
//! proceed in parallel (subject to the server-wide inference limit and the agent's
//! `max_concurrency`). Cross-conversation parallelism is intentional; within-lane
//! ordering is a correctness property.

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn same_session_waits_for_the_active_permit() {
        let lanes = SessionLaneGuard::new();
        let first = lanes.acquire("session-a").await;
        let second = lanes.acquire("session-a");
        tokio::pin!(second);

        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut second)
                .await
                .is_err()
        );

        drop(first);
        tokio::time::timeout(Duration::from_secs(1), &mut second)
            .await
            .expect("the next same-session permit should become available");
    }

    #[tokio::test]
    async fn different_sessions_acquire_independent_permits() {
        let lanes = SessionLaneGuard::new();
        let _first = lanes.acquire("session-a").await;

        tokio::time::timeout(Duration::from_secs(1), lanes.acquire("session-b"))
            .await
            .expect("different sessions should not share a lane");
    }
}
