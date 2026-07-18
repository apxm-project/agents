//! Long-Term Memory (LTM) - Persistent semantic storage
//!
//! Uses persistent backends (SQLite, Redb) for durable storage.
//! Supports semantic search and long-term knowledge retention.

use super::config::{LtmBackend, LtmConfig};
use apxm_backends::{InMemoryBackend, RedbBackend, SearchResult, SqliteBackend, StorageBackend};
use apxm_core::{constants::memory as mem_const, error::RuntimeError, types::values::Value};
use std::sync::Arc;

type Result<T> = std::result::Result<T, RuntimeError>;

/// Long-Term Memory layer with pluggable backend
pub struct LongTermMemory {
    backend: Arc<dyn StorageBackend + Send + Sync>,
}

impl LongTermMemory {
    /// Create a new LTM instance with the given configuration
    pub async fn new(config: LtmConfig) -> Result<Self> {
        let backend: Arc<dyn StorageBackend + Send + Sync> = match config.backend {
            LtmBackend::Memory => Arc::new(InMemoryBackend::unlimited()),
            LtmBackend::Sqlite => {
                let path = config.path.ok_or_else(|| RuntimeError::Memory {
                    message: "SQLite backend requires a path".to_string(),
                    space: Some(mem_const::LTM.to_string()),
                })?;
                Arc::new(SqliteBackend::new(path, config.max_connections).await?)
            }
            LtmBackend::Redb => {
                let path = config.path.ok_or_else(|| RuntimeError::Memory {
                    message: "Redb backend requires a path".to_string(),
                    space: Some(mem_const::LTM.to_string()),
                })?;
                Arc::new(RedbBackend::new(path).await?)
            }
        };

        Ok(Self { backend })
    }

    /// Create in-memory LTM (for testing)
    pub fn in_memory() -> impl std::future::Future<Output = Result<Self>> {
        std::future::ready(Ok(Self {
            backend: Arc::new(InMemoryBackend::unlimited()),
        }))
    }

    /// Store a value in LTM
    pub async fn put(&self, key: &str, value: Value) -> Result<()> {
        self.backend.put(key, value).await
    }

    /// Retrieve a value from LTM
    pub async fn get(&self, key: &str) -> Result<Option<Value>> {
        self.backend.get(key).await
    }

    /// Delete a value from LTM
    pub async fn delete(&self, key: &str) -> Result<()> {
        self.backend.delete(key).await
    }

    /// Check if a key exists in LTM
    pub async fn exists(&self, key: &str) -> Result<bool> {
        self.backend.exists(key).await
    }

    /// List all keys in LTM
    pub async fn list_keys(&self) -> Result<Vec<String>> {
        self.backend.list_keys().await
    }

    /// Search LTM by key substring
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        self.backend.search(query, limit).await
    }

    /// Semantic/vector search (currently falls back to substring search)
    pub async fn search_semantic(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        self.backend.search_vector(query, limit).await
    }

    /// Clear all entries from LTM
    pub async fn clear(&self) -> Result<()> {
        self.backend.clear().await
    }

    /// Get LTM statistics
    pub async fn stats(&self) -> Result<apxm_backends::BackendStats> {
        self.backend.stats().await
    }
}
