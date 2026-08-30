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
const COMPILER_DIR: &str = "compiler";
const LOGS_DIR: &str = "logs";
const INTEGRATIONS_DIR: &str = "integrations";

/// Resolved APXM directories for the current process.
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
pub struct ApxmPaths {
    home_dir: PathBuf,
    project_dir: PathBuf,
}

impl ApxmPaths {
    /// Discover paths based on current working directory and environment.
    ///
    pub fn discover() -> io::Result<Self> {
        let cwd = env::current_dir()?;

        let project_candidate = Self::resolve_project_dir(&cwd);
        let project_dir = Self::canonicalize_if_exists(project_candidate);
        let home_dir = Self::resolve_home_dir()
            .map_or_else(|_| project_dir.clone(), Self::canonicalize_if_exists);
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

    /// Global home directory (typically `~/.apxm`).
    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    /// Project-specific directory (`<repo>/.apxm`).
    pub fn project_dir(&self) -> &Path {
        &self.project_dir
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
}
