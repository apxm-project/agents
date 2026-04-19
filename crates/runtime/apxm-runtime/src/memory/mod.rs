//! Three-tier memory system for APxM runtime
//!
//! Provides:
//! - **STM** (Short-Term Memory): Fast, volatile working memory
//! - **LTM** (Long-Term Memory): Persistent semantic storage
//! - **Episodic**: Append-only execution trace

mod config;
mod episodic;
mod facts;
mod ltm;
mod stm;

pub use config::MemoryConfig;
pub use episodic::{EpisodicEntry, EpisodicMemory};
pub use facts::{Fact, FactFilter, FactResult};
pub use ltm::LongTermMemory;
pub use stm::ShortTermMemory;

use apxm_core::constants::memory as mem_const;
use apxm_core::error::RuntimeError;
use std::path::PathBuf;
use std::sync::Arc;

type Result<T> = std::result::Result<T, RuntimeError>;
const SCOPE_KEY_PREFIX: &str = "__scope__/";

/// Memory space identifier for routing operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySpace {
    /// Short-term, volatile memory (fast cache)
    Stm,
    /// Long-term, persistent memory (durable storage)
    Ltm,
    /// Episodic trace memory (execution history)
    Episodic,
}

impl std::str::FromStr for MemorySpace {
    type Err = RuntimeError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            mem_const::STM => Ok(MemorySpace::Stm),
            mem_const::LTM => Ok(MemorySpace::Ltm),
            mem_const::EPISODIC => Ok(MemorySpace::Episodic),
            other => Err(RuntimeError::Memory {
                message: format!("Unknown memory tier: {}", other),
                space: Some(other.to_string()),
            }),
        }
    }
}

/// Unified memory system coordinating all three tiers
#[derive(Clone)]
pub struct MemorySystem {
    stm: Arc<ShortTermMemory>,
    ltm: Arc<LongTermMemory>,
    episodic: Arc<EpisodicMemory>,
}

impl MemorySystem {
    /// Create a new memory system with the given configuration
    pub async fn new(config: MemoryConfig) -> Result<Self> {
        let stm = Arc::new(ShortTermMemory::new(config.stm_config)?);
        let ltm = Arc::new(LongTermMemory::new(config.ltm_config).await?);
        let episodic = Arc::new(EpisodicMemory::new(config.episodic_config));

        Ok(Self { stm, ltm, episodic })
    }

    /// Read a value from the specified memory space
    pub async fn read(
        &self,
        space: MemorySpace,
        key: &str,
    ) -> Result<Option<apxm_core::types::values::Value>> {
        match space {
            MemorySpace::Stm => self.stm.get(key).await,
            MemorySpace::Ltm => self.ltm.get(key).await,
            MemorySpace::Episodic => Err(RuntimeError::Memory {
                message: "Episodic memory is append-only, use query instead".to_string(),
                space: Some(mem_const::EPISODIC.to_string()),
            }),
        }
    }

    /// Return the storage prefix used for a hierarchical execution scope.
    pub fn scope_prefix(scope_id: &str) -> String {
        format!("{SCOPE_KEY_PREFIX}{scope_id}/")
    }

    /// Build the storage key for a value scoped to a hierarchical execution context.
    pub fn scoped_key(scope_id: &str, key: &str) -> String {
        format!("{}{}", Self::scope_prefix(scope_id), key)
    }

    /// Remove the storage prefix from a scoped key and return the logical key.
    pub fn strip_scope_prefix(scope_id: &str, key: &str) -> Option<String> {
        key.strip_prefix(&Self::scope_prefix(scope_id))
            .map(str::to_string)
    }

    /// Read a value from STM/LTM within a specific execution scope.
    pub async fn read_scoped(
        &self,
        space: MemorySpace,
        scope_id: &str,
        key: &str,
    ) -> Result<Option<apxm_core::types::values::Value>> {
        let scoped_key = Self::scoped_key(scope_id, key);
        match space {
            MemorySpace::Stm => self.stm.get(&scoped_key).await,
            MemorySpace::Ltm => self.ltm.get(&scoped_key).await,
            MemorySpace::Episodic => Err(RuntimeError::Memory {
                message: "Episodic memory is append-only, use query instead".to_string(),
                space: Some(mem_const::EPISODIC.to_string()),
            }),
        }
    }

    /// Write a value to the specified memory space
    pub async fn write(
        &self,
        space: MemorySpace,
        key: String,
        value: apxm_core::types::values::Value,
    ) -> Result<()> {
        match space {
            MemorySpace::Stm => self.stm.put(&key, value).await,
            MemorySpace::Ltm => self.ltm.put(&key, value).await,
            MemorySpace::Episodic => Err(RuntimeError::Memory {
                message: "Episodic memory is append-only, use record instead".to_string(),
                space: Some(mem_const::EPISODIC.to_string()),
            }),
        }
    }

    /// Write a value to STM/LTM within a specific execution scope.
    pub async fn write_scoped(
        &self,
        space: MemorySpace,
        scope_id: &str,
        key: String,
        value: apxm_core::types::values::Value,
    ) -> Result<()> {
        self.write(space, Self::scoped_key(scope_id, &key), value)
            .await
    }

    /// Delete a key from the specified memory space
    pub async fn delete(&self, space: MemorySpace, key: &str) -> Result<()> {
        match space {
            MemorySpace::Stm => self.stm.delete(key).await,
            MemorySpace::Ltm => self.ltm.delete(key).await,
            MemorySpace::Episodic => Err(RuntimeError::Memory {
                message: "Episodic memory is append-only, cannot delete".to_string(),
                space: Some(mem_const::EPISODIC.to_string()),
            }),
        }
    }

    /// Delete a key from STM/LTM within a specific execution scope.
    pub async fn delete_scoped(&self, space: MemorySpace, scope_id: &str, key: &str) -> Result<()> {
        self.delete(space, &Self::scoped_key(scope_id, key)).await
    }

    /// Search memory space (substring matching on keys)
    pub async fn search(
        &self,
        space: MemorySpace,
        query: &str,
        limit: usize,
    ) -> Result<Vec<apxm_backends::SearchResult>> {
        match space {
            MemorySpace::Stm => self.stm.search(query, limit).await,
            MemorySpace::Ltm => self.ltm.search(query, limit).await,
            MemorySpace::Episodic => self.episodic.search(query, limit).await,
        }
    }

    /// Search memory within a specific execution scope.
    ///
    /// STM/LTM keys are physically namespaced by `scope_id` and returned with
    /// the scope prefix stripped so callers continue to work with logical keys.
    /// Episodic memory remains global for now.
    pub async fn search_scoped(
        &self,
        space: MemorySpace,
        scope_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<apxm_backends::SearchResult>> {
        match space {
            MemorySpace::Stm | MemorySpace::Ltm => {
                let scoped_query = Self::scoped_key(scope_id, query);
                let results = self.search(space, &scoped_query, limit).await?;
                Ok(results
                    .into_iter()
                    .filter_map(|result| {
                        Self::strip_scope_prefix(scope_id, &result.key).map(|logical_key| {
                            apxm_backends::SearchResult {
                                key: logical_key,
                                value: result.value,
                                score: result.score,
                            }
                        })
                    })
                    .collect())
            }
            MemorySpace::Episodic => self.search(space, query, limit).await,
        }
    }

    /// Record an entry in episodic memory
    pub async fn record_episode(
        &self,
        event_type: String,
        payload: apxm_core::types::values::Value,
        execution_id: String,
        node_id: Option<u64>,
        session_dir: Option<PathBuf>,
    ) -> Result<String> {
        self.episodic
            .record(event_type, payload, execution_id, node_id, session_dir)
            .await
    }

    /// Record an entry with explicit event type and payload map
    pub async fn record_episodic_event(
        &self,
        execution_id: String,
        event_type: &str,
        payload: apxm_core::types::values::Value,
        node_id: Option<u64>,
        session_dir: Option<PathBuf>,
    ) -> Result<String> {
        self.record_episode(
            event_type.to_string(),
            payload,
            execution_id,
            node_id,
            session_dir,
        )
        .await
    }

    /// Query episodic memory by execution ID
    pub async fn query_episodes(&self, execution_id: &str) -> Result<Vec<EpisodicEntry>> {
        self.episodic.get_by_execution(execution_id).await
    }

    /// Get STM reference (for advanced use cases)
    pub fn stm(&self) -> &ShortTermMemory {
        &self.stm
    }

    /// Get LTM reference (for advanced use cases)
    pub fn ltm(&self) -> &LongTermMemory {
        &self.ltm
    }

    /// Get Episodic reference (for advanced use cases)
    pub fn episodic(&self) -> &EpisodicMemory {
        &self.episodic
    }

    /// Clear all memory tiers (useful for testing)
    pub async fn clear_all(&self) -> Result<()> {
        self.stm.clear().await?;
        self.ltm.clear().await?;
        self.episodic.clear().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::values::Value;

    #[tokio::test]
    async fn test_memory_system_stm() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let config = MemoryConfig::in_memory_ltm();
        let system = MemorySystem::new(config).await?;

        // Write to STM
        system
            .write(
                MemorySpace::Stm,
                "test_key".to_string(),
                Value::String("test_value".to_string()),
            )
            .await?;

        // Read from STM
        let result = system.read(MemorySpace::Stm, "test_key").await?;
        assert_eq!(result, Some(Value::String("test_value".to_string())));

        // Delete from STM
        system.delete(MemorySpace::Stm, "test_key").await?;
        let result = system.read(MemorySpace::Stm, "test_key").await?;
        assert_eq!(result, None);

        Ok(())
    }

    #[tokio::test]
    async fn test_memory_system_ltm() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let config = MemoryConfig::in_memory_ltm();
        let system = MemorySystem::new(config).await?;

        // Write to LTM
        system
            .write(
                MemorySpace::Ltm,
                "persistent_key".to_string(),
                Value::String("persistent_value".to_string()),
            )
            .await?;

        // Read from LTM
        let result = system.read(MemorySpace::Ltm, "persistent_key").await?;
        assert_eq!(result, Some(Value::String("persistent_value".to_string())));

        Ok(())
    }

    #[tokio::test]
    async fn test_memory_system_episodic() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let config = MemoryConfig::in_memory_ltm();
        let system = MemorySystem::new(config).await?;

        // Record episode
        let entry_id = system
            .record_episode(
                "test_event".to_string(),
                Value::String("event_data".to_string()),
                "exec_123".to_string(),
                None,
                None,
            )
            .await?;

        assert!(!entry_id.is_empty());

        // Query episodes
        let episodes = system.query_episodes("exec_123").await?;
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].event_type, "test_event");

        Ok(())
    }

    #[tokio::test]
    async fn test_memory_system_scoped_stm_isolation()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let config = MemoryConfig::in_memory_ltm();
        let system = MemorySystem::new(config).await?;

        system
            .write_scoped(
                MemorySpace::Stm,
                "scope-a",
                "shared_key".to_string(),
                Value::String("alpha".to_string()),
            )
            .await?;
        system
            .write_scoped(
                MemorySpace::Stm,
                "scope-b",
                "shared_key".to_string(),
                Value::String("beta".to_string()),
            )
            .await?;

        let scope_a = system
            .read_scoped(MemorySpace::Stm, "scope-a", "shared_key")
            .await?;
        let scope_b = system
            .read_scoped(MemorySpace::Stm, "scope-b", "shared_key")
            .await?;

        assert_eq!(scope_a, Some(Value::String("alpha".to_string())));
        assert_eq!(scope_b, Some(Value::String("beta".to_string())));
        assert_eq!(system.read(MemorySpace::Stm, "shared_key").await?, None);

        Ok(())
    }

    #[tokio::test]
    async fn test_memory_system_scoped_search_strips_prefix()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let config = MemoryConfig::in_memory_ltm();
        let system = MemorySystem::new(config).await?;

        system
            .write_scoped(
                MemorySpace::Ltm,
                "scope-a",
                "user:1".to_string(),
                Value::String("alice".to_string()),
            )
            .await?;
        system
            .write_scoped(
                MemorySpace::Ltm,
                "scope-b",
                "user:2".to_string(),
                Value::String("bob".to_string()),
            )
            .await?;

        let results = system
            .search_scoped(MemorySpace::Ltm, "scope-a", "user", 10)
            .await?;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].key, "user:1");
        assert_eq!(results[0].value, Value::String("alice".to_string()));

        Ok(())
    }
}
