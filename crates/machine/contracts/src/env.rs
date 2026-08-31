//! Process-level helpers for reading the user's environment.
//!
//! Centralizes lookups (HOME, APXM_HOME) that were previously open-coded
//! across handlers and tools, with subtle drift each time.

use std::path::PathBuf;

/// Environment variable name overriding the global APXM home directory.
pub const APXM_HOME: &str = "APXM_HOME";

/// Resolve the user's home directory.
///
/// Prefers `dirs::home_dir()` (which honors `$HOME` on Unix and the
/// platform-appropriate equivalent elsewhere). Panics with a clear message if
/// neither `$HOME` nor a passwd entry is available — every caller in the
/// workspace was previously falling back to `/root`, which is wrong on every
/// machine that isn't a stripped-down container running as root.
pub fn home_dir() -> PathBuf {
    dirs::home_dir().expect(
        "Unable to determine user home directory: $HOME is unset and no passwd entry exists",
    )
}

/// Resolve the global APXM home directory (`$APXM_HOME` or `~/.apxm`).
pub fn apxm_home() -> PathBuf {
    if let Ok(path) = std::env::var(APXM_HOME) {
        return PathBuf::from(path);
    }
    home_dir().join(".apxm")
}
