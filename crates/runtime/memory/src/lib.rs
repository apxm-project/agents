//! Three-tier memory system for APxM runtime
//!
//! Provides:
//! - **STM** (Short-Term Memory): Fast, volatile working memory
//! - **LTM** (Long-Term Memory): Persistent semantic storage
//! - **Episodic**: Append-only execution trace

mod config;
mod episodic;
mod ltm;
mod stm;

pub use config::MemoryConfig;
pub use episodic::{EpisodicEntry, EpisodicMemory};
pub use ltm::LongTermMemory;
pub use stm::ShortTermMemory;

use apxm_core::constants::memory as mem_const;
use apxm_core::error::RuntimeError;
use apxm_core::types::MemoryTier;
use std::path::PathBuf;
use std::sync::Arc;

type Result<T> = std::result::Result<T, RuntimeError>;
const SCOPE_KEY_PREFIX: &str = "__scope__/";

/// Memory space identifier — alias of the contract [`MemoryTier`].
pub type MemorySpace = MemoryTier;

/// Parse a memory tier string into [`MemorySpace`], mapping parse errors to runtime errors.
pub fn parse_memory_space(s: &str) -> Result<MemorySpace> {
    s.parse::<MemoryTier>().map_err(|e| RuntimeError::Memory {
        message: e.to_string(),
        space: Some(s.to_string()),
    })
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
    /// Episodic memory is global.
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

    /// Recency window: return the last `n` scoped entries whose logical key
    /// starts with `key_prefix` and ends with a numeric suffix, in TRUE temporal
    /// order. Backs `qmem(recall_mode="recent", recent=N)` — the
    /// transcript-as-memory window.
    ///
    /// The conversation records the user message (`conversation:user:<n>`) and
    /// the assistant reply (`conversation:turn:<n>`) under INDEPENDENT counters,
    /// so both share the turn index `n`. Sorting by `n` alone leaves their
    /// intra-turn order to `list_keys()` insertion order (which could surface the
    /// reply before its prompting message). We therefore sort by
    /// `(turn_index, role_rank)` with `user` before the assistant `turn`, so the
    /// recalled transcript reads in conversational order (M2/M3).
    /// Recency window over `key_prefix`, plus any `pins` (exact logical keys)
    /// surfaced ahead of it. Pinning is a generic mechanism — WHICH key to pin
    /// (e.g. a compaction summary) is a caller/program decision, not a policy
    /// baked here. A pinned key with no numeric transcript sort order would
    /// otherwise be dropped from the recency window.
    pub async fn recent_scoped(
        &self,
        space: MemorySpace,
        scope_id: &str,
        key_prefix: &str,
        n: usize,
        pins: &[String],
    ) -> Result<Vec<apxm_backends::SearchResult>> {
        let all_keys = match space {
            MemorySpace::Stm => self.stm.list_keys().await?,
            MemorySpace::Ltm => self.ltm.list_keys().await?,
            MemorySpace::Episodic => return Ok(Vec::new()),
        };
        let scoped_prefix = Self::scoped_key(scope_id, key_prefix);
        let mut ordered: Vec<((i64, u8), String)> = all_keys
            .into_iter()
            .filter(|k| k.starts_with(&scoped_prefix))
            .filter_map(|k| {
                let logical = Self::strip_scope_prefix(scope_id, &k)?;
                let sort_key = Self::transcript_sort_key(&logical)?;
                Some((sort_key, logical))
            })
            .collect();
        ordered.sort_by_key(|(sort_key, _)| *sort_key);
        let start = ordered.len().saturating_sub(n);
        let mut out = Vec::new();
        // Surface any caller-pinned keys ahead of the recency window. These have
        // no numeric transcript sort order, so they would otherwise be dropped;
        // pinning lets compacted older history (a program's rolling summary)
        // survive once it slides out of the last-`n` window. The program decides
        // which keys to pin (constitution #2), not this layer.
        for pin in pins {
            if let Some(value) = self.read_scoped(space, scope_id, pin).await? {
                out.push(apxm_backends::SearchResult {
                    key: pin.clone(),
                    value,
                    score: 1.0,
                });
            }
        }
        for (_, logical) in &ordered[start..] {
            if let Some(value) = self.read_scoped(space, scope_id, logical).await? {
                out.push(apxm_backends::SearchResult {
                    key: logical.clone(),
                    value,
                    score: 1.0,
                });
            }
        }
        Ok(out)
    }

    /// Temporal sort key for a transcript entry: `(turn_index, role_rank)` where
    /// the trailing `:<n>` is the turn index and the segment before it is the
    /// role — `user` (0) sorts before the assistant `turn` (1) within a turn so
    /// the recalled window reads user-then-assistant. Returns `None` when the key
    /// has no numeric suffix (e.g. the `..._count` counter keys), excluding it.
    fn transcript_sort_key(logical: &str) -> Option<(i64, u8)> {
        let mut segments = logical.rsplit(':');
        let idx = segments.next()?.parse::<i64>().ok()?;
        let role_rank = match segments.next() {
            Some("user") => 0,
            Some("turn") => 1,
            _ => 2,
        };
        Some((idx, role_rank))
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
mod recent_window_tests {
    use super::{MemoryConfig, MemorySpace, MemorySystem};
    use apxm_core::types::values::Value;

    /// A folded compaction summary (`<prefix>summary`) must be recalled ahead of
    /// the recent window even when there are far more turns than `n` — this is
    /// what lets a turn-1 fact survive once it slides out of the last-`n` turns.
    #[tokio::test]
    async fn recent_scoped_surfaces_compaction_summary_outside_window() {
        let mem = MemorySystem::new(MemoryConfig::in_memory_ltm())
            .await
            .expect("memory");
        let scope = "sess-1";

        // 20 assistant turns; the early ones are well outside a 4-turn window.
        for i in 1..=20 {
            mem.write_scoped(
                MemorySpace::Stm,
                scope,
                format!("conversation:turn:{i}"),
                Value::String(format!("answer {i}")),
            )
            .await
            .unwrap();
        }
        // A folded summary capturing the turn-1 fact.
        mem.write_scoped(
            MemorySpace::Stm,
            scope,
            "conversation:summary".to_string(),
            Value::String("user's name is Ada (from turn 1)".to_string()),
        )
        .await
        .unwrap();

        let out = mem
            .recent_scoped(
                MemorySpace::Stm,
                scope,
                "conversation:",
                4,
                &["conversation:summary".to_string()],
            )
            .await
            .unwrap();

        // The pinned summary is present and first, even though turn 1 is far
        // outside the 4-turn recency window.
        assert_eq!(
            out.first().map(|r| r.key.as_str()),
            Some("conversation:summary")
        );
        assert!(
            out.iter()
                .any(|r| r.value.as_string().is_some_and(|s| s.contains("Ada"))),
            "folded turn-1 fact must be recalled"
        );
        // Plus the last 4 turns (summary + 4 = 5 results), not turn 1 directly.
        assert_eq!(out.len(), 5);
        assert!(out.iter().all(|r| r.key != "conversation:turn:1"));
    }

    /// Audit minor 2: the recalled transcript window must read in TRUE
    /// conversational order — user message before its assistant reply within a
    /// turn, and turns in ascending order — regardless of `list_keys()` insertion
    /// order. The sort key `(turn_index, role_rank)` guarantees this.
    #[test]
    fn transcript_sort_key_orders_user_before_assistant_then_by_turn() {
        // Deliberately scrambled (assistant recorded before user; turn 2 first).
        let mut keys = vec![
            "conversation:turn:2".to_string(),
            "conversation:turn:1".to_string(),
            "conversation:user:2".to_string(),
            "conversation:user:1".to_string(),
        ];
        keys.sort_by_key(|k| MemorySystem::transcript_sort_key(k).unwrap());
        assert_eq!(
            keys,
            vec![
                "conversation:user:1".to_string(), // turn 1: user before assistant
                "conversation:turn:1".to_string(),
                "conversation:user:2".to_string(), // turn 2
                "conversation:turn:2".to_string(),
            ],
            "transcript must read user-then-assistant within a turn, turns ascending"
        );
    }

    #[test]
    fn transcript_sort_key_excludes_non_numeric_counter_keys() {
        // The `*_count` counter keys have no numeric suffix → excluded.
        assert!(MemorySystem::transcript_sort_key("conversation:user_count").is_none());
        assert!(MemorySystem::transcript_sort_key("conversation:turn_count").is_none());
        // A plain `<prefix><n>` series still orders by index (role_rank falls to 2).
        assert_eq!(
            MemorySystem::transcript_sort_key("conversation:turn:7"),
            Some((7, 1))
        );
        assert_eq!(
            MemorySystem::transcript_sort_key("conversation:user:7"),
            Some((7, 0))
        );
    }
}
