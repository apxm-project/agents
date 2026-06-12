//! Short-Term Memory (STM) - Fast, volatile working memory
//!
//! Uses in-memory backend for microsecond-latency operations.
//! Ideal for temporary results, intermediate values, and caching.

use super::config::StmConfig;
use apxm_backends::{InMemoryBackend, SearchResult, StorageBackend};
use apxm_core::{error::RuntimeError, types::values::Value};

type Result<T> = std::result::Result<T, RuntimeError>;

/// Short-Term Memory layer using in-memory backend
pub struct ShortTermMemory {
    backend: InMemoryBackend,
}

impl ShortTermMemory {
    /// Create a new STM instance with the given configuration
    pub fn new(config: StmConfig) -> Result<Self> {
        let backend = InMemoryBackend::new(config.max_entries);
        Ok(Self { backend })
    }

    /// Create unlimited-capacity STM (for testing)
    pub fn unlimited() -> Self {
        Self {
            backend: InMemoryBackend::unlimited(),
        }
    }

    /// Store a value in STM
    pub async fn put(&self, key: &str, value: Value) -> Result<()> {
        self.backend.put(key, value).await
    }

    /// Retrieve a value from STM
    pub async fn get(&self, key: &str) -> Result<Option<Value>> {
        self.backend.get(key).await
    }

    /// Delete a value from STM
    pub async fn delete(&self, key: &str) -> Result<()> {
        self.backend.delete(key).await
    }

    /// Check if a key exists in STM
    pub async fn exists(&self, key: &str) -> Result<bool> {
        self.backend.exists(key).await
    }

    /// List all keys in STM
    pub async fn list_keys(&self) -> Result<Vec<String>> {
        self.backend.list_keys().await
    }

    /// Search STM by key substring
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        self.backend.search(query, limit).await
    }

    /// Clear all entries from STM
    pub async fn clear(&self) -> Result<()> {
        self.backend.clear().await
    }

    /// Get current number of entries
    pub async fn len(&self) -> usize {
        self.backend.len().await
    }

    /// Check if STM is empty
    pub async fn is_empty(&self) -> bool {
        self.backend.is_empty().await
    }

    /// Get STM statistics
    pub async fn stats(&self) -> Result<apxm_backends::BackendStats> {
        self.backend.stats().await
    }
}

