//! GUI launcher.

use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};
use colored::Colorize;

use super::dekk_hints;

pub fn gui_command(file: Option<PathBuf>, port: u16, open: bool) -> Result<()> {
    // Find the apxm-gui binary next to the current executable, or in PATH.
    let gui_bin = {
        let self_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));

        let candidate = self_dir.as_ref().map(|d| d.join("apxm-gui"));
        if candidate.as_ref().map_or(false, |p| p.exists()) {
            candidate.unwrap()
        } else {
            PathBuf::from("apxm-gui")
        }
    };

    // Resolve file to absolute path.
    let resolved_file = file.map(|f| {
        if f.is_absolute() {
            f
        } else {
            env::current_dir().unwrap_or_default().join(&f)
        }
    });

    if let Some(ref f) = resolved_file {
        if !f.exists() {
            anyhow::bail!("File not found: {}", f.display());
        }
        println!(
            "Opening {} in APXM GUI on port {}...",
            f.display().to_string().bold(),
            port.to_string().bold()
        );
    } else {
        println!("Launching APXM GUI on port {}...", port.to_string().bold());
    }

    let mut cmd = std::process::Command::new(&gui_bin);
    cmd.arg("--port").arg(port.to_string());

    if let Some(ref f) = resolved_file {
        cmd.arg("--file").arg(f);
    }

    // Pass APXM_HOME so the GUI can find examples.
    if let Ok(home) = env::var("APXM_HOME") {
        cmd.env("APXM_HOME", home);
    }

    if open {
        // Open browser after a short delay to let the server start.
        let url = format!("http://localhost:{}", port);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            let _ = std::process::Command::new("xdg-open")
                .arg(&url)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        });
    }

    let status = cmd.status().with_context(|| {
        format!(
            "Failed to launch apxm-gui (looked for: {}). Build it with: {}",
            gui_bin.display(),
            dekk_hints::BUILD_GUI
        )
    })?;

    if !status.success() {
        anyhow::bail!("apxm-gui exited with status {}", status);
    }

    Ok(())
}
