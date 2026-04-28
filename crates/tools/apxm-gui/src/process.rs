//! Subprocess invocation for the `apxm` CLI.

use std::path::{Path, PathBuf};

use crate::env::keys::LD_LIBRARY_PATH;

/// Locate the `apxm` CLI binary by trying the binary directory, then
/// `target/release/apxm`, then falling back to PATH.
pub fn find_apxm_cli() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap_or(Path::new("."));
        let sibling = dir.join("apxm");
        if sibling.exists() {
            return sibling;
        }
    }
    let cwd_release = PathBuf::from("target/release/apxm");
    if cwd_release.exists() {
        return cwd_release;
    }
    PathBuf::from("apxm")
}

/// Construct a `tokio::process::Command` for the CLI binary with
/// `LD_LIBRARY_PATH` augmented so MLIR/LLVM shared libs resolve.
pub fn apxm_command(cli_bin: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(cli_bin);

    let mut dirs: Vec<String> = Vec::new();
    if let Some(bin_dir) = cli_bin.parent().and_then(|p| std::fs::canonicalize(p).ok()) {
        let lib_dir = bin_dir.join("lib");
        if lib_dir.is_dir() {
            dirs.push(lib_dir.to_string_lossy().into_owned());
        }
        dirs.push(bin_dir.to_string_lossy().into_owned());
    }
    if let Ok(existing) = std::env::var(LD_LIBRARY_PATH) {
        if !existing.is_empty() {
            dirs.push(existing);
        }
    }
    if !dirs.is_empty() {
        cmd.env(LD_LIBRARY_PATH, dirs.join(":"));
    }

    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_apxm_cli_returns_a_path() {
        let p = find_apxm_cli();
        assert!(!p.as_os_str().is_empty());
    }

    #[test]
    fn apxm_command_uses_provided_binary() {
        let bin = PathBuf::from("/nonexistent/apxm");
        let cmd = apxm_command(&bin);
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_program(), bin.as_os_str());
    }
}
