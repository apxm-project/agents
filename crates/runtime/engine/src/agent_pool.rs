//! Agent warm pool for reusing spawned agent sessions across SPAWN_AGENT operations.

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

struct PooledSession {
    session: Arc<Mutex<dyn std::any::Any + Send + Sync>>,
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
    pub async fn acquire(
        &self,
        profile: &str,
    ) -> Option<Arc<Mutex<dyn std::any::Any + Send + Sync>>> {
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

    /// Release a session back to the pool. Drops if at capacity.
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

        pool.push(PooledSession {
            session,
            last_used: Instant::now(),
        });

        tracing::debug!(
            profile = %profile,
            pool_size = pool.len(),
            "Released agent session to warm pool"
        );
    }

    /// Clean up expired sessions from all pools.
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
