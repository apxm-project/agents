//! Process-level helpers for reading the user's environment.
//!
//! Centralizes lookups (HOME, APXM_HOME) that were previously open-coded
//! across handlers and tools, with subtle drift each time.

use std::path::PathBuf;

/// Environment variable name overriding the global APXM home directory.
pub const APXM_HOME: &str = "APXM_HOME";

/// Environment variable name overriding the read-write state root.
///
/// `APXM_HOME` resolves read-only configuration (the backend roster in
/// `$APXM_HOME/config.toml`); `APXM_STATE_HOME` resolves the directory the
/// process *writes* into — sessions, memory, rollouts, checkpoints. A deploy
/// that mounts config read-only and state read-write sets the two to distinct
/// paths; left unset, state falls back to `apxm_home()` so single-mount
/// deployments behave exactly as before.
pub const APXM_STATE_HOME: &str = "APXM_STATE_HOME";

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

/// Resolve the read-write state root (`$APXM_STATE_HOME`, else `apxm_home()`).
///
/// This is where the process persists sessions, memory, rollouts, and
/// checkpoints. Config (backends) stays under [`apxm_home`]; only mutable
/// state honors this override so a deploy can keep config read-only.
pub fn state_home() -> PathBuf {
    if let Ok(path) = std::env::var(APXM_STATE_HOME) {
        return PathBuf::from(path);
    }
    apxm_home()
}
