//! Graceful shutdown coordination: track in-flight HTTP work and signal drain.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use tokio::sync::Notify;
use tracing::warn;

/// Tracks in-flight HTTP handlers and exposes a drain barrier.
#[derive(Clone, Default)]
pub(crate) struct ShutdownCoordinator {
    in_flight: Arc<AtomicUsize>,
    draining: Arc<Notify>,
}

impl ShutdownCoordinator {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn track_request(&self) -> RequestGuard {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        RequestGuard {
            in_flight: Arc::clone(&self.in_flight),
        }
    }

    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::SeqCst)
    }

    /// Wait until all tracked HTTP handlers complete or timeout elapses.
    pub(crate) async fn wait_for_http_drain(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.in_flight() == 0 {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                warn!(
                    in_flight = self.in_flight(),
                    "HTTP drain deadline elapsed before all requests completed"
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub(crate) fn signal_drain(&self) {
        self.draining.notify_waiters();
    }
}

pub(crate) struct RequestGuard {
    in_flight: Arc<AtomicUsize>,
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Count in-flight HTTP requests for graceful drain metrics.
pub(crate) async fn track_in_flight(
    axum::extract::State(coordinator): axum::extract::State<ShutdownCoordinator>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let _guard = coordinator.track_request();
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn drain_waits_for_in_flight_to_clear() {
        let coord = ShutdownCoordinator::new();
        let _guard = coord.track_request();
        let coord_clone = coord.clone();
        let handle = tokio::spawn(async move {
            coord_clone
                .wait_for_http_drain(Duration::from_secs(2))
                .await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(coord.in_flight(), 1);
        drop(_guard);
        handle.await.expect("drain task");
        assert_eq!(coord.in_flight(), 0);
    }
}
