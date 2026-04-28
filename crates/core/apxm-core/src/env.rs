//! Process-level helpers for reading the user's environment.
//!
//! Centralizes lookups (HOME, APXM_HOME, APXM_SERVER_URL) that were previously
//! open-coded across handlers and tools, with subtle drift each time.

use crate::constants::defaults::DEFAULT_SERVER_URL;
use std::path::PathBuf;

/// Environment variable name for the APXM server URL override.
pub const APXM_SERVER_URL: &str = "APXM_SERVER_URL";

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

/// Resolve the APXM server URL, honoring `APXM_SERVER_URL` and falling back to
/// the canonical default (`http://127.0.0.1:18800`).
pub fn server_url() -> String {
    std::env::var(APXM_SERVER_URL).unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string())
}

/// Resolve the APXM server URL with an explicit override.
///
/// If `override_url` is `Some`, it wins. Otherwise falls back to
/// `APXM_SERVER_URL` and then the canonical default. Used by handlers that
/// accept a per-node `server_url` attribute.
pub fn server_url_with_override(override_url: Option<String>) -> String {
    override_url
        .or_else(|| std::env::var(APXM_SERVER_URL).ok())
        .unwrap_or_else(|| DEFAULT_SERVER_URL.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env-var mutation tests must serialize.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn home_dir_returns_a_path() {
        let _g = ENV_LOCK.lock().unwrap();
        let h = home_dir();
        assert!(!h.as_os_str().is_empty());
    }

    #[test]
    #[allow(unsafe_code)]
    fn apxm_home_honors_env_override() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var(APXM_HOME).ok();
        // SAFETY: serialized via ENV_LOCK; restored at end of test.
        unsafe {
            std::env::set_var(APXM_HOME, "/tmp/apxm-env-test");
        }
        assert_eq!(apxm_home(), PathBuf::from("/tmp/apxm-env-test"));
        unsafe {
            match prev {
                Some(v) => std::env::set_var(APXM_HOME, v),
                None => std::env::remove_var(APXM_HOME),
            }
        }
    }

    #[test]
    #[allow(unsafe_code)]
    fn apxm_home_falls_back_to_dot_apxm_under_home() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var(APXM_HOME).ok();
        // SAFETY: serialized via ENV_LOCK; restored at end of test.
        unsafe {
            std::env::remove_var(APXM_HOME);
        }
        let resolved = apxm_home();
        assert!(resolved.ends_with(".apxm"));
        unsafe {
            if let Some(v) = prev {
                std::env::set_var(APXM_HOME, v);
            }
        }
    }

    #[test]
    #[allow(unsafe_code)]
    fn server_url_default_is_canonical() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var(APXM_SERVER_URL).ok();
        // SAFETY: serialized via ENV_LOCK; restored at end of test.
        unsafe {
            std::env::remove_var(APXM_SERVER_URL);
        }
        assert_eq!(server_url(), DEFAULT_SERVER_URL);
        unsafe {
            if let Some(v) = prev {
                std::env::set_var(APXM_SERVER_URL, v);
            }
        }
    }

    #[test]
    fn server_url_with_override_honors_explicit_value() {
        let _g = ENV_LOCK.lock().unwrap();
        assert_eq!(
            server_url_with_override(Some("http://explicit:9000".to_string())),
            "http://explicit:9000"
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn server_url_with_override_falls_back_to_env_then_default() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var(APXM_SERVER_URL).ok();
        // SAFETY: serialized via ENV_LOCK; restored at end of test.
        unsafe {
            std::env::set_var(APXM_SERVER_URL, "http://env:7000");
        }
        assert_eq!(server_url_with_override(None), "http://env:7000");

        unsafe {
            std::env::remove_var(APXM_SERVER_URL);
        }
        assert_eq!(server_url_with_override(None), DEFAULT_SERVER_URL);

        unsafe {
            if let Some(v) = prev {
                std::env::set_var(APXM_SERVER_URL, v);
            }
        }
    }
}
