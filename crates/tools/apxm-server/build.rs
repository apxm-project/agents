//! Embed runpaths so `apxm-server` resolves APXM compiler and MLIR/LLVM dylibs.
//!
//! The compiler crate links `libapxm_compiler_c` with `@rpath/...`, but Cargo
//! does not reliably propagate transitive rpath flags to final binaries. Keep
//! this leaf binary aligned with `apxm-cli`.

use std::env;
use std::path::{Path, PathBuf};

use apxm_core::toolchain_env;

mod cargo_out {
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

    #[cfg(target_os = "macos")]
    const DYLIB_SUBDIR_REL_TO_LOADER: &str = "@loader_path/lib";
    #[cfg(target_os = "linux")]
    const DYLIB_SUBDIR_REL_TO_LOADER: &str = "$ORIGIN/lib";

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

fn main() {
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
