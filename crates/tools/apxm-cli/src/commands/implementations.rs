//! Shared helpers and small utilities used across command modules.

#[cfg(feature = "driver")]
use std::path::PathBuf;

#[cfg(feature = "driver")]
use anyhow::Context;
use anyhow::Result;
#[cfg(feature = "driver")]
use apxm_driver::{ApXmConfig, ConfigError};
use colored::Colorize;

#[cfg(feature = "driver")]
pub(super) fn parse_opt_level(level: u8) -> apxm_core::types::OptimizationLevel {
    use apxm_core::types::OptimizationLevel;
    match level {
        0 => OptimizationLevel::O0,
        1 => OptimizationLevel::O1,
        2 => OptimizationLevel::O2,
        _ => OptimizationLevel::O3,
    }
}

pub(crate) fn category_str(cat: apxm_core::types::OperationCategory) -> &'static str {
    use apxm_core::types::OperationCategory;
    match cat {
        OperationCategory::Metadata => "metadata",
        OperationCategory::Memory => "memory",
        OperationCategory::Reasoning => "reasoning",
        OperationCategory::Tools => "tools",
        OperationCategory::ControlFlow => "control_flow",
        OperationCategory::Synchronization => "synchronization",
        OperationCategory::ErrorHandling => "error_handling",
        OperationCategory::Communication => "communication",
        OperationCategory::Internal => "internal",
        OperationCategory::Coordination => "coordination",
        OperationCategory::Identity => "identity",
    }
}

fn latency_to_ms(lat: apxm_core::types::OperationLatency) -> u64 {
    use apxm_core::types::OperationLatency;
    match lat {
        OperationLatency::None => 10,
        OperationLatency::Low => 100,
        OperationLatency::Medium => 1000,
        OperationLatency::High => 5000,
    }
}

pub(crate) fn find_op_spec(op: &str) -> Option<&'static apxm_core::types::OperationSpec> {
    use apxm_core::types::AIS_OPERATIONS;
    AIS_OPERATIONS.iter().find(|s| s.op_type.to_string() == op)
}

pub(super) fn op_latency_ms(op: &str) -> u64 {
    find_op_spec(op).map_or(100, |s| latency_to_ms(s.latency))
}

pub(crate) fn parse_header(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid header: no '=' found in '{s}'"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

pub(super) fn print_section_header(title: &str) {
    use apxm_core::constants::ui;
    println!();
    println!("  {}", title.bold().cyan());
    println!("  {}", ui::icons::HRULE.repeat(title.len()).dimmed());
}

pub(super) fn print_subsection_header(title: &str) {
    println!();
    println!("  {}", title.bold());
}

pub(super) fn print_hint(message: &str) {
    use apxm_core::constants::ui;
    println!("  {} {}", ui::icons::INFO.cyan(), message);
}

pub(super) enum Status {
    Ok,
    Warning,
    Error,
}

pub(super) fn print_status_line(label: &str, status: Status, value: &str) {
    use apxm_core::constants::ui;
    let (icon, status_str) = match status {
        Status::Ok => (ui::icons::SUCCESS.green(), ui::labels::OK.green().bold()),
        Status::Warning => (
            ui::icons::WARNING.yellow(),
            ui::labels::WARN.yellow().bold(),
        ),
        Status::Error => (ui::icons::FAILED.red(), ui::labels::MISSING.red().bold()),
    };
    println!("  {} {:<14} [{}] {}", icon, label.bold(), status_str, value);
}

pub(super) fn parse_duration(s: &str) -> Result<chrono::Duration> {
    let s = s.trim();
    if let Some(days_str) = s.strip_suffix('d') {
        let days: i64 = days_str.parse()?;
        Ok(chrono::Duration::days(days))
    } else if let Some(hours_str) = s.strip_suffix('h') {
        let hours: i64 = hours_str.parse()?;
        Ok(chrono::Duration::hours(hours))
    } else {
        Err(anyhow::anyhow!(
            "Invalid duration format. Use '7d' or '24h'"
        ))
    }
}

#[cfg(feature = "driver")]
pub(super) fn load_config(config: Option<PathBuf>) -> Result<ApXmConfig> {
    if let Some(path) = config {
        return ApXmConfig::load_scoped_with_explicit(Some(path.clone()))
            .map_err(|e| anyhow::anyhow!(e))
            .with_context(|| {
                format!(
                    "Failed to load config hierarchy with explicit layer {}",
                    path.display()
                )
            });
    }

    match ApXmConfig::load_scoped() {
        Ok(config) => Ok(config),
        Err(ConfigError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(ApXmConfig::default())
        }
        Err(ConfigError::HomeDirMissing) => Ok(ApXmConfig::default()),
        Err(err) => Err(anyhow::anyhow!(err)),
    }
}
