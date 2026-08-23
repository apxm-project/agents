//! Constants for the sandbox subsystem.

pub mod executables {
    pub const BASH: &str = "bash";
    pub const BUBBLEWRAP: &str = "bwrap";
    pub const ENV: &str = "env";
    pub const SHELL: &str = "sh";
    pub const SYSTEMD_RUN: &str = "systemd-run";
    pub const SYSTEMCTL: &str = "systemctl";
    pub const TRUE: &str = "true";
}

pub mod backend_names {
    pub const PROCESS: &str = "apxm-process";
    pub const BUBBLEWRAP: &str = "apxm-bwrap";
    pub const SYSTEMD: &str = "apxm-systemd";
}

pub mod session_prefixes {
    pub const PROCESS: &str = "process";
    pub const BUBBLEWRAP: &str = "bwrap";
    pub const SYSTEMD: &str = "systemd";
    pub const WORKDIR: &str = "workdir";
    pub const SCRIPT: &str = "script";
}

pub mod limits {
    use std::time::Duration;

    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
    pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 1024 * 1024;
}

pub mod env {
    use std::collections::BTreeMap;
    use std::fmt;

    pub const PATH: &str = "PATH";
    pub const HOME: &str = "HOME";
    pub const LANG: &str = "LANG";
    pub const LC_ALL: &str = "LC_ALL";
    pub const TERM: &str = "TERM";
    pub const DBUS_SESSION_BUS_ADDRESS: &str = "DBUS_SESSION_BUS_ADDRESS";
    pub const LD_LIBRARY_PATH: &str = "LD_LIBRARY_PATH";
    pub const VIRTUAL_ENV: &str = "VIRTUAL_ENV";
    pub const XDG_RUNTIME_DIR: &str = "XDG_RUNTIME_DIR";
    pub const TMPDIR: &str = "TMPDIR";
    pub const TEMP: &str = "TEMP";
    pub const TMP: &str = "TMP";

    pub const CHILD_PASSTHROUGH: &[&str] =
        &[PATH, HOME, LANG, LC_ALL, TERM, LD_LIBRARY_PATH, VIRTUAL_ENV];
    pub const SYSTEMD_LAUNCHER_PASSTHROUGH: &[&str] = &[DBUS_SESSION_BUS_ADDRESS, XDG_RUNTIME_DIR];

    pub const AWS_SECRET_ACCESS_KEY: &str = "AWS_SECRET_ACCESS_KEY";
    pub const AWS_ACCESS_KEY_ID: &str = "AWS_ACCESS_KEY_ID";
    pub const DATABASE_URL: &str = "DATABASE_URL";
    pub const SECRET_KEY: &str = "SECRET_KEY";
    pub const PRIVATE_KEY: &str = "PRIVATE_KEY";
    pub const API_KEY_MARKER: &str = "API_KEY";
    pub const ACCESS_TOKEN_MARKER: &str = "ACCESS_TOKEN";
    pub const SECRET_MARKER: &str = "SECRET";
    pub const PRIVATE_KEY_MARKER: &str = "PRIVATE_KEY";

    pub const BLOCKED_DEFAULTS: &[&str] = &[
        AWS_SECRET_ACCESS_KEY,
        AWS_ACCESS_KEY_ID,
        DATABASE_URL,
        SECRET_KEY,
        PRIVATE_KEY,
    ];

    pub fn is_sensitive_name(key: &str) -> bool {
        let key = key.to_ascii_uppercase();
        key.contains(API_KEY_MARKER)
            || key.contains(ACCESS_TOKEN_MARKER)
            || key.contains(SECRET_MARKER)
            || key.contains(PRIVATE_KEY_MARKER)
    }

    /// Build the complete environment for a sandboxed child from the small
    /// host allowlist plus explicit runtime-provided overrides.
    pub fn child_environment<I, K, V>(overrides: I) -> Vec<(String, String)>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut env = BTreeMap::new();
        for key in CHILD_PASSTHROUGH {
            if let Ok(value) = std::env::var(key) {
                env.insert((*key).to_string(), value);
            }
        }
        for (key, value) in overrides {
            env.insert(key.as_ref().to_string(), value.as_ref().to_string());
        }
        env.into_iter().collect()
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct InvalidEnvironmentEntry {
        pub name: String,
        pub reason: &'static str,
    }

    impl fmt::Display for InvalidEnvironmentEntry {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "invalid child environment entry '{}': {}",
                self.name, self.reason
            )
        }
    }

    impl std::error::Error for InvalidEnvironmentEntry {}

    pub fn validate_child_environment(
        env: &[(String, String)],
    ) -> Result<(), InvalidEnvironmentEntry> {
        for (name, value) in env {
            let reason = if name.is_empty() {
                Some("name is empty")
            } else if name.starts_with('-') {
                Some("name begins with an option marker")
            } else if name.contains('=') {
                Some("name contains '='")
            } else if name.contains('\0') {
                Some("name contains NUL")
            } else if value.contains('\0') {
                Some("value contains NUL")
            } else {
                None
            };
            if let Some(reason) = reason {
                return Err(InvalidEnvironmentEntry {
                    name: name.clone(),
                    reason,
                });
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn explicit_child_environment_values_override_and_sort_the_host_baseline() {
            let env = child_environment([
                (PATH, "/explicit/bin"),
                ("ZZZ_APXM_TEST", "last"),
                ("AAA_APXM_TEST", "first"),
            ]);

            assert_eq!(
                env.iter().find(|(key, _)| key == PATH),
                Some(&(PATH.to_string(), "/explicit/bin".to_string()))
            );
            assert!(env.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        }

        #[test]
        fn child_environment_validation_rejects_option_like_names() {
            let env = vec![("--chdir".to_string(), "/tmp".to_string())];
            assert_eq!(
                validate_child_environment(&env),
                Err(InvalidEnvironmentEntry {
                    name: "--chdir".to_string(),
                    reason: "name begins with an option marker",
                })
            );
        }
    }
}

pub mod bubblewrap {
    pub const VERSION_ARG: &str = "--version";
    pub const FLAG_NEW_SESSION: &str = "--new-session";
    pub const FLAG_DIE_WITH_PARENT: &str = "--die-with-parent";
    pub const FLAG_RO_BIND: &str = "--ro-bind";
    pub const FLAG_BIND: &str = "--bind";
    pub const FLAG_DEV: &str = "--dev";
    pub const FLAG_PROC: &str = "--proc";
    pub const FLAG_CHDIR: &str = "--chdir";
    pub const FLAG_UNSHARE_USER: &str = "--unshare-user";
    pub const FLAG_UNSHARE_PID: &str = "--unshare-pid";
    pub const FLAG_UNSHARE_NET: &str = "--unshare-net";
    pub const FLAG_TMPFS: &str = "--tmpfs";
    pub const FLAG_SEPARATOR: &str = "--";
    pub const FILESYSTEM_ROOT: &str = "/";
    pub const FILESYSTEM_DEV: &str = "/dev";
    pub const FILESYSTEM_PROC: &str = "/proc";
    pub const FILESYSTEM_RUN: &str = "/run";
    pub const FILESYSTEM_RUN_USER: &str = "/run/user";
    pub const FILESYSTEM_SYS: &str = "/sys";
    pub const FILESYSTEM_TMP: &str = "/tmp";
    pub const ERR_NOT_AVAILABLE: &str = "bubblewrap is not available on this host";
    pub const ERR_ONLY_LINUX: &str = "bubblewrap backend is only supported on Linux";
    pub const ERR_SESSION_STATE: &str = "invalid bubblewrap session state";
    pub const ERR_WORKDIR_NOT_DIRECTORY: &str = "sandbox working directory must be a directory";
    pub const ERR_TIMED_OUT: &str = "command timed out and was killed";
    pub const ERR_EXECUTION_PREFIX: &str = "bubblewrap sandbox";
}

pub mod systemd_run {
    pub const FLAG_USER: &str = "--user";
    pub const FLAG_WAIT: &str = "--wait";
    pub const FLAG_PIPE: &str = "--pipe";
    pub const FLAG_QUIET: &str = "--quiet";
    pub const FLAG_COLLECT: &str = "--collect";
    pub const FLAG_PROPERTY: &str = "--property";
    pub const FLAG_UNIT: &str = "--unit";
    pub const FLAG_WORKING_DIRECTORY: &str = "--working-directory";
    pub const ENV_CLEAR: &str = "-i";
    pub const ENV_OPTION_TERMINATOR: &str = "--";
    pub const PROPERTY_NO_NEW_PRIVILEGES: &str = "NoNewPrivileges=yes";
    pub const PROPERTY_RESTRICT_SUID_SGID: &str = "RestrictSUIDSGID=yes";
    pub const PROPERTY_LOCK_PERSONALITY: &str = "LockPersonality=yes";
    pub const PROPERTY_REMOVE_IPC: &str = "RemoveIPC=yes";
    pub const PROPERTY_UMASK: &str = "UMask=0077";
    pub const PROPERTY_KILL_MODE: &str = "KillMode=control-group";
    pub const PROPERTY_TIMEOUT_STOP: &str = "TimeoutStopSec=5s";
    pub const PROPERTY_SYSTEM_CALL_ARCHITECTURES: &str = "SystemCallArchitectures=native";
    pub const PROPERTY_SYSTEM_CALL_ERROR_NUMBER: &str = "SystemCallErrorNumber=EPERM";
    pub const PROPERTY_SYSTEM_CALL_FILTER: &str =
        "SystemCallFilter=~@mount @reboot @swap @privileged";
    pub const PROPERTY_SYSTEM_CALL_FILTER_NO_NETWORK: &str =
        "SystemCallFilter=~@mount @reboot @swap @privileged @network-io";
    pub const PROBE_RESTRICTED_OK: &str = "APXM_SYSTEMD_RESTRICTED_OK";
    pub const PROBE_NETWORK_OK: &str = "APXM_SYSTEMD_NETWORK_OK";
    pub const PROBE_NETWORK_DENIED: &str = "Operation not permitted";
    pub const ERR_NOT_AVAILABLE: &str =
        "systemd user-service sandbox is not available on this host";
    pub const ERR_ONLY_LINUX: &str = "systemd sandbox backend is only supported on Linux";
    pub const ERR_SESSION_STATE: &str = "invalid systemd sandbox session state";
    pub const ERR_EXECUTION_PREFIX: &str = "systemd sandbox";
    pub const ERR_TIMED_OUT: &str = "command timed out and was killed";
}

pub mod systemctl {
    pub const FLAG_USER: &str = "--user";
    pub const STOP: &str = "stop";
}

pub mod shell_args {
    pub const COMMAND: &str = "-c";
    pub const LOGIN_COMMAND: &str = "-lc";
}

pub mod messages {
    pub const COMMAND_BLOCKED_BY_POLICY: &str = "command blocked by sandbox policy";
    pub const PROCESS_TIMED_OUT_AND_KILLED: &str = "Process timed out and was killed";
}
