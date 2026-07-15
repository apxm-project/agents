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
use dashmap::mapref::entry::Entry;
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Session lane coordinator.
#[derive(Default)]
pub struct SessionLaneGuard {
    lanes: Arc<DashMap<String, Arc<Mutex<()>>>>,
}

impl SessionLaneGuard {
    /// Create an empty lane guard.
    pub fn new() -> Self {
        Self {
            lanes: Arc::new(DashMap::new()),
        }
    }

    /// Acquire the lane for a session.
    pub async fn acquire(&self, session_id: &str) -> SessionLanePermit {
        let lane = self
            .lanes
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let guard = Arc::clone(&lane).lock_owned().await;
        SessionLanePermit {
            guard: Some(guard),
            lanes: Arc::clone(&self.lanes),
            lane,
            session_id: session_id.to_string(),
        }
    }

    #[cfg(test)]
    fn lane_count(&self) -> usize {
        self.lanes.len()
    }
}

/// Held while a session lane lock is active.
pub struct SessionLanePermit {
    guard: Option<OwnedMutexGuard<()>>,
    lanes: Arc<DashMap<String, Arc<Mutex<()>>>>,
    lane: Arc<Mutex<()>>,
    session_id: String,
}

impl Drop for SessionLanePermit {
    fn drop(&mut self) {
        // Unlock before inspecting the map. A queued acquirer keeps its own
        // Arc, so the final strong-count check retains its existing lane and
        // preserves Tokio's FIFO mutex ordering.
        self.guard.take();

        if Arc::strong_count(&self.lane) != 2 {
            return;
        }

        if let Entry::Occupied(entry) = self.lanes.entry(self.session_id.clone())
            && Arc::ptr_eq(entry.get(), &self.lane)
            && Arc::strong_count(entry.get()) == 2
        {
            entry.remove();
        }
    }
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

    #[tokio::test]
    async fn releases_idle_lanes_after_high_cardinality_traffic() {
        let lanes = SessionLaneGuard::new();

        for session in 0..256 {
            let permit = lanes.acquire(&format!("session-{session}")).await;
            drop(permit);
        }

        assert_eq!(
            lanes.lane_count(),
            0,
            "idle session lanes must not grow with completed session traffic"
        );
    }

    #[tokio::test]
    async fn queued_same_session_work_keeps_its_lane_until_the_final_permit_drops() {
        let lanes = Arc::new(SessionLaneGuard::new());
        let first = lanes.acquire("session-a").await;
        let lane = lanes
            .lanes
            .get("session-a")
            .expect("active session must have a lane")
            .clone();
        let before_waiter = Arc::strong_count(&lane);
        let waiting_lanes = Arc::clone(&lanes);
        let waiting = tokio::spawn(async move { waiting_lanes.acquire("session-a").await });

        tokio::time::timeout(Duration::from_secs(1), async {
            while Arc::strong_count(&lane) == before_waiter {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("queued acquisition must retain the existing lane before release");
        assert_eq!(lanes.lane_count(), 1);
        drop(lane);
        drop(first);

        let second = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("queued acquisition must finish after the active permit drops")
            .expect("queued acquisition task must not panic");
        assert_eq!(lanes.lane_count(), 1, "the queued permit retains its lane");
        drop(second);
        assert_eq!(lanes.lane_count(), 0);
    }
}
