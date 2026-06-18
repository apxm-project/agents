//! In-memory storage backend.

use crate::storage::{SearchResult, StorageBackend, StorageResult, backend::BackendStats};
use apxm_core::{error::RuntimeError, types::values::Value};
use async_trait::async_trait;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

const MEM_SPACE: &str = "in-memory";

#[inline]
fn mem_err(message: impl Into<String>) -> RuntimeError {
    RuntimeError::Memory {
        message: message.into(),
        space: Some(MEM_SPACE.to_string()),
    }
}

/// In-memory storage backend.
///
/// Time:
/// - put/get/delete/exists: average O(1)
/// - list_keys/search/stats: O(n)
///
/// Space: O(n)
#[derive(Clone, Debug)]
pub struct InMemoryBackend {
    data: Arc<RwLock<HashMap<String, Value>>>,
    max_capacity: Option<usize>,
}

impl InMemoryBackend {
    pub fn new(max_capacity: Option<usize>) -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
            max_capacity: max_capacity.filter(|&c| c > 0),
        }
    }

    pub fn unlimited() -> Self {
        Self::new(None)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self::new(Some(capacity))
    }

    pub async fn len(&self) -> usize {
        self.data.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.data.read().await.is_empty()
    }

    #[inline]
    fn enforce_capacity(&self, len: usize, inserting_new_key: bool) -> StorageResult<()> {
        match (self.max_capacity, inserting_new_key) {
            (Some(max), true) if len >= max => Err(mem_err(format!(
                "capacity exceeded: {len} items, max {max}"
            ))),
            _ => Ok(()),
        }
    }
}

#[async_trait]
impl StorageBackend for InMemoryBackend {
    async fn put(&self, key: &str, value: Value) -> StorageResult<()> {
        if key.is_empty() {
            return Err(mem_err("key must not be empty"));
        }

        let mut data = self.data.write().await;
        let inserting_new_key = !data.contains_key(key);

        self.enforce_capacity(data.len(), inserting_new_key)?;
        data.insert(key.to_owned(), value);

        Ok(())
    }

    async fn get(&self, key: &str) -> StorageResult<Option<Value>> {
        Ok(self.data.read().await.get(key).cloned())
    }

    async fn delete(&self, key: &str) -> StorageResult<()> {
        self.data.write().await.remove(key);
        Ok(())
    }

    async fn exists(&self, key: &str) -> StorageResult<bool> {
        Ok(self.data.read().await.contains_key(key))
    }

    async fn list_keys(&self) -> StorageResult<Vec<String>> {
        let data = self.data.read().await;
        let mut keys: Vec<String> = data.keys().cloned().collect();
        keys.sort_unstable(); // deterministic and fast
        Ok(keys)
    }

    async fn search(&self, query: &str, limit: usize) -> StorageResult<Vec<SearchResult>> {
        if limit == 0 || query.is_empty() {
            return Ok(Vec::new());
        }

        let data = self.data.read().await;

        // Deterministic ordering to avoid flakiness and surprising results.
        // Costs O(n log n); acceptable for in-memory scans, and keeps behavior stable.
        let mut keys: Vec<&String> = data.keys().collect();
        keys.sort_unstable();

        let mut results = Vec::with_capacity(limit);
        for key in keys.into_iter().filter(|k| k.contains(query)) {
            if let Some(value) = data.get(key) {
                results.push(SearchResult {
                    key: key.clone(),
                    value: value.clone(),
                    score: 1.0,
                });
                if results.len() == limit {
                    break;
                }
            }
        }

        Ok(results)
    }

    async fn clear(&self) -> StorageResult<()> {
        self.data.write().await.clear();
        Ok(())
    }

    async fn stats(&self) -> StorageResult<BackendStats> {
        let data = self.data.read().await;

        let total_keys = data.len();
        let total_size_bytes = data.iter().fold(0usize, |acc, (k, v)| {
            acc.saturating_add(k.len())
                .saturating_add(estimate_value_size(v))
        });

        Ok(BackendStats {
            total_keys,
            total_size_bytes,
            backend_type: MEM_SPACE.to_string(),
        })
    }
}

/// Rough estimation of value size in bytes (best-effort; not allocator-accurate).
fn estimate_value_size(value: &Value) -> usize {
    match value {
        Value::Null => 1,
        Value::Bool(_) => 1,
        Value::Number(_) => 8,
        Value::String(s) => s.len(),
        Value::Array(arr) => arr
            .iter()
            .fold(0usize, |acc, v| acc.saturating_add(estimate_value_size(v))),
        Value::Object(obj) => obj.iter().fold(0usize, |acc, (k, v)| {
            acc.saturating_add(k.len())
                .saturating_add(estimate_value_size(v))
        }),
        Value::Token(_) => 8,
    }
}
