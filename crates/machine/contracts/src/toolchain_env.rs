//! Canonical environment keys for locating the MLIR/LLVM toolchain (conda, Dekk, custom prefixes).
//!
//! **Organization:** Runtime (`MlirEnvReport`), CLI (`apxm-cli`), driver (`apxm-driver`), and
//! `build.rs` scripts (`apxm-compiler`, `apxm-cli`) should import names from here so spellings never
//! drift. Dekk mirrors these keys via `.dekk.toml` `[env]` — keep that table aligned when adding keys.
//!
//! Bindgen/`clang-sys` uses [`LIBCLANG_PATH`]; file naming patterns for `libclang` stay in
//! the compiler build script (platform-specific binaries, not OS env contracts).

/// Typical `lib/` directory under an install prefix.
pub const PREFIX_LIB_SUBDIR: &str = "lib";

/// Env keys used to infer an install prefix (set by conda, Dekk `.dekk.toml`, or the user).
pub const MLIR_PREFIX: &str = "MLIR_PREFIX";
pub const LLVM_PREFIX: &str = "LLVM_PREFIX";
pub const CONDA_PREFIX: &str = "CONDA_PREFIX";

/// CMake package locations (directories containing `mlir`, `LLVM*Config.cmake`, etc.).
pub const MLIR_DIR: &str = "MLIR_DIR";
pub const LLVM_DIR: &str = "LLVM_DIR";

/// Passed to bindgen/`clang-sys` so it can locate `libclang` when generating FFI bindings.
pub const LIBCLANG_PATH: &str = "LIBCLANG_PATH";

/// Standard executable search path (`std::env::var`).
pub const PATH: &str = "PATH";

/// Env keys whose changes should rerun `apxm-compiler`'s build script (`cargo:rerun-if-env-changed`).
pub const APXM_COMPILER_BUILD_RERUN_ENV_KEYS: &[&str] = &[
    MLIR_PREFIX,
    LLVM_PREFIX,
    CONDA_PREFIX,
    MLIR_DIR,
    LLVM_DIR,
    LIBCLANG_PATH,
    PATH,
];

/// Prefer `MLIR_PREFIX` / `LLVM_PREFIX` over bare `CONDA_PREFIX` where multiple apply.
pub const PREFIX_ENV_KEYS_LOOKUP_ORDER: &[&str] = &[MLIR_PREFIX, LLVM_PREFIX, CONDA_PREFIX];

/// All prefix-related keys `build.rs` tools should watch for rebuild invalidation.
pub const PREFIX_ENV_KEYS_FOR_RERUN: &[&str] = &[MLIR_PREFIX, LLVM_PREFIX, CONDA_PREFIX];

/// Keys used when resolving LLVM/MLIR shared libraries beside CMake metadata.
pub const MLIR_TOOLCHAIN_LIBRARY_ENV_KEYS: &[&str] =
    &[MLIR_PREFIX, CONDA_PREFIX, LLVM_PREFIX, MLIR_DIR, LLVM_DIR];

/// User-facing hint when MLIR cannot be detected (drivers, diagnostics).
#[must_use]
pub fn missing_toolchain_env_hint() -> String {
    format!(
        "Set {mlir_dir}, {mlir_prefix}, {llvm_prefix}, or {conda_prefix}; or ensure mlir-tblgen is on {path}.",
        mlir_dir = MLIR_DIR,
        mlir_prefix = MLIR_PREFIX,
        llvm_prefix = LLVM_PREFIX,
        conda_prefix = CONDA_PREFIX,
        path = PATH,
    )
}
