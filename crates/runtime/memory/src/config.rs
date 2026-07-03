//! Memory system configuration

use apxm_core::log_warn;
use apxm_core::paths::ApxmPaths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for Short-Term Memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StmConfig {
    /// Maximum number of entries (None = unlimited)
    pub max_entries: Option<usize>,
}

impl Default for StmConfig {
    fn default() -> Self {
        Self {
            max_entries: Some(1024), // Reasonable default
        }
    }
}

/// Backend type for Long-Term Memory
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LtmBackend {
    /// In-memory (non-persistent, for testing)
    Memory,
    /// SQLite database
    Sqlite,
    /// Redb embedded database
    Redb,
}

/// Configuration for Long-Term Memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LtmConfig {
    /// Backend type
    pub backend: LtmBackend,
    /// Database file path (ignored for Memory backend)
    pub path: Option<PathBuf>,
    /// Maximum number of connections in pool (SQLite only)
    pub max_connections: Option<u32>,
}

impl Default for LtmConfig {
    fn default() -> Self {
        Self {
            backend: LtmBackend::Sqlite,
            path: Some(default_ltm_path()),
            max_connections: Some(8),
        }
    }
}

impl LtmConfig {
    /// Create in-memory LTM configuration (for testing)
    pub fn in_memory() -> Self {
        Self {
            backend: LtmBackend::Memory,
            path: None,
            max_connections: None,
        }
    }

    /// Create SQLite LTM configuration with custom path
    pub fn sqlite<P: Into<PathBuf>>(path: P) -> Self {
        Self {
            backend: LtmBackend::Sqlite,
            path: Some(path.into()),
            max_connections: Some(8),
        }
    }
}

/// Configuration for Episodic Memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodicConfig {
    /// Maximum number of entries to keep (older entries evicted)
    pub max_entries: Option<usize>,
    /// Optional persistent JSONL file path for episodic entries.
    pub path: Option<PathBuf>,
}

impl Default for EpisodicConfig {
    fn default() -> Self {
        Self {
            max_entries: Some(10000), // Keep last 10k episodes
            path: Some(default_episodic_path()),
        }
    }
}

/// Complete memory system configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryConfig {
    pub stm_config: StmConfig,
    pub ltm_config: LtmConfig,
    pub episodic_config: EpisodicConfig,
}

impl MemoryConfig {
    /// Create configuration with in-memory LTM (for testing)
    pub fn in_memory_ltm() -> Self {
        Self {
            stm_config: StmConfig::default(),
            ltm_config: LtmConfig::in_memory(),
            episodic_config: EpisodicConfig {
                max_entries: Some(10000),
                path: None,
            },
        }
    }

    /// Create configuration with custom LTM path
    pub fn with_ltm_path<P: Into<PathBuf>>(path: P) -> Self {
        Self {
            stm_config: StmConfig::default(),
            ltm_config: LtmConfig::sqlite(path),
            episodic_config: EpisodicConfig::default(),
        }
    }
}

fn memory_dir_path(filename: &str, fallback: &str) -> PathBuf {
    if let Ok(paths) = ApxmPaths::discover() {
        match paths.memory_dir() {
            Ok(memory_dir) => return memory_dir.join(filename),
            Err(err) => log_warn!(
                "memory::config",
                error = %err,
                "Failed to create memory directory under the state root"
            ),
        }
    }
    PathBuf::from(fallback)
}

fn default_ltm_path() -> PathBuf {
    memory_dir_path("ltm.sqlite", "apxm_ltm.sqlite")
}

fn default_episodic_path() -> PathBuf {
    memory_dir_path("episodes.jsonl", "apxm_episodes.jsonl")
}
