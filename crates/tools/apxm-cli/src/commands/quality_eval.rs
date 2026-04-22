//! `dekk apxm quality-eval` — thin wrapper that shells out to the Python
//! tier-3 harness at `tools/quality_eval`.
//!
//! Lives in Rust only so the harness is discoverable from the standard
//! `dekk apxm` surface (no `python -m` invocation required at the call
//! site). The Python module owns the actual scoring logic, fixtures, and
//! CLI flags; this wrapper does *not* re-declare them — extra arguments
//! are forwarded verbatim so adding a flag on the Python side is a
//! zero-Rust-change deployment.
//!
//! Resolution: the wrapper must be runnable from any cwd, including
//! sub-crates and out-of-tree clones. We anchor on the binary's
//! `CARGO_MANIFEST_DIR` at compile time, walk up to the workspace root,
//! and prepend `tools/` to `PYTHONPATH` so `python -m quality_eval`
//! resolves regardless of the user's current directory.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

/// Compile-time anchor for the apxm-cli crate manifest. The workspace
/// root is two parents up (`crates/tools/apxm-cli` → workspace).
const APXM_CLI_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Path segment under the workspace root holding Python tooling.
const TOOLS_SUBDIR: &str = "tools";

/// Module name passed to `python -m`.
const PY_MODULE: &str = "quality_eval";

fn workspace_root() -> PathBuf {
    // crates/tools/apxm-cli → crates/tools → crates → <workspace>
    PathBuf::from(APXM_CLI_MANIFEST_DIR)
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(APXM_CLI_MANIFEST_DIR))
}

pub fn quality_eval_command(args: Vec<String>) -> Result<()> {
    let root = workspace_root();
    let tools_dir = root.join(TOOLS_SUBDIR);
    if !tools_dir.exists() {
        anyhow::bail!(
            "quality_eval harness not found at {} — workspace anchor lost?",
            tools_dir.display()
        );
    }

    // Prepend tools/ to PYTHONPATH so the unqualified `quality_eval` import
    // resolves; preserve any pre-existing PYTHONPATH so a user with a
    // custom env (virtualenv site-packages, conda) doesn't get nuked.
    let pythonpath = match std::env::var_os("PYTHONPATH") {
        Some(existing) => {
            let mut combined = std::ffi::OsString::from(&tools_dir);
            combined.push(if cfg!(windows) { ";" } else { ":" });
            combined.push(existing);
            combined
        }
        None => std::ffi::OsString::from(&tools_dir),
    };

    let status = Command::new("python")
        .arg("-m")
        .arg(PY_MODULE)
        .args(&args)
        .env("PYTHONPATH", pythonpath)
        .current_dir(&root)
        .status()
        .with_context(|| {
            format!(
                "failed to spawn `python -m {}` from {}",
                PY_MODULE,
                root.display()
            )
        })?;

    if let Some(code) = status.code() {
        if code != 0 {
            std::process::exit(code);
        }
    } else {
        anyhow::bail!("`python -m {}` terminated by signal", PY_MODULE);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_contains_tools_quality_eval() {
        let root = workspace_root();
        let qe = root.join(TOOLS_SUBDIR).join(PY_MODULE);
        // Hard-fails when the wrapper is run from a layout that no longer
        // matches the assumed `crates/tools/apxm-cli` depth.
        assert!(
            qe.exists(),
            "workspace_root() resolved to {} but {} does not exist",
            root.display(),
            qe.display()
        );
    }
}
