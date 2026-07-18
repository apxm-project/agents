//! Episodic Memory - Append-only execution trace log
//!
//! Records execution history for debugging, auditing, and agent reflection.
//! Maintains temporal order and supports querying by execution ID.

use super::config::EpisodicConfig;
use apxm_backends::SearchResult;
use apxm_core::{constants::memory as mem_const, error::RuntimeError, types::values::Value};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

type Result<T> = std::result::Result<T, RuntimeError>;

const EPISODIC_TEMP_PREFIX: &str = ".";
const EPISODIC_TEMP_SUFFIX: &str = ".tmp";

/// A single episodic memory entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodicEntry {
    /// Unique entry ID
    pub id: String,
    /// Timestamp when entry was created
    pub timestamp: DateTime<Utc>,
    /// Type of event (e.g., "operation_started", "llm_call", "error")
    pub event_type: String,
    /// Event payload/data
    pub payload: Value,
    /// Execution ID this entry belongs to
    pub execution_id: String,
    /// Node ID that produced this entry (if applicable)
    pub node_id: Option<u64>,
    /// Session directory path (if applicable)
    pub session_dir: Option<PathBuf>,
}

impl EpisodicEntry {
    /// Create a new episodic entry
    pub fn new(
        event_type: String,
        payload: Value,
        execution_id: String,
        node_id: Option<u64>,
        session_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            timestamp: Utc::now(),
            event_type,
            payload,
            execution_id,
            node_id,
            session_dir,
        }
    }
}

/// Episodic Memory - bounded in-memory index over an append-only execution log.
pub struct EpisodicMemory {
    entries: RwLock<VecDeque<EpisodicEntry>>,
    max_entries: Option<usize>,
    path: Option<PathBuf>,
}

impl EpisodicMemory {
    /// Create a new episodic memory with the given configuration
    pub fn new(config: EpisodicConfig) -> Result<Self> {
        if config.max_entries == Some(0) {
            return Err(memory_error(
                "configuration",
                "max_entries must be greater than zero",
            ));
        }
        let entries = match config.path.as_deref() {
            Some(path) => load_entries(path, config.max_entries)?,
            None => VecDeque::new(),
        };

        Ok(Self {
            entries: RwLock::new(entries),
            max_entries: config.max_entries,
            path: config.path,
        })
    }

    /// Create unlimited episodic memory (for testing)
    pub fn unlimited() -> Self {
        Self {
            entries: RwLock::new(VecDeque::new()),
            max_entries: None,
            path: None,
        }
    }

    /// Record a new episodic entry
    pub async fn record(
        &self,
        event_type: String,
        payload: Value,
        execution_id: String,
        node_id: Option<u64>,
        session_dir: Option<PathBuf>,
    ) -> Result<String> {
        let entry = EpisodicEntry::new(event_type, payload, execution_id, node_id, session_dir);
        let entry_id = entry.id.clone();

        let mut entries = self.entries.write().await;

        let mut next_entries = entries.clone();
        let compaction_required = self
            .max_entries
            .is_some_and(|max_entries| next_entries.len() >= max_entries);
        if let Some(max_entries) = self.max_entries {
            while next_entries.len() >= max_entries {
                next_entries.pop_front();
            }
        }
        next_entries.push_back(entry.clone());

        if compaction_required {
            self.replace_entries(&next_entries)?;
        } else {
            self.append_entry(&entry)?;
        }
        *entries = next_entries;

        Ok(entry_id)
    }

    /// Get all entries for a specific execution ID
    pub async fn get_by_execution(&self, execution_id: &str) -> Result<Vec<EpisodicEntry>> {
        let entries = self.entries.read().await;
        Ok(entries
            .iter()
            .filter(|e| e.execution_id == execution_id)
            .cloned()
            .collect())
    }

    /// Get all entries for a specific node ID
    pub async fn get_by_node(&self, node_id: u64) -> Result<Vec<EpisodicEntry>> {
        let entries = self.entries.read().await;
        Ok(entries
            .iter()
            .filter(|e| e.node_id == Some(node_id))
            .cloned()
            .collect())
    }

    /// Get entry by ID
    pub async fn get_by_id(&self, id: &str) -> Result<Option<EpisodicEntry>> {
        let entries = self.entries.read().await;
        Ok(entries.iter().find(|e| e.id == id).cloned())
    }

    /// Get all entries (ordered by timestamp)
    pub async fn get_all(&self) -> Result<Vec<EpisodicEntry>> {
        let entries = self.entries.read().await;
        Ok(entries.iter().cloned().collect())
    }

    /// Get recent entries (last N)
    pub async fn get_recent(&self, limit: usize) -> Result<Vec<EpisodicEntry>> {
        let entries = self.entries.read().await;
        let start = entries.len().saturating_sub(limit);
        Ok(entries.range(start..).cloned().collect())
    }

    /// Search episodic memory by event type or execution ID
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let entries = self.entries.read().await;
        let mut results = Vec::new();

        for entry in entries.iter() {
            if entry.event_type.contains(query) || entry.execution_id.contains(query) {
                results.push(SearchResult {
                    key: entry.id.clone(),
                    value: serde_json::to_value(entry)
                        .map_err(|e| {
                            RuntimeError::Serialization(format!(
                                "Failed to serialize episodic entry: {}",
                                e
                            ))
                        })?
                        .try_into()
                        .unwrap_or(Value::Null),
                    score: 1.0,
                });

                if results.len() >= limit {
                    break;
                }
            }
        }

        Ok(results)
    }

    /// Get current number of entries
    pub async fn len(&self) -> usize {
        self.entries.read().await.len()
    }

    /// Check if empty
    pub async fn is_empty(&self) -> bool {
        self.entries.read().await.is_empty()
    }

    /// Clear all entries
    pub async fn clear(&self) -> Result<()> {
        let mut entries = self.entries.write().await;
        self.replace_entries(&VecDeque::new())?;
        entries.clear();
        Ok(())
    }

    /// Get statistics
    pub async fn stats(&self) -> Result<EpisodicStats> {
        let entries = self.entries.read().await;

        let total_entries = entries.len();
        let unique_executions = entries
            .iter()
            .map(|e| &e.execution_id)
            .collect::<std::collections::HashSet<_>>()
            .len();

        let oldest_timestamp = entries.front().map(|e| e.timestamp);
        let newest_timestamp = entries.back().map(|e| e.timestamp);

        Ok(EpisodicStats {
            total_entries,
            unique_executions,
            oldest_timestamp,
            newest_timestamp,
        })
    }

    /// Commit one new record to the append log before publishing it in memory.
    fn append_entry(&self, entry: &EpisodicEntry) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let parent = ensure_parent(path)?;
        let line = serde_json::to_string(entry)
            .map_err(|error| RuntimeError::Serialization(error.to_string()))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| memory_error("open append log", error))?;
        file.write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_data())
            .map_err(|error| memory_error("append log", error))?;
        sync_directory(&parent)?;
        Ok(())
    }

    /// Atomically compact the durable log to the supplied retained window.
    fn replace_entries(&self, entries: &VecDeque<EpisodicEntry>) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let parent = ensure_parent(path)?;
        let payload = encode_entries(entries)?;
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                memory_error(
                    "replace log",
                    format!("path {} has no UTF-8 filename", path.display()),
                )
            })?;
        let temporary_path = parent.join(format!(
            "{EPISODIC_TEMP_PREFIX}{filename}.{}{}",
            uuid::Uuid::now_v7(),
            EPISODIC_TEMP_SUFFIX
        ));

        let replace_result = (|| -> Result<()> {
            let mut temporary = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary_path)
                .map_err(|error| memory_error("create replacement log", error))?;
            temporary
                .write_all(&payload)
                .and_then(|()| temporary.sync_all())
                .map_err(|error| memory_error("write replacement log", error))?;
            drop(temporary);
            std::fs::rename(&temporary_path, path)
                .map_err(|error| memory_error("activate replacement log", error))?;
            sync_directory(&parent)
        })();
        if replace_result.is_err() {
            let _ = std::fs::remove_file(&temporary_path);
        }
        replace_result
    }
}

/// Restore a valid append-log prefix, rejecting corrupt committed records.
fn load_entries(path: &Path, max_entries: Option<usize>) -> Result<VecDeque<EpisodicEntry>> {
    if !path.exists() {
        return Ok(VecDeque::new());
    }
    let content = std::fs::read_to_string(path).map_err(|error| memory_error("read log", error))?;
    let has_final_newline = content.ends_with('\n');
    let mut entries = VecDeque::new();
    let lines = content.split('\n').collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<EpisodicEntry>(line) {
            Ok(entry) => entries.push_back(entry),
            Err(_) if index + 1 == lines.len() && !has_final_newline => break,
            Err(error) => {
                return Err(memory_error(
                    "decode log",
                    format!("record {} is invalid: {error}", index + 1),
                ));
            }
        }
    }
    if let Some(max_entries) = max_entries {
        while entries.len() > max_entries {
            entries.pop_front();
        }
    }
    Ok(entries)
}

/// Serialize the retained window as newline-delimited records.
fn encode_entries(entries: &VecDeque<EpisodicEntry>) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    for entry in entries {
        serde_json::to_writer(&mut payload, entry)
            .map_err(|error| RuntimeError::Serialization(error.to_string()))?;
        payload.push(b'\n');
    }
    Ok(payload)
}

/// Create and return the directory containing the configured log path.
fn ensure_parent(path: &Path) -> Result<PathBuf> {
    let parent = match path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        Some(parent) => parent.to_path_buf(),
        None => {
            std::env::current_dir().map_err(|error| memory_error("resolve log directory", error))?
        }
    };
    std::fs::create_dir_all(&parent)
        .map_err(|error| memory_error("create log directory", error))?;
    Ok(parent)
}

/// Persist a directory entry mutation after append or replacement.
fn sync_directory(directory: &Path) -> Result<()> {
    File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| memory_error("sync log directory", error))
}

/// Construct a typed episodic persistence failure.
fn memory_error(operation: &str, error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Memory {
        message: format!("episodic {operation}: {error}"),
        space: Some(mem_const::EPISODIC.to_string()),
    }
}

/// Statistics for episodic memory
#[derive(Debug, Clone)]
pub struct EpisodicStats {
    pub total_entries: usize,
    pub unique_executions: usize,
    pub oldest_timestamp: Option<DateTime<Utc>>,
    pub newest_timestamp: Option<DateTime<Utc>>,
}

/// Restart and crash-prefix coverage for the episodic append log.
#[cfg(test)]
mod tests {
    use super::{EpisodicMemory, load_entries};
    use crate::config::EpisodicConfig;
    use apxm_core::types::values::Value;

    /// Produce an isolated log path for one persistence test.
    fn temporary_path(test_name: &str) -> std::path::PathBuf {
        let unique = format!(
            "{test_name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time after epoch")
                .as_nanos(),
        );
        std::env::temp_dir().join(unique).join("episodes.jsonl")
    }

    /// Construct one explicit persistent episodic configuration.
    fn config(path: std::path::PathBuf, max_entries: Option<usize>) -> EpisodicConfig {
        EpisodicConfig {
            max_entries,
            path: Some(path),
        }
    }

    #[tokio::test]
    async fn append_log_restores_entries_in_recorded_order_after_restart() {
        let path = temporary_path("apxm-episodic-restart");
        let memory = EpisodicMemory::new(config(path.clone(), Some(8))).expect("episodic log");
        memory
            .record(
                "first".to_string(),
                Value::String("one".to_string()),
                "execution-1".to_string(),
                Some(1),
                None,
            )
            .await
            .expect("first append");
        memory
            .record(
                "second".to_string(),
                Value::String("two".to_string()),
                "execution-1".to_string(),
                Some(2),
                None,
            )
            .await
            .expect("second append");

        let restored = EpisodicMemory::new(config(path.clone(), Some(8))).expect("restart log");
        let entries = restored.get_all().await.expect("restored entries");
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.event_type.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"],
        );

        std::fs::remove_dir_all(path.parent().expect("episode parent"))
            .expect("remove temporary episode directory");
    }

    #[tokio::test]
    async fn retention_replaces_the_log_atomically_and_restores_the_retained_window() {
        let path = temporary_path("apxm-episodic-retention");
        let memory = EpisodicMemory::new(config(path.clone(), Some(2))).expect("episodic log");
        for event_type in ["first", "second", "third"] {
            memory
                .record(
                    event_type.to_string(),
                    Value::String(event_type.to_string()),
                    "execution-1".to_string(),
                    None,
                    None,
                )
                .await
                .expect("append episode");
        }

        let entries = load_entries(&path, Some(2)).expect("reopen retained log");
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.event_type.as_str())
                .collect::<Vec<_>>(),
            vec!["second", "third"],
        );

        std::fs::remove_dir_all(path.parent().expect("episode parent"))
            .expect("remove temporary episode directory");
    }

    #[test]
    fn recovery_ignores_only_an_incomplete_final_append() {
        let path = temporary_path("apxm-episodic-torn-final-record");
        let valid = serde_json::json!({
            "id": "episode-1",
            "timestamp": "2026-01-01T00:00:00Z",
            "event_type": "complete",
            "payload": "value",
            "execution_id": "execution-1",
            "node_id": null,
            "session_dir": null,
        });
        std::fs::create_dir_all(path.parent().expect("episode parent"))
            .expect("create temporary episode directory");
        std::fs::write(&path, format!("{valid}\n{{\"id\":\"partial")).expect("write torn append");

        let entries = load_entries(&path, Some(8)).expect("recover valid prefix");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries.front().expect("entry").event_type, "complete");

        std::fs::remove_dir_all(path.parent().expect("episode parent"))
            .expect("remove temporary episode directory");
    }
}
