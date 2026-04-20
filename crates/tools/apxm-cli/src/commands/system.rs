//! System / environment commands (init, doctor, install, activate).

use std::env;
use std::path::PathBuf;

use anyhow::Result;
use apxm_core::utils::build::MlirEnvReport;
#[cfg(feature = "driver")]
use apxm_driver::ApXmConfig;

#[cfg(feature = "driver")]
use super::implementations::load_config;
use super::implementations::{
    Status, print_section_header, print_status_line, print_subsection_header, print_hint,
};

pub fn activate_command(shell: &str) -> Result<()> {
    let prefix = detect_conda_prefix().ok_or_else(|| {
        anyhow::anyhow!("Could not detect conda prefix. Activate your env or install it first.")
    })?;

    match shell {
        "sh" | "bash" | "zsh" => {
            println!(
                "export MLIR_DIR={}",
                prefix.join("lib/cmake/mlir").display()
            );
            println!(
                "export LLVM_DIR={}",
                prefix.join("lib/cmake/llvm").display()
            );
            println!("export MLIR_PREFIX={}", prefix.display());
            println!("export LLVM_PREFIX={}", prefix.display());
            println!("export PATH={}/bin:$PATH", prefix.display());
        }
        "fish" => {
            println!(
                "set -gx MLIR_DIR {}",
                prefix.join("lib/cmake/mlir").display()
            );
            println!(
                "set -gx LLVM_DIR {}",
                prefix.join("lib/cmake/llvm").display()
            );
            println!("set -gx MLIR_PREFIX {}", prefix.display());
            println!("set -gx LLVM_PREFIX {}", prefix.display());
            println!("set -gx PATH {}/bin $PATH", prefix.display());
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported shell '{}'. Use sh, bash, zsh, or fish.",
                shell
            ));
        }
    }

    Ok(())
}

pub fn install_command() -> Result<()> {
    print_section_header("APXM Install");

    if !command_available("mamba") {
        print_status_line("mamba", Status::Error, "not found");
        return Err(anyhow::anyhow!(
            "mamba not found in PATH. Install mamba first."
        ));
    }

    print_status_line("mamba", Status::Ok, "found");
    let installer = "mamba";

    print_status_line("env", Status::Ok, "creating/updating");

    let create_status = std::process::Command::new(installer)
        .args(["env", "create", "-f", "environment.yaml"])
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to run {installer}: {e}"))?;

    if !create_status.success() {
        let update_status = std::process::Command::new(installer)
            .args(["env", "update", "-f", "environment.yaml", "-n", "apxm"])
            .status()
            .map_err(|e| anyhow::anyhow!("Failed to run {installer}: {e}"))?;

        if !update_status.success() {
            return Err(anyhow::anyhow!(
                "{installer} env create/update failed. Check output."
            ));
        }
    }

    print_status_line("env", Status::Ok, "ready");

    // Provider auto-detection after env setup
    println!();
    print_subsection_header("Provider Detection");
    let detected: Vec<&str> = ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GOOGLE_API_KEY"]
        .iter()
        .filter(|k| env::var(k).is_ok())
        .copied()
        .collect();

    if !detected.is_empty() {
        for key in &detected {
            print_status_line(key, Status::Ok, "detected");
        }
        print_hint("Run `apxm doctor` to auto-configure providers.");
    } else {
        print_hint(
            "No API keys detected. Set ANTHROPIC_API_KEY or OPENAI_API_KEY, then run `apxm doctor`.",
        );
    }

    println!();
    print_subsection_header("Next Steps");
    println!("conda activate apxm");
    println!("eval \"$(cargo run -p apxm-cli -- activate)\"");

    Ok(())
}

pub fn command_available(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

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
    let mlir_available = report.is_ready();
    let mlir_prefix = report
        .resolved_prefix
        .as_ref()
        .map(|p| p.display().to_string());
    let mlir_version = report.llvm_version.clone();

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

    // --- Environment Variables ---
    let env_apxm_backend = env::var("APXM_BACKEND").ok();
    let env_mlir_dir = env::var("MLIR_DIR").ok();
    let env_llvm_dir = env::var("LLVM_DIR").ok();

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
            },
            "backends": {
                "count": backend_count,
                "names": backend_names,
            },
            "environment": {
                "APXM_BACKEND": env_apxm_backend,
                "MLIR_DIR": env_mlir_dir,
                "LLVM_DIR": env_llvm_dir,
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
        print_hint("Run `apxm backend add` to configure a backend.");
    }

    // 3. Environment Variables
    print_section_header("Environment");
    for (name, value) in [
        ("APXM_BACKEND", &env_apxm_backend),
        ("MLIR_DIR", &env_mlir_dir),
        ("LLVM_DIR", &env_llvm_dir),
    ] {
        match value {
            Some(v) => print_status_line(name, Status::Ok, v),
            None => print_status_line(name, Status::Warning, "not set"),
        }
    }
    if env_mlir_dir.is_none() || env_llvm_dir.is_none() {
        print_hint("Run `eval $(apxm activate)` to set MLIR/LLVM environment variables.");
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
                "LLM backends",
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

        // 5. Provider auto-detection
        if !config_found {
            print_section_header("Provider Auto-Detection");

            let detected: Vec<&apxm_driver::config::DetectedProvider> =
                apxm_driver::config::DETECTABLE_PROVIDERS
                    .iter()
                    .filter(|p| env::var(p.env_var).is_ok())
                    .collect();

            if detected.is_empty() {
                print_status_line("API keys", Status::Warning, "none detected");
                print_hint(
                    "Set ANTHROPIC_API_KEY, OPENAI_API_KEY, or GOOGLE_API_KEY to auto-configure.",
                );
                print_hint(
                    "Or run `apxm backend add <name> --type api` to configure manually.",
                );
            } else {
                for provider in &detected {
                    print_status_line(
                        provider.name,
                        Status::Ok,
                        &format!("detected via {}", provider.env_var),
                    );
                }

                match ApXmConfig::generate_default(&detected) {
                    Ok(config) => match ApXmConfig::default_path() {
                        Ok(path) => match config.write_to_file(&path) {
                            Ok(()) => {
                                let first = detected[0];
                                print_status_line(
                                    "Config",
                                    Status::Ok,
                                    &format!("auto-generated at {}", path.display()),
                                );
                                print_status_line(
                                    "Default model",
                                    Status::Ok,
                                    &format!("{} ({})", first.default_model, first.name),
                                );
                            }
                            Err(e) => {
                                print_status_line(
                                    "Config",
                                    Status::Warning,
                                    &format!("failed to write: {}", e),
                                );
                            }
                        },
                        Err(e) => {
                            print_status_line(
                                "Config",
                                Status::Warning,
                                &format!("cannot determine path: {}", e),
                            );
                        }
                    },
                    Err(e) => {
                        print_status_line(
                            "Config",
                            Status::Warning,
                            &format!("auto-generation failed: {}", e),
                        );
                    }
                }
            }
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
/// 1. `CONDA_PREFIX` env var
/// 2. `conda info --envs --json` output (looks for an env named "apxm")
/// 3. Common paths: ~/miniforge3/envs/apxm, ~/mambaforge/envs/apxm, ~/miniconda3/envs/apxm
fn detect_conda_prefix() -> Option<PathBuf> {
    // 1. Check CONDA_PREFIX env var
    if let Ok(prefix) = env::var("CONDA_PREFIX") {
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
            if candidate.file_name().map(|n| n == "apxm").unwrap_or(false) && candidate.is_dir() {
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
            print_status_line("Conda prefix", Status::Ok, &prefix.display().to_string())
        }
        None => {
            print_status_line("Conda prefix", Status::Error, "<not set>");
            print_hint("Run `cargo run -p apxm-cli -- install`, then `conda activate apxm`.");
            return;
        }
    }

    let checks: &[(&str, bool)] = &[
        (
            "mlir-tblgen",
            mlir_tblgen.as_ref().map(|p| p.is_file()).unwrap_or(false),
        ),
        (
            "cmake/mlir",
            mlir_cmake.as_ref().map(|p| p.is_dir()).unwrap_or(false),
        ),
        (
            "cmake/llvm",
            llvm_cmake.as_ref().map(|p| p.is_dir()).unwrap_or(false),
        ),
    ];

    for &(label, found) in checks {
        let status = if found { Status::Ok } else { Status::Error };
        print_status_line(label, status, if found { "found" } else { "missing" });
    }

    if checks.iter().any(|(_, found)| !found) {
        print_subsection_header("Suggested Fix");
        println!("cargo run -p apxm-cli -- install");
        println!("conda activate apxm");
        println!("eval \"$(cargo run -p apxm-cli -- activate)\"");
        if let Some(ref prefix) = conda_prefix {
            println!("# Or export directly from: {}", prefix.display());
        }
    }
}
