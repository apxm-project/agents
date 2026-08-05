//! Build script for the `apxm` CLI binary.
//!
//! Two unrelated jobs share this file because Cargo allows only one
//! `build.rs` per crate:
//!
//! 1. Embed runpaths so optional driver-linked dylibs and MLIR/LLVM runtime
//!    libraries resolve consistently for the final `apxm` binary.
//! 2. Generate the typed HTTP client (`src/client/mod.rs`) from
//!    `openapi/openapi-session-v1.yaml` via `progenitor` — folded in from the
//!    former standalone `apxm-client` crate; it had no consumer
//!    outside this binary.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use apxm_core::toolchain_env;

/// Job 2: generate the progenitor client from the OpenAPI spec into
/// `OUT_DIR/apxm_client_codegen.rs`, included by `src/client/mod.rs`.
fn generate_client_codegen() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let spec_path = manifest_dir.join("openapi/openapi-session-v1.yaml");
    println!("cargo:rerun-if-changed={}", spec_path.display());

    let spec_file = std::fs::File::open(&spec_path).unwrap_or_else(|error| {
        panic!(
            "failed to open OpenAPI spec {}: {error}",
            spec_path.display()
        )
    });
    let spec: openapiv3::OpenAPI =
        serde_yaml::from_reader(spec_file).expect("OpenAPI spec parses as YAML");

    let mut generator = progenitor::Generator::default();
    let tokens = generator
        .generate_tokens(&spec)
        .expect("progenitor generates client tokens");
    let ast = syn::parse2(tokens).expect("generated tokens parse as Rust syntax");
    let content = prettyplease::unparse(&ast);

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    std::fs::write(out_dir.join("apxm_client_codegen.rs"), content)
        .expect("write generated client");
}

// --- Cargo `build.rs` directives (machine-readable prefixes from Cargo docs) ---------------

mod cargo_out {
    /// `cargo:rerun-if-env-changed=` — rebuild when the toolchain prefix env changes.
    pub(super) fn rerun_if_env_changed(key: &str) {
        println!("cargo:rerun-if-env-changed={key}");
    }

    pub(super) fn rustc_link_arg(flag: impl AsRef<str>) {
        println!("cargo:rustc-link-arg={}", flag.as_ref());
    }
}

mod link_flag {
    use std::path::Path;

    const WL_RPATH: &str = "-Wl,-rpath,";

    /// dyld: path of the loading binary; resolves next to the executable.
    #[cfg(target_os = "macos")]
    const DYLIB_SUBDIR_REL_TO_LOADER: &str = "@loader_path/lib";
    /// ELF: path of the loading binary; resolves next to the executable.
    #[cfg(target_os = "linux")]
    const DYLIB_SUBDIR_REL_TO_LOADER: &str = "$ORIGIN/lib";

    /// Directory next to this executable (`target/<profile>/`) where Cargo places dylibs (`lib/`).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) fn install_name_toolchain_libs_dir() -> String {
        format!("{WL_RPATH}{DYLIB_SUBDIR_REL_TO_LOADER}")
    }

    pub(super) fn prefix_lib_rpath(prefix_lib: &Path) -> String {
        format!("{WL_RPATH}{}", prefix_lib.display())
    }
}

fn first_prefix_from_env() -> Option<String> {
    toolchain_env::PREFIX_ENV_KEYS_LOOKUP_ORDER
        .iter()
        .find_map(|key| env::var(key).ok())
}

fn prefix_lib_dir(prefix: &str) -> PathBuf {
    Path::new(prefix).join(toolchain_env::PREFIX_LIB_SUBDIR)
}

fn git_common_repo_root(start: &Path) -> PathBuf {
    let output = Command::new("git")
        .arg("-C")
        .arg(start)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output();
    let Ok(output) = output else {
        return start.to_path_buf();
    };
    if !output.status.success() {
        return start.to_path_buf();
    }
    let common_dir = match std::str::from_utf8(&output.stdout) {
        Ok(stdout) => PathBuf::from(stdout.trim()),
        Err(_) => return start.to_path_buf(),
    };
    if common_dir.file_name() != Some(OsStr::new(".git")) {
        return start.to_path_buf();
    }
    common_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| start.to_path_buf())
}

fn export_contract_schema_env() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let agents_root = git_common_repo_root(&manifest_dir);
    let workspace_root = agents_root.parent().unwrap_or(&agents_root);
    let common_schema = workspace_root
        .join("contracts")
        .join("schemas")
        .join("contract-common.v1.json");
    println!("cargo:rerun-if-changed={}", common_schema.display());
    println!(
        "cargo:rustc-env=APXM_CONTRACT_COMMON_SCHEMA_PATH={}",
        common_schema.display()
    );
}

fn main() {
    generate_client_codegen();
    export_contract_schema_env();

    for key in toolchain_env::PREFIX_ENV_KEYS_FOR_RERUN {
        cargo_out::rerun_if_env_changed(key);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    cargo_out::rustc_link_arg(link_flag::install_name_toolchain_libs_dir());

    if let Some(prefix) = first_prefix_from_env() {
        let lib = prefix_lib_dir(&prefix);
        if lib.is_dir() {
            cargo_out::rustc_link_arg(link_flag::prefix_lib_rpath(&lib));
        }
    }
}
