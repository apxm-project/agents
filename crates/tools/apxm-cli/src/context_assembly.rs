//! Project-context assembly for `apxm chat`.
//!
//! Reads the `AGENTS.md` / `CLAUDE.md` hierarchy (global tier → cwd),
//! concatenates ancestor-first, and caps the result at [`MAX_BYTES`].

use std::path::{Path, PathBuf};

/// Per-directory context files, in append order.
const CONTEXT_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md"];
/// Cap on the assembled preamble (Codex's `project_doc_max_bytes` default).
const MAX_BYTES: usize = 32 * 1024;

/// Assemble project context from the current directory, using `$HOME` for the
/// global tier. Returns `None` when no context files exist.
pub fn assemble_context() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    assemble_from(&cwd, home.as_deref())
}

/// Core (testable): assemble from `start`, with `home` for the global tier.
pub fn assemble_from(start: &Path, home: Option<&Path>) -> Option<String> {
    let mut sections: Vec<String> = Vec::new();

    // Global tier — shared across every project (lowest precedence, read first).
    if let Some(home) = home {
        push_file(
            &mut sections,
            &home.join(".apxm/AGENTS.md"),
            "~/.apxm/AGENTS.md",
        );
        push_file(
            &mut sections,
            &home.join(".claude/CLAUDE.md"),
            "~/.claude/CLAUDE.md",
        );
    }

    // Project tier — ancestor-first from the git root down to `start`.
    for dir in project_chain(start) {
        for name in CONTEXT_FILES {
            let path = dir.join(name);
            let label = path.display().to_string();
            push_file(&mut sections, &path, &label);
        }
    }

    if sections.is_empty() {
        return None;
    }
    let mut joined = sections.join("\n\n");
    if joined.len() > MAX_BYTES {
        joined.truncate(MAX_BYTES);
        joined.push_str("\n\n[context truncated]");
    }
    Some(joined)
}

fn push_file(sections: &mut Vec<String>, path: &Path, label: &str) {
    if let Ok(body) = std::fs::read_to_string(path) {
        let trimmed = body.trim();
        if !trimmed.is_empty() {
            sections.push(format!("# Context: {label}\n{trimmed}"));
        }
    }
}

/// Directory chain from the git root down to `start` (ancestor-first). If no
/// `.git` is found on the path we use only `start` itself, so we never walk to
/// the filesystem root and ingest unrelated files.
fn project_chain(start: &Path) -> Vec<PathBuf> {
    let mut chain: Vec<PathBuf> = Vec::new();
    let mut git_root_idx: Option<usize> = None;
    let mut cur = Some(start.to_path_buf());
    while let Some(dir) = cur {
        if dir.join(".git").exists() {
            git_root_idx = Some(chain.len());
        }
        chain.push(dir.clone());
        cur = dir.parent().map(Path::to_path_buf);
    }
    // chain = [start, parent, …, fs_root]; keep start..=git_root if found.
    let kept = match git_root_idx {
        Some(idx) => &chain[..=idx],
        None => &chain[..1],
    };
    kept.iter().rev().cloned().collect() // ancestor-first
}
