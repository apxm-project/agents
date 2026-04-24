//! Helpers for resolving APXM workspace directories.
//!
//! Canonical storage resolution:
//! - explicit path (handled by the caller)
//! - local project storage at `<repo>/.apxm`
//! - global fallback at `$APXM_HOME` or `~/.apxm`
//!
//! Read paths prefer an existing local root. Write paths try local first and
//! fall back to global when the local root cannot be created.

use dirs::home_dir;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const ENV_HOME: &str = "APXM_HOME";
const PROJECT_DIR: &str = ".apxm";
const ARTIFACTS_DIR: &str = "artifacts";
const CACHE_DIR: &str = "cache";
const LOGS_DIR: &str = "logs";
const SESSIONS_DIR: &str = "sessions";

/// Resolved APXM directories for the current process.
#[derive(Debug, Clone)]
pub struct ApxmPaths {
    home_dir: PathBuf,
    project_dir: PathBuf,
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

        Ok(Self {
            home_dir,
            project_dir,
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

    fn read_root(&self) -> &Path {
        if self.project_dir.is_dir() {
            &self.project_dir
        } else {
            &self.home_dir
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

    /// Path to the project-scoped configuration file.
    pub fn project_config_path(&self) -> PathBuf {
        self.project_dir.join("config.toml")
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

    /// Directory for logs, preferring local project storage and falling back
    /// to global storage when local creation fails.
    pub fn logs_dir(&self) -> io::Result<PathBuf> {
        self.ensure_subdir_with_fallback(LOGS_DIR)
    }

    /// Directory for session output, preferring local project storage and
    /// falling back to global storage when local creation fails.
    pub fn sessions_dir(&self) -> io::Result<PathBuf> {
        self.ensure_subdir_with_fallback(SESSIONS_DIR)
    }

    /// Preferred sessions directory for read operations.
    ///
    /// If a local `.apxm` root already exists, reads stay within that root.
    /// Otherwise reads fall back to the global home root without creating a
    /// new local `.apxm` directory.
    pub fn sessions_dir_for_read(&self) -> PathBuf {
        self.read_root().join(SESSIONS_DIR)
    }

    /// Session lookup roots ordered by precedence.
    ///
    /// Local project storage is searched first when it exists, then global
    /// storage is used as a fallback for resolving a specific session ID.
    pub fn session_lookup_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::with_capacity(2);

        if self.project_dir.is_dir() {
            dirs.push(self.project_dir.join(SESSIONS_DIR));
        }

        if dirs.is_empty() || self.home_dir != self.project_dir {
            dirs.push(self.home_dir.join(SESSIONS_DIR));
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

#[cfg(test)]
mod tests {
    use super::{ApxmPaths, session_node_dir_name};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "apxm-paths-test-{name}-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn session_node_dir_name_sanitizes_labels() {
        assert_eq!(
            session_node_dir_name(7, "Spawn Architect"),
            "07_spawn_architect"
        );
        assert_eq!(session_node_dir_name(12, "!!!"), "12_node");
    }

    #[test]
    fn sessions_dir_prefers_local_storage_when_available() {
        let root = temp_path("local-storage");
        let project_dir = root.join("workspace").join(".apxm");
        let home_dir = root.join("home").join(".apxm");
        let paths = ApxmPaths {
            home_dir,
            project_dir: project_dir.clone(),
        };

        let sessions_dir = paths.sessions_dir().unwrap();
        assert_eq!(sessions_dir, project_dir.join("sessions"));
        assert!(sessions_dir.is_dir());
    }

    #[test]
    fn sessions_dir_falls_back_to_global_when_local_storage_fails() {
        let root = temp_path("session-fallback");
        std::fs::create_dir_all(&root).unwrap();
        let blocker = root.join("blocked");
        std::fs::write(&blocker, "not a directory").unwrap();

        let home_dir = root.join("home").join(".apxm");
        let paths = ApxmPaths {
            home_dir: home_dir.clone(),
            project_dir: blocker.join(".apxm"),
        };

        let sessions_dir = paths.sessions_dir().unwrap();
        assert_eq!(sessions_dir, home_dir.join("sessions"));
        assert!(sessions_dir.is_dir());
    }

    #[test]
    fn cache_dir_falls_back_to_global_when_local_storage_fails() {
        let root = temp_path("cache-fallback");
        std::fs::create_dir_all(&root).unwrap();
        let blocker = root.join("blocked");
        std::fs::write(&blocker, "not a directory").unwrap();

        let home_dir = root.join("home").join(".apxm");
        let paths = ApxmPaths {
            home_dir: home_dir.clone(),
            project_dir: blocker.join(".apxm"),
        };

        let cache_dir = paths.cache_dir().unwrap();
        assert_eq!(cache_dir, home_dir.join("cache"));
        assert!(cache_dir.is_dir());
    }

    #[test]
    fn sessions_dir_for_read_prefers_existing_local_root() {
        let root = temp_path("read-local");
        let project_dir = root.join("workspace").join(".apxm");
        let home_dir = root.join("home").join(".apxm");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::create_dir_all(&home_dir).unwrap();

        let paths = ApxmPaths {
            home_dir: home_dir.clone(),
            project_dir: project_dir.clone(),
        };

        assert_eq!(paths.sessions_dir_for_read(), project_dir.join("sessions"));
        assert_eq!(
            paths.session_lookup_dirs(),
            vec![project_dir.join("sessions"), home_dir.join("sessions")]
        );
    }

    #[test]
    fn sessions_dir_for_read_uses_global_when_local_root_missing() {
        let root = temp_path("read-global");
        let project_dir = root.join("workspace").join(".apxm");
        let home_dir = root.join("home").join(".apxm");
        std::fs::create_dir_all(&home_dir).unwrap();

        let paths = ApxmPaths {
            home_dir: home_dir.clone(),
            project_dir,
        };

        assert_eq!(paths.sessions_dir_for_read(), home_dir.join("sessions"));
        assert_eq!(paths.session_lookup_dirs(), vec![home_dir.join("sessions")]);
    }
}
