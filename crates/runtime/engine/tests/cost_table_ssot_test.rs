//! RTG-2 grep gate (decision 3, `docs/plans/routing.md`): `models.toml`
//! `ModelEntry` (`crates/runtime/engine/src/model_router/registry.rs`) is the
//! one-and-only cost table in this repo. The dead cost/tag fields that used
//! to live on `BackendConfig`/`ModelConfig` and on `ModelProfile`
//! (`model_profiles.toml`) were deleted; this test fails the build if a new
//! `cost_per_1k_*` field creeps back in on a second struct.
//!
//! Implemented as a plain source walk (no `grep`/`git` subprocess dependency)
//! so it runs the same in CI and locally regardless of what's on `PATH`.

use std::collections::BTreeSet;
use std::path::Path;

/// Walk `dir` collecting `.rs` files, skipping build/vendor/VCS directories.
fn collect_rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                name,
                "target" | ".git" | ".dekk" | "node_modules" | "workspace"
            ) {
                continue;
            }
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// For each `.rs` file, track the innermost `struct <Name>` a `cost_per_1k*`
/// field declaration falls under (`pub cost_per_1k_input: f64` etc). Plain
/// value-construction sites (`cost_per_1k_input: 0.0,`) and doc-comment
/// examples (`//!`) don't count as struct *definitions*.
fn structs_declaring_cost_field(path: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return found;
    };
    let mut current_struct: Option<String> = None;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed
            .split("struct ")
            .nth(1)
            .filter(|_| trimmed.contains("struct "))
        {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                current_struct = Some(name);
            }
        }
        // A struct field declaration: `pub cost_per_1k_input: f64,` (or
        // `Option<f64>`). Skip doc-comment prose (`//!`, `///`) and plain
        // struct-literal construction (`cost_per_1k_input: 0.0,`) by
        // requiring the `pub` keyword, which only appears on declarations.
        if trimmed.starts_with("pub ") && trimmed.contains("cost_per_1k") {
            let owner = current_struct
                .clone()
                .unwrap_or_else(|| format!("<unknown struct in {}>", path.display()));
            found.insert(owner);
        }
    }
    found
}

#[test]
fn cost_per_1k_field_lives_on_exactly_one_struct() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    // crates/runtime/engine -> repo root (agents).
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("crates/runtime/engine is three levels under the repo root");

    let mut files = Vec::new();
    collect_rs_files(repo_root, &mut files);
    assert!(
        !files.is_empty(),
        "sanity check: expected to find .rs files under {}",
        repo_root.display()
    );

    let mut owners: BTreeSet<String> = BTreeSet::new();
    for file in &files {
        owners.extend(structs_declaring_cost_field(file));
    }

    assert_eq!(
        owners,
        BTreeSet::from(["ModelEntry".to_string()]),
        "cost_per_1k_* fields must live on exactly one struct (models.toml's \
         ModelEntry, decision 3 of docs/plans/routing.md); found: {owners:?}. \
         If you added a new cost field, either put it on ModelEntry or update \
         this gate deliberately."
    );
}
