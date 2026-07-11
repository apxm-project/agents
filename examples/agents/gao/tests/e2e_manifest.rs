use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::super::{agent_build, agent_lint, agent_sync, gao_example_agent_dir, IntegrityToml};

fn require_gao_example() -> std::path::PathBuf {
    let root = gao_example_agent_dir();
    assert!(root.join("agent.toml").is_file(), "Gao package is missing");
    root
}

fn copy_gao_example() -> TempDir {
    let tmp = tempfile::tempdir().expect("Gao tempdir");
    let root = tmp.path().join("gao");
    copy_dir_all(&require_gao_example(), &root).expect("copy Gao fixture");
    tmp
}

#[test]
fn gao_python_package_syncs_and_lints() {
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_sync(&root, true).expect("Gao agent sync");
    agent_lint(&root, None, true).expect("Gao agent lint");
}

#[test]
fn gao_python_package_build_seals_integrity() {
    let tmp = copy_gao_example();
    let root = tmp.path().join("gao");
    agent_build(&root, true).expect("Gao agent build");
    let integrity: IntegrityToml = toml::from_str(
        &fs::read_to_string(root.join("integrity.toml")).expect("read integrity.toml"),
    )
    .expect("parse integrity.toml");
    assert!(
        integrity.algorithm == "sha256" && !integrity.chain.is_empty(),
        "build must write an integrity chain"
    );
}

#[test]
fn gao_python_package_contains_local_search_and_canvas_handlers() {
    let root = require_gao_example();
    assert!(root.join("capabilities/search_papers/capability.toml").is_file());
    assert!(root.join("capabilities/handlers/paper_search.py").is_file());
    assert!(root.join("capabilities/handlers/apxm_authoring.py").is_file());
    assert!(root.join("python/gao_agent.py").is_file());
    assert!(root.join("shared/papers.json").is_file());
}

#[test]
fn gao_sync_rejects_package_prefixed_capability_id() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("gao-bad-cap");
    fs::create_dir_all(root.join("capabilities/bad_cap")).unwrap();
    copy_dir_all(&gao_example_agent_dir(), &root).unwrap();
    fs::write(
        root.join("capabilities/bad_cap/capability.toml"),
        "id = \"gao.bad_cap\"\ndescription = \"bad\"\n",
    )
    .unwrap();
    fs::write(
        root.join("capabilities/bad_cap/permission.toml"),
        "capability = \"gao.bad_cap\"\ndecision = \"allow\"\n",
    )
    .unwrap();

    let err = agent_sync(&root, true).expect_err("prefixed capability id must fail sync/lint");
    let message = err.to_string();
    assert!(
        message.contains("must not embed the package id prefix")
            || message.contains("must be flat (no '.' segments)")
            || message.contains("ids must match their folder name"),
        "expected package-prefixed capability rejection, got: {message}"
    );
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
