//! Agent warm pool for reusing spawned agent sessions across operations.
//!
//! The `AgentPool` maintains a pool of idle agent sessions that can be reused
//! instead of spawning new processes for every SPAWN_AGENT operation. This
//! significantly reduces the overhead of agent spawning, especially when
//! workflows frequently spawn and close agents.
//!
//! # Architecture
//!
//! - Sessions are keyed by agent profile name (e.g., "claude", "codex")
//! - Each profile has its own pool with a configurable maximum idle count
//! - Sessions are reclaimed after a configurable idle timeout
//! - The pool automatically closes sessions on shutdown
//!
//! # Usage
//!
//! ```rust,ignore
//! let pool = AgentPool::new(4, Duration::from_secs(300));
//!
//! // Try to acquire an existing session
//! if let Some(session) = pool.acquire("claude").await {
//!     // Reuse the session
//! } else {
//!     // No warm session available, spawn a new one
//! }
//!
//! // Release back to the pool when done
//! pool.release("claude", session);
//! ```

use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Agent warm pool for reusing spawned agent sessions.
pub struct AgentPool {
    /// Warm sessions keyed by agent profile name.
    pools: DashMap<String, Vec<PooledSession>>,
    /// Maximum idle sessions per profile.
    max_idle_per_profile: usize,
    /// How long to keep idle sessions before reclaiming.
    idle_timeout: Duration,
}

/// A pooled agent session with lifecycle metadata.
struct PooledSession {
    /// Type-erased session handle (concrete type: Arc<Mutex<AcpSession>>).
    session: Arc<Mutex<dyn std::any::Any + Send + Sync>>,
    /// When this session was originally created.
    created_at: Instant,
    /// When this session was last used.
    last_used: Instant,
}

impl AgentPool {
    /// Create a new agent pool with the given configuration.
    ///
    /// # Arguments
    ///
    /// * `max_idle` - Maximum number of idle sessions to keep per profile
    /// * `timeout` - Duration after which idle sessions are reclaimed
    pub fn new(max_idle: usize, timeout: Duration) -> Self {
        Self {
            pools: DashMap::new(),
            max_idle_per_profile: max_idle,
            idle_timeout: timeout,
        }
    }

    /// Try to acquire a warm session for the given profile.
    ///
    /// Returns `Some(session)` if a warm session is available, or `None` if
    /// the pool is empty and a new session must be spawned.
    pub async fn acquire(&self, profile: &str) -> Option<Arc<Mutex<dyn std::any::Any + Send + Sync>>> {
        let mut entry = self.pools.entry(profile.to_string()).or_default();
        let pool = entry.value_mut();

        // Try to find a non-expired session
        while let Some(pooled) = pool.pop() {
            let idle_duration = pooled.last_used.elapsed();
            if idle_duration < self.idle_timeout {
                tracing::debug!(
                    profile = %profile,
                    idle_ms = idle_duration.as_millis(),
                    "Acquired warm agent session from pool"
                );
                return Some(pooled.session);
            } else {
                // Session expired — close it (Drop will handle cleanup)
                tracing::debug!(
                    profile = %profile,
                    idle_ms = idle_duration.as_millis(),
                    "Discarding expired agent session"
                );
                // The Arc<Mutex<dyn Any>> is dropped here, which will eventually
                // trigger the Drop impl on AcpSession when all references are gone.
            }
        }

        None
    }

    /// Release a session back to the pool.
    ///
    /// If the pool is already at capacity for this profile, the session is
    /// dropped (closed) instead of being pooled.
    pub fn release(&self, profile: &str, session: Arc<Mutex<dyn std::any::Any + Send + Sync>>) {
        let mut entry = self.pools.entry(profile.to_string()).or_default();
        let pool = entry.value_mut();

        if pool.len() >= self.max_idle_per_profile {
            tracing::debug!(
                profile = %profile,
                pool_size = pool.len(),
                max_idle = self.max_idle_per_profile,
                "Pool at capacity, closing agent session instead of pooling"
            );
            // Drop the session (close it)
            return;
        }

        let now = Instant::now();
        pool.push(PooledSession {
            session,
            created_at: now,
            last_used: now,
        });

        tracing::debug!(
            profile = %profile,
            pool_size = pool.len(),
            "Released agent session to warm pool"
        );
    }

    /// Clean up expired sessions from all pools.
    ///
    /// This should be called periodically (e.g., every 60 seconds) to reclaim
    /// idle sessions that have exceeded the timeout.
    pub async fn cleanup_idle(&self) {
        let mut total_cleaned = 0;

        for mut entry in self.pools.iter_mut() {
            let profile = entry.key().clone();
            let pool = entry.value_mut();
            let before = pool.len();

            // Retain only non-expired sessions
            pool.retain(|pooled| {
                let idle_duration = pooled.last_used.elapsed();
                idle_duration < self.idle_timeout
            });

            let cleaned = before - pool.len();
            if cleaned > 0 {
                tracing::debug!(
                    profile = %profile,
                    cleaned = cleaned,
                    remaining = pool.len(),
                    "Cleaned up expired agent sessions"
                );
                total_cleaned += cleaned;
            }
        }

        if total_cleaned > 0 {
            tracing::info!(
                total_cleaned = total_cleaned,
                "Agent pool cleanup: closed {} expired session(s)",
                total_cleaned
            );
        }
    }

    /// Shut down the pool, closing all warm sessions.
    ///
    /// This should be called when the runtime is shutting down to ensure
    /// all pooled agent processes are properly terminated.
    pub async fn shutdown(&self) {
        let mut total_closed = 0;

        for entry in self.pools.iter() {
            let profile = entry.key();
            let pool = entry.value();
            let count = pool.len();
            total_closed += count;

            tracing::debug!(
                profile = %profile,
                sessions = count,
                "Closing pooled agent sessions for profile"
            );
        }

        // Clear all pools (dropping all Arc<Mutex<dyn Any>> references)
        self.pools.clear();

        if total_closed > 0 {
            tracing::info!(
                total_closed = total_closed,
                "Agent pool shutdown: closed {} warm session(s)",
                total_closed
            );
        }
    }

    /// Get pool statistics for observability.
    pub fn stats(&self) -> PoolStats {
        let mut total_sessions = 0;
        let mut profiles = Vec::new();

        for entry in self.pools.iter() {
            let count = entry.value().len();
            total_sessions += count;
            profiles.push(ProfileStats {
                profile: entry.key().clone(),
                idle_sessions: count,
            });
        }

        PoolStats {
            total_sessions,
            profiles,
        }
    }
}

/// Pool statistics for observability and debugging.
#[derive(Debug, Clone)]
pub struct PoolStats {
    /// Total number of idle sessions across all profiles.
    pub total_sessions: usize,
    /// Per-profile statistics.
    pub profiles: Vec<ProfileStats>,
}

/// Per-profile pool statistics.
#[derive(Debug, Clone)]
pub struct ProfileStats {
    /// Agent profile name.
    pub profile: String,
    /// Number of idle sessions for this profile.
    pub idle_sessions: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pool_acquire_empty() {
        let pool = AgentPool::new(4, Duration::from_secs(300));
        assert!(pool.acquire("claude").await.is_none());
    }

    #[tokio::test]
    async fn test_pool_acquire_release() {
        let pool = AgentPool::new(4, Duration::from_secs(300));
        let session = Arc::new(Mutex::new(42u32));

        // Release a session
        pool.release("claude", session.clone() as Arc<Mutex<dyn std::any::Any + Send + Sync>>);

        // Acquire it back
        let acquired = pool.acquire("claude").await;
        assert!(acquired.is_some());

        // Pool should now be empty
        assert!(pool.acquire("claude").await.is_none());
    }

    #[tokio::test]
    async fn test_pool_max_capacity() {
        let pool = AgentPool::new(2, Duration::from_secs(300));

        // Release 3 sessions
        for i in 0..3 {
            let session = Arc::new(Mutex::new(i));
            pool.release("claude", session as Arc<Mutex<dyn std::any::Any + Send + Sync>>);
        }

        // Only 2 should be pooled (max_idle_per_profile = 2)
        let stats = pool.stats();
        assert_eq!(stats.total_sessions, 2);
    }

    #[tokio::test]
    async fn test_pool_expiration() {
        let pool = AgentPool::new(4, Duration::from_millis(50));
        let session = Arc::new(Mutex::new(42u32));

        pool.release("claude", session.clone() as Arc<Mutex<dyn std::any::Any + Send + Sync>>);

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Acquire should return None (session expired)
        assert!(pool.acquire("claude").await.is_none());
    }

    #[tokio::test]
    async fn test_pool_cleanup() {
        let pool = AgentPool::new(4, Duration::from_millis(50));

        // Add 3 sessions
        for i in 0..3 {
            let session = Arc::new(Mutex::new(i));
            pool.release("claude", session as Arc<Mutex<dyn std::any::Any + Send + Sync>>);
        }

        assert_eq!(pool.stats().total_sessions, 3);

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Cleanup should remove all expired sessions
        pool.cleanup_idle().await;
        assert_eq!(pool.stats().total_sessions, 0);
    }

    #[tokio::test]
    async fn test_pool_shutdown() {
        let pool = AgentPool::new(4, Duration::from_secs(300));

        // Add sessions for multiple profiles
        for i in 0..2 {
            let session = Arc::new(Mutex::new(i));
            pool.release("claude", session as Arc<Mutex<dyn std::any::Any + Send + Sync>>);
        }
        for i in 0..3 {
            let session = Arc::new(Mutex::new(i));
            pool.release("codex", session as Arc<Mutex<dyn std::any::Any + Send + Sync>>);
        }

        assert_eq!(pool.stats().total_sessions, 5);

        // Shutdown should close all sessions
        pool.shutdown().await;
        assert_eq!(pool.stats().total_sessions, 0);
    }

    #[tokio::test]
    async fn test_pool_stats() {
        let pool = AgentPool::new(4, Duration::from_secs(300));

        let session1 = Arc::new(Mutex::new(1));
        pool.release("claude", session1 as Arc<Mutex<dyn std::any::Any + Send + Sync>>);

        let session2 = Arc::new(Mutex::new(2));
        pool.release("codex", session2 as Arc<Mutex<dyn std::any::Any + Send + Sync>>);

        let stats = pool.stats();
        assert_eq!(stats.total_sessions, 2);
        assert_eq!(stats.profiles.len(), 2);
    }
}
