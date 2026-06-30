use anyhow::{Context, Result};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use apxm_ais::{
    ARTIFACT_OPERATION_KIND_CASES_FILE, ARTIFACT_OPERATION_KIND_ENTRIES_FILE,
    generate_artifact_operation_kind_cases, generate_artifact_operation_kind_entries,
    generate_pass_descriptors, generate_pass_dispatch, generate_passes_tablegen,
};
use apxm_core::toolchain_env;
use apxm_core::utils::build::{
    LibraryConfig, LinkSpec, Platform, detect_llvm_version, emit_link_directives,
    find_versioned_mlir_library, get_target_dir, get_workspace_root, locate_library,
};
use apxm_core::{log_debug, log_info};

/// Build configuration derived from environment variables
struct BuildConfig {
    manifest_dir: PathBuf,
    out_dir: PathBuf,
    workspace_dir: PathBuf,
    profile: String,
    install_dir: PathBuf,
    build_dir: PathBuf,
}

impl BuildConfig {
    fn from_env() -> Result<Self> {
        let manifest_dir: PathBuf = env::var("CARGO_MANIFEST_DIR")
            .context("CARGO_MANIFEST_DIR not set")?
            .into();
        let out_dir: PathBuf = env::var("OUT_DIR").context("OUT_DIR not set")?.into();
        let workspace_dir = get_workspace_root(&manifest_dir)?;
        let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
        let install_dir = get_target_dir(&workspace_dir, &profile);
        let build_dir = out_dir.join("build");

        Ok(Self {
            manifest_dir,
            out_dir,
            workspace_dir,
            profile,
            install_dir,
            build_dir,
        })
    }

    fn ensure_directories(&self) -> Result<()> {
        std::fs::create_dir_all(&self.install_dir)?;
        std::fs::create_dir_all(&self.build_dir)?;
        Ok(())
    }
}

/// Paths describing the discovered MLIR installation.
struct MlirLayout {
    prefix: PathBuf,
    lib_dir: PathBuf,
    link_spec: LinkSpec,
}

impl MlirLayout {
    fn new(link_spec: LinkSpec) -> Result<Self> {
        let lib_dir = link_spec
            .search_paths
            .first()
            .context("No search paths in MLIR link spec")?
            .clone();

        let prefix = lib_dir
            .parent()
            .map(|p| p.to_path_buf())
            .context("Invalid MLIR path structure")?;

        Ok(Self {
            prefix,
            lib_dir,
            link_spec,
        })
    }
}

/// Set up cargo rerun triggers for relevant files
fn setup_rerun_triggers() {
    for path in ["CMakeLists.txt", "../../../.dekk.toml"] {
        println!("cargo:rerun-if-changed={}", path);
    }
    for path in [
        "mlir/lib",
        "mlir/include",
        "../../machine/ais/src/operations",
        "../../machine/ais/src/passes",
    ] {
        emit_rerun_if_changed_recursive(Path::new(path));
    }
    println!("cargo:rerun-if-changed=../../machine/ais/src/attrs.rs");
    for key in toolchain_env::APXM_COMPILER_BUILD_RERUN_ENV_KEYS {
        println!("cargo:rerun-if-env-changed={key}");
    }
}

fn emit_rerun_if_changed_recursive(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            emit_rerun_if_changed_recursive(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

/// Generate pass-related files from Rust definitions.
///
/// This makes Rust the single source of truth for AIS passes.
/// Generates:
/// - Passes.generated.td - MLIR TableGen pass definitions
/// - PassDispatch.inc - C API dispatch switch statement
/// - PassDescriptors.inc - Pass registry descriptors
fn generate_pass_files(out_dir: &Path, build_dir: Option<&Path>) -> Result<()> {
    // Generate Passes.generated.td
    let passes_td_content = generate_passes_tablegen();
    let passes_td_path = out_dir.join("Passes.generated.td");
    fs::write(&passes_td_path, &passes_td_content)
        .with_context(|| format!("Failed to write Passes.td: {}", passes_td_path.display()))?;
    log_info!(
        "apxm-compiler-build",
        "Generated Passes.td: {} ({} bytes)",
        passes_td_path.display(),
        passes_td_content.len()
    );

    // Generate PassDispatch.inc
    let dispatch_content = generate_pass_dispatch();
    let dispatch_path = out_dir.join("PassDispatch.inc");
    fs::write(&dispatch_path, &dispatch_content).with_context(|| {
        format!(
            "Failed to write PassDispatch.inc: {}",
            dispatch_path.display()
        )
    })?;
    log_info!(
        "apxm-compiler-build",
        "Generated PassDispatch.inc: {} ({} bytes)",
        dispatch_path.display(),
        dispatch_content.len()
    );

    // Generate PassDescriptors.inc
    let descriptors_content = generate_pass_descriptors();
    let descriptors_path = out_dir.join("PassDescriptors.inc");
    fs::write(&descriptors_path, &descriptors_content).with_context(|| {
        format!(
            "Failed to write PassDescriptors.inc: {}",
            descriptors_path.display()
        )
    })?;
    log_info!(
        "apxm-compiler-build",
        "Generated PassDescriptors.inc: {} ({} bytes)",
        descriptors_path.display(),
        descriptors_content.len()
    );

    // Copy .inc files to CMake build directory if available
    if let Some(build_dir) = build_dir {
        let cmake_include_dir = build_dir.join("include/ais/CAPI");
        fs::create_dir_all(&cmake_include_dir)?;

        for file in ["PassDispatch.inc", "PassDescriptors.inc"] {
            let src = out_dir.join(file);
            let dst = cmake_include_dir.join(file);
            fs::copy(&src, &dst)
                .with_context(|| format!("Failed to copy {} to CMake build dir", file))?;
            log_info!(
                "apxm-compiler-build",
                "Copied {} to {}",
                file,
                dst.display()
            );
        }
    }

    Ok(())
}

/// Generate native compiler artifact wire fragments from Rust definitions.
fn generate_artifact_wire_files(out_dir: &Path, build_dir: Option<&Path>) -> Result<()> {
    let generated_files = [
        (
            ARTIFACT_OPERATION_KIND_ENTRIES_FILE,
            generate_artifact_operation_kind_entries(),
        ),
        (
            ARTIFACT_OPERATION_KIND_CASES_FILE,
            generate_artifact_operation_kind_cases(),
        ),
    ];

    for (file, content) in &generated_files {
        let path = out_dir.join(file);
        fs::write(&path, content)
            .with_context(|| format!("Failed to write {}: {}", file, path.display()))?;
        log_info!(
            "apxm-compiler-build",
            "Generated {}: {} ({} bytes)",
            file,
            path.display(),
            content.len()
        );
    }

    if let Some(build_dir) = build_dir {
        let cmake_include_dir = build_dir.join("include/ais/Dialect/AIS/Conversion/Artifact");
        fs::create_dir_all(&cmake_include_dir)?;

        for (file, _) in &generated_files {
            let src = out_dir.join(file);
            let dst = cmake_include_dir.join(file);
            fs::copy(&src, &dst)
                .with_context(|| format!("Failed to copy {} to CMake build dir", file))?;
            log_info!(
                "apxm-compiler-build",
                "Copied {} to {}",
                file,
                dst.display()
            );
        }
    }

    Ok(())
}

/// Locate MLIR installation and return key directories.
fn locate_mlir_layout() -> Result<MlirLayout> {
    let config = LibraryConfig::for_mlir();
    let link_spec = locate_library(&config)
        .context("MLIR not found. Please set MLIR_DIR or install LLVM/MLIR.")?;

    MlirLayout::new(link_spec)
}

/// Execute a command and check for success
fn run_command(cmd: &mut Command, error_msg: &str) -> Result<()> {
    log_debug!("apxm-compiler-build", "Running: {:?}", cmd);
    let status = cmd
        .status()
        .with_context(|| format!("Failed to execute command: {}", error_msg))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "{} (exit code: {:?})",
            error_msg,
            status.code()
        ))
    }
}

/// Configure CMake build
fn configure_cmake(
    build_dir: &Path,
    manifest_dir: &Path,
    mlir_dir: &Path,
    install_dir: &Path,
    runtime_rpaths: &[PathBuf],
    workspace_root: &Path,
    profile: &str,
) -> Result<()> {
    let mlir_cmake_dir = mlir_dir.join("lib/cmake/mlir");
    let llvm_cmake_dir = mlir_dir.join("lib/cmake/llvm");

    // Map cargo PROFILE to an appropriate CMake build type.
    let cmake_build_type = if profile.eq_ignore_ascii_case("release") {
        "Release"
    } else {
        "Debug"
    };

    let mut cmd = Command::new("cmake");
    cmd.current_dir(build_dir)
        .arg(manifest_dir)
        .arg(format!("-DMLIR_DIR={}", mlir_cmake_dir.display()))
        .arg(format!("-DLLVM_DIR={}", llvm_cmake_dir.display()))
        .arg(format!("-DCMAKE_BUILD_TYPE={}", cmake_build_type))
        .arg(format!("-DCMAKE_INSTALL_PREFIX={}", install_dir.display()))
        .arg(format!(
            "-DAPXM_WORKSPACE_ROOT={}",
            workspace_root.display()
        ));

    if !runtime_rpaths.is_empty() {
        let joined = runtime_rpaths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(";");
        cmd.arg(format!("-DAPXM_RUNTIME_RPATHS={joined}"));
    }

    run_command(&mut cmd, "CMake configuration failed")
}

/// Build CMake target
fn build_cmake(build_dir: &Path) -> Result<()> {
    remove_zero_byte_objects(build_dir)?;

    run_command(
        Command::new("cmake").current_dir(build_dir).args([
            "--build",
            ".",
            "--target",
            "apxm_compiler_c",
            "--config",
            "Release",
            "-j",
        ]),
        "CMake build failed",
    )
}

fn remove_zero_byte_objects(dir: &Path) -> Result<()> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            remove_zero_byte_objects(&path)?;
            continue;
        }

        if metadata.is_file()
            && metadata.len() == 0
            && path.extension().and_then(|ext| ext.to_str()) == Some("o")
        {
            log_info!(
                "apxm-compiler-build",
                "Removing stale zero-byte object: {}",
                path.display()
            );
            fs::remove_file(&path)
                .with_context(|| format!("Failed to remove stale object: {}", path.display()))?;
        }
    }

    Ok(())
}

/// Install CMake build
fn install_cmake(build_dir: &Path) -> Result<()> {
    run_command(
        Command::new("cmake")
            .current_dir(build_dir)
            .args(["--install", "."]),
        "CMake install failed",
    )
}

// --- libclang discovery for bindgen / clang-sys (`toolchain_env::LIBCLANG_PATH`) ------------

/// Directory containing the loadable `libclang` shared library clang-sys expects.
const LIBCLANG_FILE_DLL: &str = "libclang.dll";
const LIBCLANG_FILE_SO: &str = "libclang.so";
const LIBCLANG_FILE_DYLIB: &str = "libclang.dylib";
const LIBCLANG_WIN_PREFIX: &str = "libclang-";
/// Windows MSVC layout (`libclang-13.dll`, …), not the host's `DLL_SUFFIX`.
const LIBCLANG_WINDOWS_DLL_SUFFIX: &str = ".dll";
const LIBCLANG_SO_DOT: &str = "libclang.so.";
/// conda-forge macOS: `libclang.13.dylib` (SONAME), often without `libclang.dylib`.
const LIBCLANG_MACOS_DOT_PREFIX: &str = "libclang.";
const LIBCLANG_MACOS_DYLIB_SUFFIX: &str = ".dylib";

fn is_libclang_shared_library_file_name(file_name: &str) -> bool {
    if matches!(
        file_name,
        LIBCLANG_FILE_DLL | LIBCLANG_FILE_SO | LIBCLANG_FILE_DYLIB
    ) {
        return true;
    }
    if let Some(body) = file_name
        .strip_prefix(LIBCLANG_WIN_PREFIX)
        .and_then(|s| s.strip_suffix(LIBCLANG_WINDOWS_DLL_SUFFIX))
    {
        return !body.is_empty() && body.chars().all(|c| c.is_ascii_digit());
    }
    if file_name.starts_with(LIBCLANG_SO_DOT) {
        return file_name.len() > LIBCLANG_SO_DOT.len();
    }
    if let Some(body) = file_name
        .strip_prefix(LIBCLANG_MACOS_DOT_PREFIX)
        .and_then(|s| s.strip_suffix(LIBCLANG_MACOS_DYLIB_SUFFIX))
    {
        return !body.is_empty() && body.chars().all(|c| c.is_ascii_digit());
    }
    false
}

fn directory_contains_libclang_shared_library(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        e.file_name()
            .to_str()
            .is_some_and(is_libclang_shared_library_file_name)
    })
}

fn find_libclang_search_dir(bin_dir: &Path, lib_dir: &Path) -> Option<PathBuf> {
    for dir in [bin_dir, lib_dir] {
        if directory_contains_libclang_shared_library(dir) {
            return Some(dir.to_path_buf());
        }
    }
    None
}

/// conda-forge on macOS ships `libclang.<major>.dylib` but not always `libclang.dylib`;
/// clang-sys looks for the unversioned name under [`toolchain_env::LIBCLANG_PATH`].
#[cfg(target_os = "macos")]
fn ensure_libclang_dylib_alias_for_bindgen(prefix: &Path) -> Result<()> {
    let lib_dir = prefix.join("lib");
    if !lib_dir.is_dir() {
        return Ok(());
    }
    let dst = lib_dir.join(LIBCLANG_FILE_DYLIB);
    if dst.exists() {
        return Ok(());
    }

    let mut versioned: Vec<(u32, PathBuf)> = Vec::new();
    for entry in
        fs::read_dir(&lib_dir).with_context(|| format!("read_dir {}", lib_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(body) = name
            .strip_prefix(LIBCLANG_MACOS_DOT_PREFIX)
            .and_then(|s| s.strip_suffix(LIBCLANG_MACOS_DYLIB_SUFFIX))
        else {
            continue;
        };
        if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(major) = body.parse::<u32>() else {
            continue;
        };
        versioned.push((major, path));
    }

    let Some((_, src)) = versioned.into_iter().max_by_key(|(m, _)| *m) else {
        return Ok(());
    };

    let link_target = src.file_name().context("libclang path has no file name")?;
    std::os::unix::fs::symlink(link_target, &dst)
        .with_context(|| format!("symlink {} -> {}", src.display(), dst.display()))?;
    log_info!(
        "apxm-compiler-build",
        "Created {} -> {} for bindgen (conda-forge macOS layout)",
        dst.display(),
        src.display()
    );
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn ensure_libclang_dylib_alias_for_bindgen(_prefix: &Path) -> Result<()> {
    Ok(())
}

fn clang_builtin_include_dirs(clang_roots: &[PathBuf]) -> Option<PathBuf> {
    let mut best: Option<(u32, PathBuf)> = None;
    for root in clang_roots {
        let lib_clang = root.join("lib/clang");
        if !lib_clang.is_dir() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&lib_clang) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(major) = name.split('.').next().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let include = path.join("include");
            if include.join("stddef.h").is_file() {
                let replace = best.as_ref().is_none_or(|(v, _)| major > *v);
                if replace {
                    best = Some((major, include));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

fn conda_sysroot_include_dir(root: &Path) -> Option<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return None;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Typical conda sysroot layout:
        //   <triplet>/sysroot/usr/include (e.g., x86_64-conda-linux-gnu)
        if !name.contains("-conda-") {
            continue;
        }
        let include = path.join("sysroot/usr/include");
        if include.join("stdint.h").is_file() {
            return Some(include);
        }
    }
    None
}

/// Generate Rust bindings using bindgen
fn generate_bindings(
    manifest_dir: &Path,
    out_dir: &Path,
    mlir_include_dir: &Path,
    project_include_dir: &Path,
    mlir_prefix: &Path,
) -> Result<()> {
    let bindings_path = out_dir.join("bindings.rs");
    let header_path = manifest_dir.join("mlir/include/ais/CAPI/Compiler.h");

    // Set up libclang path for bindgen (clang-sys reads `toolchain_env::LIBCLANG_PATH`).
    // Priority: existing env > prefix/bin (Windows conda) > prefix/lib.
    if env::var(toolchain_env::LIBCLANG_PATH).is_err() {
        let bin_dir = mlir_prefix.join("bin");
        let lib_dir = mlir_prefix.join("lib");
        let chosen = find_libclang_search_dir(&bin_dir, &lib_dir).unwrap_or(lib_dir);
        if chosen.exists() {
            log_info!(
                "apxm-compiler-build",
                "Setting {} for bindgen: {}",
                toolchain_env::LIBCLANG_PATH,
                chosen.display()
            );
            // SAFETY: Build scripts are single-threaded.
            #[allow(unsafe_code)]
            unsafe {
                env::set_var(toolchain_env::LIBCLANG_PATH, &chosen);
            }
        }
    }

    ensure_libclang_dylib_alias_for_bindgen(mlir_prefix)?;

    // Find clang resource directory (contains stddef.h and other builtins).
    // conda-forge puts them at $prefix/lib/clang/<version>/include.
    // We search a few candidate roots around the active toolchain prefix.
    let mut extra_clang_args: Vec<String> = Vec::new();
    let home_dir = apxm_core::env::home_dir();
    let mut clang_roots = vec![mlir_prefix.to_path_buf()];
    if let Ok(prefix) = env::var(toolchain_env::CONDA_PREFIX) {
        clang_roots.push(PathBuf::from(prefix));
    }
    clang_roots.push(home_dir.join("miniforge3/envs/apxm"));
    clang_roots.push(home_dir.join("miniforge3"));
    if let Some(candidate) = clang_builtin_include_dirs(&clang_roots) {
        log_info!(
            "apxm-compiler-build",
            "Found clang builtins at {}",
            candidate.display()
        );
        extra_clang_args.push(format!("-I{}", candidate.display()));
    }
    if extra_clang_args.is_empty() {
        log_info!(
            "apxm-compiler-build",
            "WARNING: Could not find clang builtins (stddef.h). Bindgen may fail."
        );
    }

    // Also add the conda sysroot include path so clang's stdint.h can
    // `#include_next <stdint.h>` to find the next system header.
    for root in &clang_roots {
        if let Some(sysroot_include) = conda_sysroot_include_dir(root) {
            log_info!(
                "apxm-compiler-build",
                "Found conda sysroot includes at {}",
                sysroot_include.display()
            );
            extra_clang_args.push(format!("-I{}", sysroot_include.display()));
            break;
        }
    }

    let builder = bindgen::Builder::default()
        .header(header_path.to_str().context("Invalid header path")?)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .allowlist_function("apxm_.*")
        .allowlist_type("Apxm.*")
        .clang_arg(format!("-I{}", mlir_include_dir.display()))
        .clang_arg(format!("-I{}", project_include_dir.display()))
        .clang_args(extra_clang_args)
        .size_t_is_usize(true);

    let bindings = builder.generate().context("Failed to generate bindings")?;

    let bindings_str = bindings.to_string();
    let with_allow = format!(
        "#[allow(dead_code, non_upper_case_globals)]\npub mod bindings_inner {{\n{}\n}}\npub use bindings_inner::*;",
        bindings_str
    );

    std::fs::write(&bindings_path, with_allow).context("Failed to write bindings")?;

    Ok(())
}

/// Emit linker directives for C++ libraries
fn emit_compiler_link_directives(install_dir: &Path, mlir_layout: &MlirLayout) -> Result<()> {
    let apxm_lib_dir = install_dir.join("lib");
    let mlir_lib_dir = &mlir_layout.lib_dir;
    let mlir_prefix = &mlir_layout.prefix;

    // Link our own library
    println!("cargo:rustc-link-search=native={}", apxm_lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=apxm_compiler_c");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", apxm_lib_dir.display());

    // Link MLIR/LLVM libraries
    println!("cargo:rustc-link-search=native={}", mlir_lib_dir.display());

    let llvm_version = detect_llvm_version(mlir_prefix).context("Failed to detect LLVM version")?;

    log_info!(
        "apxm-compiler-build",
        "Detected LLVM version: {}",
        llvm_version
    );

    // Platform-specific LLVM linking
    let platform = Platform::current();
    match platform {
        Platform::Windows => {
            println!("cargo:rustc-link-lib=dylib=LLVM-{}", llvm_version);
        }
        Platform::MacOS | Platform::Linux => {
            println!("cargo:rustc-link-lib=dylib=LLVM-{}", llvm_version);
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", mlir_lib_dir.display());
        }
    }

    // Find and link MLIR library
    if let Some(mlir_lib) = find_versioned_mlir_library(mlir_lib_dir, &llvm_version) {
        log_info!(
            "apxm-compiler-build",
            "Using MLIR library: {}",
            mlir_lib.display()
        );

        // Emit the full path on macOS and Linux for better resolution
        match platform {
            Platform::Windows => {
                // Windows just needs the library name
                println!("cargo:rustc-link-lib=dylib=MLIR-{}", llvm_version);
            }
            Platform::MacOS | Platform::Linux => {
                println!("cargo:rustc-link-arg={}", mlir_lib.display());
            }
        }
    } else {
        println!("cargo:rustc-link-lib=dylib=MLIR");
    }

    // Emit MLIR link spec directives
    emit_link_directives(&mlir_layout.link_spec, install_dir)?;

    Ok(())
}

/// Generate stub bindings when MLIR is not available.
///
/// This allows the crate to compile without a full LLVM/MLIR installation.
/// All stub functions return null/error — the Rust API layer converts them to
/// `CompilerError::MlirNotAvailable` at runtime.
fn generate_stub_bindings(out_dir: &Path) -> Result<()> {
    // Signatures are derived from how the Rust API layer in api/module.rs, api/context.rs,
    // passes/manager.rs, and passes/registry.rs call into the FFI layer.
    let stub = r"
// Stub bindings generated because MLIR was not found at build time.
// All functions return null/false/empty — the Rust wrappers surface CompilerError at runtime.
#[allow(dead_code, non_upper_case_globals, non_camel_case_types, clippy::missing_safety_doc)]
pub mod bindings_inner {
    use std::os::raw::{c_char, c_uint};

    // ── Opaque handle types ────────────────────────────────────────────────
    #[repr(C)] pub struct ApxmCompilerContext { _p: [u8; 0] }
    #[repr(C)] pub struct ApxmModule          { _p: [u8; 0] }
    #[repr(C)] pub struct ApxmPassManager     { _p: [u8; 0] }
    #[repr(C)] pub struct ApxmError           {
        pub code: c_uint,
        pub message: *const c_char,
        pub file_path: *const c_char,
        pub file_line: c_uint,
        pub file_col: c_uint,
        pub file_line_end: c_uint,
        pub file_col_end: c_uint,
        pub snippet: *const c_char,
        pub label: *const c_char,
        pub help: *const c_char,
        pub highlight_start: c_uint,
        pub highlight_end: c_uint,
    }
    #[repr(C)] pub struct ApxmPassInfo {
        pub name: *const c_char,
        pub description: *const c_char,
        pub category: c_uint,
    }
    #[repr(C)] pub struct ApxmArtifactOptions {
        pub module_name: *const c_char,
        pub emit_debug_json: bool,
        pub target_version: *const c_char,
    }

    // ── Context ────────────────────────────────────────────────────────────
    pub unsafe fn apxm_compiler_context_create() -> *mut ApxmCompilerContext { std::ptr::null_mut() }
    pub unsafe fn apxm_compiler_context_destroy(_ctx: *mut ApxmCompilerContext) {}

    // ── Module ─────────────────────────────────────────────────────────────
    /// Parse MLIR text into a module. Returns null on failure.
    pub unsafe fn apxm_module_parse(
        _ctx: *mut ApxmCompilerContext, _src: *const c_char,
    ) -> *mut ApxmModule { std::ptr::null_mut() }
    /// Verify module. Returns false on failure.
    pub unsafe fn apxm_module_verify(_m: *mut ApxmModule) -> bool { false }
    /// Serialize module to MLIR text. Caller must free with apxm_string_free.
    pub unsafe fn apxm_module_to_string(_m: *mut ApxmModule) -> *mut c_char { std::ptr::null_mut() }
    pub unsafe fn apxm_module_destroy(_m: *mut ApxmModule) {}
    /// Set a string-valued module attribute. Returns false on failure.
    pub unsafe fn apxm_module_set_string_attr(
        _m: *mut ApxmModule, _name: *const c_char, _value: *const c_char,
    ) -> bool { false }
    /// Set a bool-valued module attribute. Returns false on failure.
    pub unsafe fn apxm_module_set_bool_attr(
        _m: *mut ApxmModule, _name: *const c_char, _value: bool,
    ) -> bool { false }
    /// Remove a module attribute. Returns false on failure.
    pub unsafe fn apxm_module_remove_attr(
        _m: *mut ApxmModule, _name: *const c_char,
    ) -> bool { false }

    // ── Pass manager ───────────────────────────────────────────────────────
    pub unsafe fn apxm_pass_manager_create(
        _ctx: *mut ApxmCompilerContext,
    ) -> *mut ApxmPassManager { std::ptr::null_mut() }
    pub unsafe fn apxm_pass_manager_destroy(_pm: *mut ApxmPassManager) {}
    pub unsafe fn apxm_pass_manager_add_pass_by_name(
        _pm: *mut ApxmPassManager, _name: *const c_char,
    ) -> bool { false }
    pub unsafe fn apxm_pass_manager_has_pass(
        _pm: *mut ApxmPassManager, _name: *const c_char,
    ) -> bool { false }
    pub unsafe fn apxm_pass_manager_clear(_pm: *mut ApxmPassManager) {}
    pub unsafe fn apxm_pass_manager_run(
        _pm: *mut ApxmPassManager, _m: *mut ApxmModule,
    ) -> bool { false }

    pub unsafe fn apxm_module_drain_pass_stats(
        _m: *mut ApxmModule,
        _pass_name: *const c_char,
        fired_count_out: *mut i64,
        ir_size_delta_out: *mut i64,
    ) -> i32 {
        if !fired_count_out.is_null() {
            unsafe { *fired_count_out = 0; }
        }
        if !ir_size_delta_out.is_null() {
            unsafe { *ir_size_delta_out = 0; }
        }
        0
    }

    pub unsafe fn apxm_module_strip_all_pass_stats(_m: *mut ApxmModule) -> i32 { 0 }

    pub unsafe fn apxm_module_total_template_tokens(
        _m: *mut ApxmModule,
        total_out: *mut u64,
    ) -> i32 {
        if !total_out.is_null() {
            unsafe { *total_out = 0; }
        }
        0
    }

    // ── Pass registry ──────────────────────────────────────────────────────
    pub unsafe fn apxm_pass_registry_get_count() -> usize { 0 }
    pub unsafe fn apxm_pass_registry_get_pass(_idx: usize) -> *const ApxmPassInfo { std::ptr::null() }
    pub unsafe fn apxm_pass_registry_find_pass(_name: *const c_char) -> *const ApxmPassInfo { std::ptr::null() }

    // ── Codegen ────────────────────────────────────────────────────────────
    /// Emit artifact bytes. Returns allocated buffer (caller frees with apxm_codegen_free), or null.
    pub unsafe fn apxm_codegen_emit_artifact(
        _m: *mut ApxmModule, _opts: *const ApxmArtifactOptions,
    ) -> *mut c_char { std::ptr::null_mut() }
    pub unsafe fn apxm_codegen_free(_buf: *mut c_char) {}

    // ── String / error ─────────────────────────────────────────────────────
    pub unsafe fn apxm_string_free(_s: *mut c_char) {}
    pub unsafe fn apxm_error_collector_count() -> usize { 0 }
    pub unsafe fn apxm_error_collector_get_all(
        _out: *mut *mut ApxmError, _cap: usize,
    ) -> usize { 0 }
    pub unsafe fn apxm_error_collector_get_first() -> *const ApxmError { std::ptr::null() }
    pub unsafe fn apxm_error_free(_err: *mut ApxmError) {}
}
pub use bindings_inner::*;
";
    let bindings_path = out_dir.join("bindings.rs");
    std::fs::write(&bindings_path, stub).context("Failed to write stub bindings")?;
    log_info!(
        "apxm-compiler-build",
        "MLIR not found — wrote stub bindings. Compiler will return errors at runtime."
    );
    Ok(())
}

/// Main build process coordinator
fn build() -> Result<()> {
    let config = BuildConfig::from_env()?;
    config.ensure_directories()?;

    // ═══ STEP 1: Generate Pass files from Rust (Single Source of Truth) ═══
    log_info!(
        "apxm-compiler-build",
        "Generating Pass files from Rust definitions..."
    );
    generate_pass_files(&config.out_dir, Some(&config.build_dir))?;

    // ═══ STEP 1c: Generate artifact wire fragments from Rust (Single Source of Truth) ═══
    log_info!(
        "apxm-compiler-build",
        "Generating artifact wire fragments from Rust definitions..."
    );
    generate_artifact_wire_files(&config.out_dir, Some(&config.build_dir))?;

    // ═══ STEP 2: Locate MLIR installation (optional) ═══
    let mlir_layout = match locate_mlir_layout() {
        Ok(layout) => {
            log_info!(
                "apxm-compiler-build",
                "Found MLIR at: {}",
                layout.prefix.display()
            );
            Some(layout)
        }
        Err(e) => {
            log_info!(
                "apxm-compiler-build",
                "MLIR not found ({}). Building without native compiler support.",
                e
            );
            None
        }
    };

    let Some(mlir_layout) = mlir_layout else {
        // No MLIR: emit stub bindings and exit successfully.
        // The Rust wrapper will return errors at runtime when compile_graph() is called.
        // MLIR-dependent tests will be skipped (no `mlir` feature emitted).
        generate_stub_bindings(&config.out_dir)?;
        return Ok(());
    };

    // MLIR found — emit the feature flag so MLIR-dependent tests are enabled.
    println!("cargo:rustc-cfg=feature=\"mlir\"");

    let MlirLayout {
        prefix: ref mlir_dir,
        lib_dir: ref mlir_lib_dir,
        ..
    } = mlir_layout;

    let apxm_lib_dir = config.install_dir.join("lib");
    let mut runtime_rpaths = vec![apxm_lib_dir, mlir_lib_dir.clone()];
    runtime_rpaths.sort();
    runtime_rpaths.dedup();

    // Check if CMake is available
    let cmake_available = Command::new("cmake")
        .arg("--version")
        .status()
        .is_ok_and(|s| s.success());

    if cmake_available {
        log_info!("apxm-compiler-build", "Configuring CMake...");
        configure_cmake(
            &config.build_dir,
            &config.manifest_dir,
            mlir_dir,
            &config.install_dir,
            &runtime_rpaths,
            &config.workspace_dir,
            &config.profile,
        )?;

        log_info!("apxm-compiler-build", "Building CMake...");
        build_cmake(&config.build_dir)?;

        log_info!("apxm-compiler-build", "Installing CMake...");
        install_cmake(&config.build_dir)?;
    } else {
        log_info!(
            "apxm-compiler-build",
            "CMake not found in PATH. Skipping native library build."
        );
        log_info!(
            "apxm-compiler-build",
            "Please install CMake and rerun the build."
        );
    }

    // Generate bindings regardless of CMake availability
    let mlir_include_dir = mlir_dir.join("include");
    let project_include_dir = config.manifest_dir.join("mlir/include");

    generate_bindings(
        &config.manifest_dir,
        &config.out_dir,
        &mlir_include_dir,
        &project_include_dir,
        mlir_dir,
    )?;

    emit_compiler_link_directives(&config.install_dir, &mlir_layout)?;

    Ok(())
}

fn main() {
    setup_rerun_triggers();

    if let Err(e) = build() {
        eprintln!("apxm-compiler-build: build failed: {:#}", e);
        std::process::exit(1);
    }
}
