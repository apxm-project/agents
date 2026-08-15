//! Shared helpers and small utilities used across command modules.

use colored::Colorize;

pub(super) fn print_section_header(title: &str) {
    use apxm_core::constants::ui;
    println!();
    println!("  {}", title.bold().cyan());
    println!("  {}", ui::icons::HRULE.repeat(title.len()).dimmed());
}

pub(super) enum Status {
    Ok,
    Error,
}

pub(super) fn print_status_line(label: &str, status: Status, value: &str) {
    use apxm_core::constants::ui;
    let (icon, status_str) = match status {
        Status::Ok => (ui::icons::SUCCESS.green(), ui::labels::OK.green().bold()),
        Status::Error => (ui::icons::FAILED.red(), ui::labels::MISSING.red().bold()),
    };
    println!("  {} {:<14} [{}] {}", icon, label.bold(), status_str, value);
}
