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
const INTEGRATIONS_DIR: &str = "integrations";

/// Resolved APXM directories for the current process.
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
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
    ///
    /// Validates the read-write state root against [`AGENTS_STATE_LAYOUT`]
    /// before returning: a state root that already exists but has the wrong
    /// entry kind (e.g. a plain file sitting where `sessions/` should be a
    /// directory) fails fast here instead of surfacing as a confusing I/O
    /// error deep in the rollout writer.
    pub fn discover() -> io::Result<Self> {
        let cwd = env::current_dir()?;

        let project_candidate = Self::resolve_project_dir(&cwd);
        let project_dir = Self::canonicalize_if_exists(project_candidate);
        let home_dir = Self::resolve_home_dir()
            .map_or_else(|_| project_dir.clone(), Self::canonicalize_if_exists);
        let state_dir = Self::canonicalize_if_exists(state_home());

        validate_state_layout(&state_dir)?;

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

    /// Integration catalog roots ordered by precedence.
    ///
    /// Honors `APXM_INTEGRATIONS_ROOT` when set, then `$APXM_WORKSPACE_ROOT/integrations`,
    /// then `<project>/.apxm/integrations` and `<home>/.apxm/integrations`.
    pub fn integrations_dirs(&self) -> Vec<PathBuf> {
        use crate::constants::env::{APXM_INTEGRATIONS_ROOT, APXM_WORKSPACE_ROOT};

        let mut dirs = Vec::new();
        if let Ok(root) = env::var(APXM_INTEGRATIONS_ROOT) {
            let root = PathBuf::from(root);
            if !dirs.contains(&root) {
                dirs.push(root);
            }
        }
        if let Ok(workspace) = env::var(APXM_WORKSPACE_ROOT) {
            let root = PathBuf::from(workspace).join(INTEGRATIONS_DIR);
            if !dirs.contains(&root) {
                dirs.push(root);
            }
        }
        if self.project_dir.is_dir() {
            let root = self.project_dir.join(INTEGRATIONS_DIR);
            if !dirs.contains(&root) {
                dirs.push(root);
            }
        }
        let home_integrations = self.home_dir.join(INTEGRATIONS_DIR);
        if !dirs.contains(&home_integrations) {
            dirs.push(home_integrations);
        }
        dirs
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

/// A layout entry this service owns under the state root.
struct LayoutEntry {
    /// Path relative to the state root.
    relative_path: &'static str,
    /// Whether this entry is expected to be a directory (`true`) or a file
    /// (`false`) once it exists.
    is_dir: bool,
}

/// The `agents`-owned entries of the state layout. This table is the source of
/// truth for them: no contract under `contracts/` describes the state root, so
/// a change here is the only place the layout is stated.
///
/// Entries that do not exist yet are fine — they are created on demand by
/// [`ApxmPaths::sessions_dir`] and the rollout writer. Only an existing path of
/// the *wrong kind* (e.g. a plain file where a directory belongs) is a layout
/// error.
const AGENTS_STATE_LAYOUT: &[LayoutEntry] = &[
    LayoutEntry {
        relative_path: SESSIONS_DIR,
        is_dir: true,
    },
    LayoutEntry {
        relative_path: "sessions/rollouts",
        is_dir: true,
    },
    LayoutEntry {
        relative_path: "sessions/index.sqlite",
        is_dir: false,
    },
];

/// Validate the on-disk state root against [`AGENTS_STATE_LAYOUT`].
///
/// This is a boot-time check, not a migration: it never creates or moves
/// anything. It only rejects a state root where an existing path has the
/// wrong kind (file vs. directory) for its declared role, which would
/// otherwise surface later as an opaque I/O error from the rollout writer.
pub fn validate_state_layout(state_root: &Path) -> io::Result<()> {
    for entry in AGENTS_STATE_LAYOUT {
        let path = state_root.join(entry.relative_path);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue; // not created yet — fine, callers create on demand.
        };
        let is_dir = metadata.is_dir();
        if is_dir != entry.is_dir {
            let expected = if entry.is_dir {
                "a directory"
            } else {
                "a file"
            };
            let found = if is_dir { "a directory" } else { "a file" };
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "state layout violation: {} must be {expected}, found {found}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod state_layout_tests {
    use super::validate_state_layout;
    use std::fs;
    use std::path::PathBuf;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "apxm-core-state-layout-test-{name}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn empty_state_root_is_valid() {
        let dir = scratch_dir("empty");
        assert!(validate_state_layout(&dir).is_ok());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn correct_layout_is_valid() {
        let dir = scratch_dir("correct");
        fs::create_dir_all(dir.join("sessions/rollouts")).unwrap();
        fs::write(dir.join("sessions/index.sqlite"), b"").unwrap();
        assert!(validate_state_layout(&dir).is_ok());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_where_directory_expected_is_rejected() {
        let dir = scratch_dir("file-for-dir");
        fs::create_dir_all(&dir.join("sessions")).unwrap();
        // `sessions/rollouts` must be a directory; make it a file instead.
        fs::write(dir.join("sessions/rollouts"), b"not a directory").unwrap();
        let err = validate_state_layout(&dir).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn directory_where_file_expected_is_rejected() {
        let dir = scratch_dir("dir-for-file");
        // `sessions/index.sqlite` must be a file; make it a directory instead.
        fs::create_dir_all(dir.join("sessions/index.sqlite")).unwrap();
        let err = validate_state_layout(&dir).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        fs::remove_dir_all(&dir).ok();
    }
}
