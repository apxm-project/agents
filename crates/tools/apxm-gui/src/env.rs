//! Environment-variable surface for the GUI binary.
//!
//! Wrappers around `apxm_core::env::*` so handlers never spell raw env-var
//! names inline.

use std::path::PathBuf;

pub mod keys {
    pub const APXM_GUI_PORT: &str = "APXM_GUI_PORT";
    pub const APXM_HOME: &str = apxm_core::env::APXM_HOME;
    pub const LD_LIBRARY_PATH: &str = "LD_LIBRARY_PATH";
}

pub fn home_dir() -> PathBuf {
    apxm_core::env::home_dir()
}

pub fn apxm_home() -> PathBuf {
    apxm_core::env::apxm_home()
}

pub fn gui_port(default: u16) -> u16 {
    std::env::var(keys::APXM_GUI_PORT)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn home_dir_returns_a_path() {
        let _g = ENV_LOCK.lock().unwrap();
        assert!(!home_dir().as_os_str().is_empty());
    }

    #[test]
    fn apxm_home_returns_a_path() {
        let _g = ENV_LOCK.lock().unwrap();
        assert!(!apxm_home().as_os_str().is_empty());
    }

    #[test]
    #[allow(unsafe_code)]
    fn gui_port_honors_env_override() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var(keys::APXM_GUI_PORT).ok();
        unsafe {
            std::env::set_var(keys::APXM_GUI_PORT, "23456");
        }
        assert_eq!(gui_port(11111), 23456);
        unsafe {
            match prev {
                Some(v) => std::env::set_var(keys::APXM_GUI_PORT, v),
                None => std::env::remove_var(keys::APXM_GUI_PORT),
            }
        }
    }

    #[test]
    #[allow(unsafe_code)]
    fn gui_port_falls_back_to_default() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var(keys::APXM_GUI_PORT).ok();
        unsafe {
            std::env::remove_var(keys::APXM_GUI_PORT);
        }
        assert_eq!(gui_port(18801), 18801);
        unsafe {
            if let Some(v) = prev {
                std::env::set_var(keys::APXM_GUI_PORT, v);
            }
        }
    }
}
