//! System / environment commands (init, doctor, install, activate).

use std::env;
use std::path::PathBuf;

use anyhow::Result;
use apxm_compiler::Context as CompilerContext;
use apxm_core::utils::build::MlirEnvReport;
#[cfg(feature = "driver")]
use apxm_driver::ApXmConfig;

#[cfg(feature = "driver")]
use super::implementations::load_config;
use super::implementations::{
    Status, print_hint, print_section_header, print_status_line, print_subsection_header,
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

    // Backend registration guidance after env setup
    println!();
    print_subsection_header("Backend Registration");
    print_hint(
        "Register a backend explicitly with `dekk apxm backend add <name> --type <cloud|onprem|local> --protocol <protocol>`.",
    );
    print_hint(
        "If the backend needs authentication, attach it during backend registration instead of relying on ad hoc shell hints.",
    );

    println!();
    print_subsection_header("Next Steps");
    println!("dekk apxm backend add <name> --type <cloud|onprem|local> --protocol <protocol>");
    println!("dekk apxm doctor");

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
                "compiler_ready": compiler_ready,
                "compiler_error": compiler_error,
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
        print_hint(
            "Register a backend first with `dekk apxm backend add <name> --type <cloud|onprem|local> --protocol <protocol>`.",
        );
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
        print_hint(
            "Invoke project commands through `dekk apxm ...` so dekk injects the MLIR/LLVM environment.",
        );
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
            print_hint(
                "Register a backend explicitly with `dekk apxm backend add <name> --type <cloud|onprem|local> --protocol <protocol>`.",
            );
            print_hint("Then rerun `dekk apxm doctor` to verify the registered backend set.");
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
            print_hint(
                "Run `dekk apxm install --no-interactive`, then invoke commands through `dekk apxm ...`.",
            );
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
        println!("dekk apxm install --no-interactive");
        println!("dekk apxm doctor");
        if let Some(ref prefix) = conda_prefix {
            println!("# Runtime environment prefix: {}", prefix.display());
        }
    }
}
