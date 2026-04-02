use apxm_core::constants::defaults::DEFAULT_MAX_SESSIONS;
use dashmap::DashMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::AcpError;
use crate::registry::AgentProfile;
use crate::session::AcpSession;

/// Key for identifying a unique session in the pool.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SessionKey {
    agent: String,
    cwd: PathBuf,
    handle: String,
}

/// Named session pool for multi-turn ACP sessions.
///
/// Multiple INV(acp) nodes in a graph can share a session by using the
/// same `session_handle` attribute. The pool keeps sessions alive across
/// nodes and cleans them up when execution completes.
pub struct SessionPool {
    sessions: DashMap<SessionKey, Arc<Mutex<AcpSession>>>,
    max_sessions: usize,
}

impl SessionPool {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            max_sessions: DEFAULT_MAX_SESSIONS,
        }
    }

    /// Create a session pool with a custom session limit.
    pub fn with_max_sessions(max: usize) -> Self {
        Self {
            sessions: DashMap::new(),
            max_sessions: max,
        }
    }

    /// Get an existing session or create a new one.
    ///
    /// Uses DashMap's entry API to avoid TOCTOU race conditions where
    /// two concurrent callers could both see an absent key and spawn
    /// duplicate sessions.
    pub async fn get_or_create(
        &self,
        agent: &str,
        cwd: &Path,
        handle: &str,
        profile: &AgentProfile,
    ) -> Result<Arc<Mutex<AcpSession>>, AcpError> {
        let key = SessionKey {
            agent: agent.to_string(),
            cwd: cwd.to_path_buf(),
            handle: handle.to_string(),
        };

        // Fast path: session already exists
        if let Some(entry) = self.sessions.get(&key) {
            return Ok(Arc::clone(entry.value()));
        }

        // Check capacity before spawning a new session
        if self.sessions.len() >= self.max_sessions {
            return Err(AcpError::Spawn {
                agent: agent.to_string(),
                reason: format!(
                    "Session pool full: {} active sessions (max {})",
                    self.sessions.len(),
                    self.max_sessions
                ),
            });
        }

        // Slow path: spawn a new session outside the lock, then insert
        // atomically. If another thread raced us, we close our session
        // and return the winner.
        let session = AcpSession::spawn(agent, profile, cwd).await?;
        let session_arc = Arc::new(Mutex::new(session));

        use dashmap::mapref::entry::Entry;
        match self.sessions.entry(key) {
            Entry::Occupied(existing) => {
                // Another thread beat us — close the session we just spawned
                // and return the existing one.
                if let Ok(mutex) = Arc::try_unwrap(session_arc) {
                    mutex.into_inner().close().await;
                }
                Ok(Arc::clone(existing.get()))
            }
            Entry::Vacant(vacant) => {
                vacant.insert(Arc::clone(&session_arc));
                Ok(session_arc)
            }
        }
    }

    /// Close a specific session by key components.
    pub async fn close(&self, agent: &str, cwd: &Path, handle: &str) {
        let key = SessionKey {
            agent: agent.to_string(),
            cwd: cwd.to_path_buf(),
            handle: handle.to_string(),
        };
        if let Some((_, session_arc)) = self.sessions.remove(&key) {
            // Try to take the session out of the Arc — only possible if we hold
            // the last reference. Otherwise the close in Drop will handle it.
            if let Ok(session) = Arc::try_unwrap(session_arc) {
                session.into_inner().close().await;
            }
        }
    }

    /// Gracefully close all sessions in the pool.
    pub async fn close_all(&self) {
        let keys: Vec<SessionKey> = self.sessions.iter().map(|e| e.key().clone()).collect();
        for key in keys {
            if let Some((_, session_arc)) = self.sessions.remove(&key) {
                if let Ok(session) = Arc::try_unwrap(session_arc) {
                    session.into_inner().close().await;
                }
            }
        }
    }

    /// Number of active sessions in the pool.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

impl Default for SessionPool {
    fn default() -> Self {
        Self::new()
    }
}
