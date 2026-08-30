use std::path::PathBuf;
use std::process::Command;

/// Locate the `apxm-dev` fixture binary.
///
/// Cargo normally injects `CARGO_BIN_EXE_apxm-dev` as a compile-time env for
/// integration tests. Some host cargo/rustc combinations only export it at
/// run time, so this helper accepts either and finally falls back to the
/// machine-local target directory.
pub fn command() -> Command {
    Command::new(apxm_dev_path())
}

fn apxm_dev_path() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_apxm-dev") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_apxm-dev") {
        return PathBuf::from(path);
    }
    let mut path = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target"));
    path.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    path.push("apxm-dev");
    path
}
