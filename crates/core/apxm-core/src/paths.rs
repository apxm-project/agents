//! Helpers for resolving APXM workspace directories.
//!
//! Canonical storage resolution:
//! - explicit path (handled by the caller)
//! - local project storage at `<repo>/.apxm`
//! - global fallback at `$APXM_HOME` or `~/.apxm`
//!
//! Read paths prefer an existing local root. Write paths try local first and
//! fall back to global when the local root cannot be created.

use crate::env::state_home;
use dirs::home_dir;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const ENV_HOME: &str = "APXM_HOME";
const PROJECT_DIR: &str = ".apxm";
const ARTIFACTS_DIR: &str = "artifacts";
const CACHE_DIR: &str = "cache";
const COMPILER_DIR: &str = "compiler";
const LOGS_DIR: &str = "logs";
const SESSIONS_DIR: &str = "sessions";
const MEMORY_DIR: &str = "memory";

/// Resolved APXM directories for the current process.
#[derive(Debug, Clone)]
pub struct ApxmPaths {
    home_dir: PathBuf,
    project_dir: PathBuf,
    /// Read-write state root (`$APXM_STATE_HOME`, else `apxm_home()`). Sessions
    /// (and other mutable state) anchor here so a deploy can mount config
    /// read-only while keeping state on a writable volume.
    state_dir: PathBuf,
}

impl ApxmPaths {
    /// Discover paths based on current working directory and environment.
    pub fn discover() -> io::Result<Self> {
        let cwd = env::current_dir()?;

        let project_candidate = Self::resolve_project_dir(&cwd);
        let project_dir = Self::canonicalize_if_exists(project_candidate);
        let home_dir = Self::resolve_home_dir()
            .map(Self::canonicalize_if_exists)
            .unwrap_or_else(|_| project_dir.clone());
        let state_dir = Self::canonicalize_if_exists(state_home());

        Ok(Self {
            home_dir,
            project_dir,
            state_dir,
        })
    }

    fn resolve_home_dir() -> io::Result<PathBuf> {
        if let Ok(path) = env::var(ENV_HOME) {
            return Ok(PathBuf::from(path));
        }

        let home = home_dir().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Unable to determine user home directory",
            )
        })?;
        Ok(home.join(PROJECT_DIR))
    }

    fn resolve_project_dir(start: &Path) -> PathBuf {
        for ancestor in start.ancestors() {
            let candidate = ancestor.join(PROJECT_DIR);
            if candidate.is_dir() {
                return candidate;
            }
        }
        start.join(PROJECT_DIR)
    }

    fn canonicalize_if_exists(path: PathBuf) -> PathBuf {
        if path.exists() {
            path.canonicalize().unwrap_or(path)
        } else {
            path
        }
    }

    fn ensure_subdir_at(root: &Path, name: &str) -> io::Result<PathBuf> {
        fs::create_dir_all(root)?;
        let path = root.join(name);
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    fn ensure_subdir_with_fallback(&self, name: &str) -> io::Result<PathBuf> {
        match Self::ensure_subdir_at(&self.project_dir, name) {
            Ok(path) => Ok(path),
            Err(project_err) => {
                if self.home_dir == self.project_dir {
                    return Err(project_err);
                }
                Self::ensure_subdir_at(&self.home_dir, name)
            }
        }
    }

    /// Global home directory (typically `~/.apxm`).
    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    /// Project-specific directory (`<repo>/.apxm`).
    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    /// Read-write state root (`$APXM_STATE_HOME`, else `apxm_home()`).
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Directory for persisted memory databases (`<state_dir>/memory`),
    /// created on demand.
    pub fn memory_dir(&self) -> io::Result<PathBuf> {
        Self::ensure_subdir_at(&self.state_dir, MEMORY_DIR)
    }

    /// Path to the project-scoped configuration file.
    pub fn project_config_path(&self) -> PathBuf {
        self.project_dir.join("config.toml")
    }

    /// Directory for compiler-owned project configuration and training data.
    pub fn compiler_dir(&self) -> io::Result<PathBuf> {
        Self::ensure_subdir_at(&self.project_dir, COMPILER_DIR)
    }

    /// Path to the compiler-owned project configuration file.
    pub fn compiler_config_path(&self) -> PathBuf {
        self.project_dir.join(COMPILER_DIR).join("config.toml")
    }

    /// Directory for compiled artifacts, e.g. `.apxm/artifacts`.
    pub fn artifacts_dir(&self) -> io::Result<PathBuf> {
        Self::ensure_subdir_at(&self.project_dir, ARTIFACTS_DIR)
    }

    /// Directory for cached files, preferring local project storage and
    /// falling back to global storage when local creation fails.
    pub fn cache_dir(&self) -> io::Result<PathBuf> {
        self.ensure_subdir_with_fallback(CACHE_DIR)
    }

    /// Directory for a component under `.apxm/cache`.
    pub fn cache_component_dir(&self, component: &str) -> io::Result<PathBuf> {
        let cache_dir = self.cache_dir()?;
        let path = cache_dir.join(component);
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    /// Directory for logs, preferring local project storage and falling back
    /// to global storage when local creation fails.
    pub fn logs_dir(&self) -> io::Result<PathBuf> {
        self.ensure_subdir_with_fallback(LOGS_DIR)
    }

    /// Directory for session output under the read-write state root
    /// (`<state_dir>/sessions`). Sessions are mutable per-run state, so they
    /// anchor on the state root rather than the CWD-walked project dir — this
    /// matches the rollout layout (`<apxm_home>/sessions`) and lets a deploy
    /// mount config read-only.
    pub fn sessions_dir(&self) -> io::Result<PathBuf> {
        Self::ensure_subdir_at(&self.state_dir, SESSIONS_DIR)
    }

    /// Preferred sessions directory for read operations (`<state_dir>/sessions`).
    pub fn sessions_dir_for_read(&self) -> PathBuf {
        self.state_dir.join(SESSIONS_DIR)
    }

    /// Session lookup roots ordered by precedence.
    ///
    /// The state root is authoritative for new sessions. A pre-existing local
    /// `.apxm/sessions` is still searched first so sessions written before the
    /// state-root contract remain resolvable.
    pub fn session_lookup_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::with_capacity(2);

        if self.project_dir.is_dir() {
            dirs.push(self.project_dir.join(SESSIONS_DIR));
        }

        let state_sessions = self.state_dir.join(SESSIONS_DIR);
        if !dirs.contains(&state_sessions) {
            dirs.push(state_sessions);
        }

        dirs
    }
}

/// Build a stable per-node workspace directory name.
pub fn session_node_dir_name(node_id: u64, node_name: &str) -> String {
    let mut sanitized = String::with_capacity(node_name.len());
    let mut last_was_separator = false;

    for ch in node_name.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            sanitized.push('_');
            last_was_separator = true;
        }
    }

    let sanitized = sanitized.trim_matches('_');
    let suffix = if sanitized.is_empty() {
        "node"
    } else {
        sanitized
    };

    format!("{node_id:02}_{suffix}")
}

