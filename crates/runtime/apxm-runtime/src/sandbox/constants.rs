//! Constants used by the sandbox subsystem.

pub mod executables {
    pub const BASH: &str = "bash";
    pub const BUBBLEWRAP: &str = "bwrap";
    pub const SHELL: &str = "sh";
}

pub mod backend_names {
    pub const PROCESS: &str = "apxm-process";
    pub const BUBBLEWRAP: &str = "apxm-bwrap";
}

pub mod session_prefixes {
    pub const PROCESS: &str = "process";
    pub const BUBBLEWRAP: &str = "bwrap";
    pub const SCRATCH: &str = "scratch";
    pub const WORKDIR: &str = "workdir";
    pub const SCRIPT: &str = "script";
}

pub mod env {
    pub const PATH: &str = "PATH";
    pub const HOME: &str = "HOME";
    pub const LANG: &str = "LANG";
    pub const LC_ALL: &str = "LC_ALL";
    pub const TERM: &str = "TERM";
    pub const TMPDIR: &str = "TMPDIR";
    pub const TEMP: &str = "TEMP";
    pub const TMP: &str = "TMP";

    pub const SAFE_PASSTHROUGH: &[&str] = &[PATH, HOME, LANG, LC_ALL, TERM];

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
}

pub mod bubblewrap {
    pub const VERSION_ARG: &str = "--version";
    pub const FLAG_NEW_SESSION: &str = "--new-session";
    pub const FLAG_DIE_WITH_PARENT: &str = "--die-with-parent";
    pub const FLAG_RO_BIND: &str = "--ro-bind";
    pub const FLAG_BIND: &str = "--bind";
    pub const FLAG_DEV: &str = "--dev";
    pub const FLAG_PROC: &str = "--proc";
    pub const FLAG_DIR: &str = "--dir";
    pub const FLAG_CHDIR: &str = "--chdir";
    pub const FLAG_UNSHARE_USER: &str = "--unshare-user";
    pub const FLAG_UNSHARE_PID: &str = "--unshare-pid";
    pub const FLAG_UNSHARE_NET: &str = "--unshare-net";
    pub const FLAG_SEPARATOR: &str = "--";
    pub const FILESYSTEM_ROOT: &str = "/";
    pub const FILESYSTEM_DEV: &str = "/dev";
    pub const FILESYSTEM_PROC: &str = "/proc";
    pub const TMP_DIR: &str = "/apxm-tmp";
    pub const WORKDIR: &str = "/apxm-workdir";
    pub const WARN_READ_ALLOWLISTS: &str = "bubblewrap backend currently enforces read-only root plus writable carve-outs, not per-path read allowlists";
    pub const ERR_NOT_AVAILABLE: &str = "bubblewrap is not available on this host";
    pub const ERR_ONLY_LINUX: &str = "bubblewrap backend is only supported on Linux";
    pub const ERR_SESSION_STATE: &str = "invalid bubblewrap session state";
    pub const ERR_WORKDIR_NOT_DIRECTORY: &str = "sandbox working directory must be a directory";
    pub const ERR_TIMED_OUT: &str = "command timed out and was killed";
    pub const ERR_EXECUTION_PREFIX: &str = "bubblewrap sandbox";
}

pub mod shell_args {
    pub const COMMAND: &str = "-c";
    pub const LOGIN_COMMAND: &str = "-lc";
}

pub mod messages {
    pub const COMMAND_BLOCKED_BY_POLICY: &str = "command blocked by sandbox policy";
    pub const PROCESS_TIMED_OUT_AND_KILLED: &str = "Process timed out and was killed";
}
