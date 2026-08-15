//! System commands for environment diagnostics.

use std::env;
use std::path::PathBuf;

use anyhow::Result;
use apxm_core::constants::env as apxm_env;
use apxm_core::toolchain_env;
use apxm_core::utils::build::MlirEnvReport;

use super::dekk_hints;
use super::implementations::{
    Status, print_hint, print_section_header, print_status_line, print_subsection_header,
};

pub fn doctor_command(config: Option<PathBuf>, json_output: bool) -> Result<()> {
    let report = MlirEnvReport::detect();
    report.apply_env();
    let mlir_available = report.is_ready();
    let mlir_prefix = report
        .resolved_prefix
        .as_ref()
        .map(|p| p.display().to_string());
    let mlir_version = report.llvm_version.clone();

    let env_mlir_dir = env::var(apxm_env::MLIR_DIR).ok();
    let env_llvm_dir = env::var(apxm_env::LLVM_DIR).ok();
    let package_contract = inspect_package_contract();
    let _ = config;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "environment": {
                    apxm_env::MLIR_DIR: env_mlir_dir,
                    apxm_env::LLVM_DIR: env_llvm_dir,
                    "mlir_available": mlir_available,
                    "mlir_prefix": mlir_prefix,
                    "mlir_version": mlir_version,
                },
                "package_contract": package_contract,
            }))?
        );
        return Ok(());
    }

    print_section_header("Environment");
    print_minimal_mlir_status();
    if mlir_available {
        let detail = match &mlir_version {
            Some(v) => format!("ready (LLVM {v})"),
            None => "ready".to_owned(),
        };
        print_status_line("MLIR toolchain", Status::Ok, &detail);
    } else {
        print_status_line("MLIR toolchain", Status::Error, "missing");
    }
    for (name, value) in [
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

    print_section_header("Package contract");
    match package_contract
        .get("status")
        .and_then(|value| value.as_str())
    {
        Some("ok") => print_status_line("agent.toml", Status::Ok, "apxm.agent"),
        Some(status) => print_status_line("agent.toml", Status::Warning, status),
        None => print_status_line("agent.toml", Status::Warning, "absent"),
    }

    if !mlir_available {
        return Err(anyhow::anyhow!("MLIR toolchain not detected"));
    }
    Ok(())
}

fn inspect_package_contract() -> serde_json::Value {
    let path = PathBuf::from("agent.toml");
    if !path.is_file() {
        return serde_json::json!({"status": "absent"});
    }
    match std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| text.parse::<toml::Value>().ok())
    {
        Some(document)
            if document.get("schema_version").and_then(toml::Value::as_str)
                == Some("apxm.agent") =>
        {
            serde_json::json!({"status": "ok", "path": "agent.toml"})
        }
        Some(_) => serde_json::json!({"status": "unrecognized", "path": "agent.toml"}),
        None => serde_json::json!({"status": "unreadable", "path": "agent.toml"}),
    }
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
