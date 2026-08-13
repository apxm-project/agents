//! Build script for the `apxm` CLI binary.
//!
//! Embed runpaths so driver-linked dylibs and MLIR/LLVM runtime libraries
//! resolve consistently for the final `apxm` binary.

use std::env;
use std::path::{Path, PathBuf};

use apxm_core::toolchain_env;

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

fn export_contract_schema_env() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // The common envelope is an immutable, provenance-pinned upstream input.
    // Resolve the checked-in snapshot from the Agents repository itself so a
    // standalone checkout never depends on a private sibling workspace.
    let common_schema = manifest_dir
        .join("../../machine/program/tests/fixtures/contracts/apxm.contract-common.v1.json");
    println!("cargo:rerun-if-changed={}", common_schema.display());
    println!(
        "cargo:rustc-env=APXM_CONTRACT_COMMON_SCHEMA_PATH={}",
        common_schema.display()
    );
}

fn main() {
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
