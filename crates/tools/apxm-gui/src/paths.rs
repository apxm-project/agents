//! Path-handling primitives: validation, expansion, source-kind classification.

use std::path::{Path, PathBuf};

use crate::env;
use crate::error::AppError;

/// Source file kinds the GUI recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Air,
    Py,
    Json,
    Ndjson,
    Toml,
}

impl SourceKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Air => "air",
            Self::Py => "py",
            Self::Json => "json",
            Self::Ndjson => "ndjson",
            Self::Toml => "toml",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "air" => Some(Self::Air),
            "py" => Some(Self::Py),
            "json" => Some(Self::Json),
            "ndjson" => Some(Self::Ndjson),
            "toml" => Some(Self::Toml),
            _ => None,
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(Self::from_extension)
    }
}

/// Expand a leading `~/` to the user's home directory.
pub fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        env::home_dir().join(rest)
    } else {
        PathBuf::from(path)
    }
}

/// Path to the user's `~/.apxm/sessions` directory.
pub fn sessions_dir() -> PathBuf {
    env::apxm_home().join("sessions")
}

/// Path to the user's `~/.apxm/config.toml` file.
pub fn config_path() -> PathBuf {
    env::apxm_home().join("config.toml")
}

/// Validate that a user-provided path resolves to within cwd or the sessions dir.
pub fn validate_path(path: &str) -> Result<PathBuf, AppError> {
    let expanded = expand_home(path);
    let canonical = std::fs::canonicalize(&expanded)
        .map_err(|_| AppError::not_found(format!("path not found: {path}")))?;
    let cwd = std::env::current_dir().unwrap_or_default();
    let sessions = sessions_dir();
    if canonical.starts_with(&cwd) || canonical.starts_with(&sessions) {
        Ok(canonical)
    } else {
        Err(AppError::forbidden(
            "path outside allowed directories".to_string(),
        ))
    }
}

/// Whether a directory name represents build artifacts or caches that should be
/// skipped during filesystem scans.
pub fn is_skip_dir(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules"
                | "target"
                | "__pycache__"
                | "frontend-dist"
                | "dist"
                | "build"
                | "venv"
                | "site-packages"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_kind_round_trips() {
        for kind in [
            SourceKind::Air,
            SourceKind::Py,
            SourceKind::Json,
            SourceKind::Ndjson,
            SourceKind::Toml,
        ] {
            let s = kind.as_str();
            assert_eq!(SourceKind::from_extension(s), Some(kind));
        }
        assert_eq!(SourceKind::from_extension("unknown"), None);
    }

    #[test]
    fn source_kind_from_path() {
        assert_eq!(
            SourceKind::from_path(Path::new("foo.air")),
            Some(SourceKind::Air)
        );
        assert_eq!(
            SourceKind::from_path(Path::new("foo.py")),
            Some(SourceKind::Py)
        );
        assert_eq!(
            SourceKind::from_path(Path::new("foo.json")),
            Some(SourceKind::Json)
        );
        assert_eq!(
            SourceKind::from_path(Path::new("foo.ndjson")),
            Some(SourceKind::Ndjson)
        );
        assert_eq!(SourceKind::from_path(Path::new("foo.txt")), None);
        assert_eq!(SourceKind::from_path(Path::new("foo")), None);
    }

    #[test]
    fn expand_home_with_tilde() {
        let expanded = expand_home("~/foo");
        assert!(expanded.to_string_lossy().contains("foo"));
        assert!(!expanded.to_string_lossy().starts_with("~/"));
    }

    #[test]
    fn expand_home_without_tilde() {
        assert_eq!(expand_home("/abs/path"), PathBuf::from("/abs/path"));
        assert_eq!(expand_home("rel/path"), PathBuf::from("rel/path"));
    }

    #[test]
    fn validate_path_rejects_etc() {
        let result = validate_path("/etc");
        assert!(result.is_err(), "expected /etc to be rejected");
    }

    #[test]
    fn validate_path_accepts_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let result = validate_path(&cwd.to_string_lossy());
        assert!(
            result.is_ok(),
            "expected cwd to be accepted: {:?}",
            result.err().map(|e| e.1)
        );
    }

    #[test]
    fn skip_dir_matches() {
        assert!(is_skip_dir(".git"));
        assert!(is_skip_dir("target"));
        assert!(is_skip_dir("node_modules"));
        assert!(!is_skip_dir("src"));
        assert!(!is_skip_dir("examples"));
    }
}
