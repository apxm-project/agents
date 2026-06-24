//! System commands for project initialization and environment diagnostics.

use std::env;
use std::path::PathBuf;

use anyhow::Result;
use apxm_compiler::Context as CompilerContext;
use apxm_core::constants::env as apxm_env;
use apxm_core::toolchain_env;
use apxm_core::utils::build::MlirEnvReport;
#[cfg(feature = "driver")]
use apxm_driver::ApXmConfig;

use super::dekk_hints;
#[cfg(feature = "driver")]
use super::implementations::load_config;
use super::implementations::{
    Status, print_hint, print_section_header, print_status_line, print_subsection_header,
};

pub fn init_command(name: &str) -> Result<()> {
    let base = PathBuf::from(name);
    if base.exists() {
        return Err(anyhow::anyhow!("Directory '{}' already exists", name));
    }

    let dirs = ["agents", "flows", "nodes", "prompts", "tools"];
    for d in &dirs {
        std::fs::create_dir_all(base.join(d))
            .map_err(|e| anyhow::anyhow!("Failed to create {}/{}: {}", name, d, e))?;
    }

    let toml_content = format!(
        r#"[project]
name = "{name}"
version = "0.1.0"

[build]
opt_level = "O1"

[runtime]
max_parallel = 4
"#
    );
    std::fs::write(base.join("apxm.toml"), toml_content)
        .map_err(|e| anyhow::anyhow!("Failed to write {}/apxm.toml: {}", name, e))?;

    println!("Initialized APXM project '{}'", name);
    for d in &dirs {
        println!("  {}/{}/", name, d);
    }
    println!("  {}/apxm.toml", name);
    Ok(())
}

pub fn doctor_command(config: Option<PathBuf>, json_output: bool) -> Result<()> {
    let report = MlirEnvReport::detect();
    report.apply_env();
    let mlir_available = report.is_ready();
    let mlir_prefix = report
        .resolved_prefix
        .as_ref()
        .map(|p| p.display().to_string());
    let mlir_version = report.llvm_version.clone();
    let compiler_probe = CompilerContext::new()
        .map(|_| ())
        .map_err(|err| err.to_string());
    let compiler_ready = compiler_probe.is_ok();
    let compiler_error = compiler_probe.err();

    // --- Backends ---
    let (backend_count, backend_names): (usize, Vec<String>) =
        match apxm_credentials::BackendStore::open() {
            Ok(store) => match store.list() {
                Ok(backends) => {
                    let names: Vec<String> = backends.iter().map(|b| b.name.clone()).collect();
                    (names.len(), names)
                }
                Err(_) => (0, vec![]),
            },
            Err(_) => (0, vec![]),
        };

    // --- Sandbox (bubblewrap) ---
    // bwrap confines EXC, the bash/user-tool capabilities, and sandboxed ACP
    // agents. Without it OsLevel requests fail closed. A successful --version
    // also confirms it can run (on Ubuntu the AppArmor userns profile must be
    // installed, otherwise bwrap is present but unusable).
    let bwrap_available = std::process::Command::new("bwrap")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());

    // --- Environment Variables ---
    let env_apxm_backend = env::var(apxm_env::APXM_BACKEND).ok();
    let env_mlir_dir = env::var(apxm_env::MLIR_DIR).ok();
    let env_llvm_dir = env::var(apxm_env::LLVM_DIR).ok();

    // --- Config (driver feature only) ---
    #[cfg(feature = "driver")]
    let (config_found, config_path, config_backends) = {
        match load_config(config) {
            Ok(cfg) => {
                let path = ApXmConfig::default_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "~/.apxm/config.toml".to_string());
                let backends = cfg.backends.len();
                (true, Some(path), backends)
            }
            Err(_) => {
                let path = ApXmConfig::default_path()
                    .map(|p| p.display().to_string())
                    .ok();
                (false, path, 0usize)
            }
        }
    };
    #[cfg(not(feature = "driver"))]
    let (_config_found, _config_path, _config_backends) = {
        let _ = config;
        (false, None::<String>, 0usize)
    };

    // --- JSON output mode ---
    if json_output {
        #[allow(unused_mut)]
        let mut report_json = serde_json::json!({
            "mlir": {
                "available": mlir_available,
                "prefix": mlir_prefix,
                "version": mlir_version,
                "compiler_ready": compiler_ready,
                "compiler_error": compiler_error,
            },
            "backends": {
                "count": backend_count,
                "names": backend_names,
            },
            "sandbox": {
                "bwrap_available": bwrap_available,
            },
            "environment": {
                apxm_env::APXM_BACKEND: env_apxm_backend,
                apxm_env::MLIR_DIR: env_mlir_dir,
                apxm_env::LLVM_DIR: env_llvm_dir,
            },
        });
        // Only include config section when driver feature is available
        #[cfg(feature = "driver")]
        {
            report_json["config"] = serde_json::json!({
                "found": config_found,
                "path": config_path,
                "backends": config_backends,
            });
        }
        println!("{}", serde_json::to_string_pretty(&report_json)?);
        return Ok(());
    }

    // --- Human-readable output ---

    // 1. MLIR toolchain (existing checks)
    print_section_header("MLIR Toolchain");
    print_minimal_mlir_status();

    if mlir_available {
        let detail = match &mlir_version {
            Some(v) => format!("ready (LLVM {})", v),
            None => "ready".to_string(),
        };
        print_status_line("MLIR toolchain", Status::Ok, &detail);
    } else {
        print_status_line("MLIR toolchain", Status::Error, "missing");
    }

    if compiler_ready {
        print_status_line(
            "APXM compiler",
            Status::Ok,
            "context initialization succeeded",
        );
    } else {
        let detail = compiler_error
            .as_deref()
            .unwrap_or("context initialization failed");
        print_status_line("APXM compiler", Status::Error, detail);
        print_hint(
            "If MLIR is present but the compiler is unavailable, rebuild after the real toolchain is visible so apxm-compiler does not keep stub bindings.",
        );
    }

    // 2. Backends
    print_section_header("Backends");
    if backend_count > 0 {
        print_status_line(
            "Backends",
            Status::Ok,
            &format!(
                "{} backend{} registered",
                backend_count,
                if backend_count == 1 { "" } else { "s" }
            ),
        );
    } else {
        print_status_line("Backends", Status::Warning, "none registered");
        print_hint(&format!(
            "Register a backend first with `{}`.",
            dekk_hints::BACKEND_ADD_GENERIC
        ));
    }

    // 3. Sandbox
    print_section_header("Sandbox");
    if bwrap_available {
        print_status_line("bubblewrap", Status::Ok, "available (bwrap)");
    } else {
        print_status_line("bubblewrap", Status::Warning, "not available");
        print_hint(
            "Install bubblewrap (`apt-get install bubblewrap`) to confine tool execution. \
             On Ubuntu also add an AppArmor userns profile for bwrap (see README \
             \u{2192} System dependencies). Without it, OsLevel sandbox requests fail closed.",
        );
    }

    // 4. Environment Variables
    print_section_header("Environment");
    for (name, value) in [
        (apxm_env::APXM_BACKEND, &env_apxm_backend),
        (apxm_env::MLIR_DIR, &env_mlir_dir),
        (apxm_env::LLVM_DIR, &env_llvm_dir),
    ] {
        match value {
            Some(v) => print_status_line(name, Status::Ok, v),
            None => print_status_line(name, Status::Warning, "not set"),
        }
    }
    if env_mlir_dir.is_none() || env_llvm_dir.is_none() {
        print_hint(&format!(
            "Invoke project commands through `{}` so dekk injects the MLIR/LLVM environment.",
            dekk_hints::APXM_ENV_HINT
        ));
    }

    // 4. Config (driver feature only)
    #[cfg(feature = "driver")]
    {
        print_section_header("Configuration");
        if config_found {
            let path_display = config_path.as_deref().unwrap_or("~/.apxm/config.toml");
            print_status_line(
                "Config file",
                Status::Ok,
                &format!("found at {}", path_display),
            );
            print_status_line(
                "Configured backends",
                if config_backends > 0 {
                    Status::Ok
                } else {
                    Status::Warning
                },
                &format!("{} configured", config_backends),
            );
        } else {
            let path_display = config_path.as_deref().unwrap_or("~/.apxm/config.toml");
            print_status_line(
                "Config file",
                Status::Warning,
                &format!("not found ({})", path_display),
            );
        }

        // 5. Backend registration guidance
        if !config_found {
            print_section_header("Backend Registration");
            print_status_line("Config file", Status::Warning, "not configured");
            print_hint(&format!(
                "Register a backend explicitly with `{}`.",
                dekk_hints::BACKEND_ADD_GENERIC
            ));
            print_hint(&format!(
                "Then rerun `{}` to verify the registered backend set.",
                dekk_hints::DOCTOR
            ));
        }
    }

    // Return error if MLIR is missing (critical dependency)
    if !mlir_available {
        return Err(anyhow::anyhow!("MLIR toolchain not detected"));
    }

    Ok(())
}

/// Auto-detect the conda prefix for the `apxm` environment.
///
/// Resolution order:
/// 1. [`toolchain_env::CONDA_PREFIX`] env var
/// 2. `conda info --envs --json` output (looks for an env named "apxm")
/// 3. Common paths: ~/miniforge3/envs/apxm, ~/mambaforge/envs/apxm, ~/miniconda3/envs/apxm
fn detect_conda_prefix() -> Option<PathBuf> {
    // 1. Check conda-reported prefix
    if let Ok(prefix) = env::var(toolchain_env::CONDA_PREFIX) {
        let p = PathBuf::from(&prefix);
        if p.is_dir() {
            return Some(p);
        }
    }

    // 2. Try `conda info --envs --json`
    if let Ok(output) = std::process::Command::new("conda")
        .args(["info", "--envs", "--json"])
        .output()
        && output.status.success()
        && let Ok(text) = String::from_utf8(output.stdout)
    {
        // Minimal JSON parsing: look for paths ending in /apxm
        for line in text.lines() {
            let trimmed = line.trim().trim_matches('"').trim_end_matches(',');
            let candidate = PathBuf::from(trimmed);
            if candidate.file_name().is_some_and(|n| n == "apxm") && candidate.is_dir() {
                return Some(candidate);
            }
        }
    }

    // 3. Check common paths
    if let Some(home) = dirs::home_dir() {
        let candidates = [
            home.join("miniforge3/envs/apxm"),
            home.join("mambaforge/envs/apxm"),
            home.join("miniconda3/envs/apxm"),
        ];
        for candidate in &candidates {
            if candidate.is_dir() {
                return Some(candidate.clone());
            }
        }
    }

    None
}

fn print_minimal_mlir_status() {
    let conda_prefix = detect_conda_prefix();
    let conda_bin = conda_prefix.as_ref().map(|p| p.join("bin"));
    let mlir_tblgen = conda_bin.as_ref().map(|p| p.join("mlir-tblgen"));
    let mlir_cmake = conda_prefix.as_ref().map(|p| p.join("lib/cmake/mlir"));
    let llvm_cmake = conda_prefix.as_ref().map(|p| p.join("lib/cmake/llvm"));

    match conda_prefix.as_ref() {
        Some(prefix) => {
            print_status_line("Conda prefix", Status::Ok, &prefix.display().to_string());
        }
        None => {
            print_status_line("Conda prefix", Status::Error, "<not set>");
            print_hint(&format!(
                "Run `{}`, then invoke commands through `{}`.",
                dekk_hints::INSTALL_NO_INTERACTIVE,
                dekk_hints::APXM_ENV_HINT
            ));
            return;
        }
    }

    let checks: &[(&str, bool)] = &[
        (
            "mlir-tblgen",
            mlir_tblgen.as_ref().is_some_and(|p| p.is_file()),
        ),
        (
            "cmake/mlir",
            mlir_cmake.as_ref().is_some_and(|p| p.is_dir()),
        ),
        (
            "cmake/llvm",
            llvm_cmake.as_ref().is_some_and(|p| p.is_dir()),
        ),
    ];

    for &(label, found) in checks {
        let status = if found { Status::Ok } else { Status::Error };
        print_status_line(label, status, if found { "found" } else { "missing" });
    }

    if checks.iter().any(|(_, found)| !found) {
        print_subsection_header("Suggested Fix");
        println!("{}", dekk_hints::INSTALL_NO_INTERACTIVE);
        println!("{}", dekk_hints::DOCTOR);
        if let Some(ref prefix) = conda_prefix {
            println!("# Runtime environment prefix: {}", prefix.display());
        }
    }
}
