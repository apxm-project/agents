//! Command-line argument parsing.

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "apxm-gui", about = "APXM GUI server")]
pub struct CliArgs {
    /// Listen port (overrides `APXM_GUI_PORT`).
    #[arg(long, short = 'p')]
    pub port: Option<u16>,

    /// Static asset directory (defaults to `<crate>/src/frontend-dist`).
    #[arg(long)]
    pub static_dir: Option<PathBuf>,

    /// Initial workflow file to load.
    #[arg(long)]
    pub file: Option<String>,

    /// Examples directory to scan for sample workflows.
    #[arg(long)]
    pub examples_dir: Option<PathBuf>,

    /// Trailing positional argument: convenience for `--file <path>`.
    #[arg(value_name = "FILE", trailing_var_arg = true, num_args = 0..)]
    pub trailing: Vec<String>,
}

impl CliArgs {
    /// Resolve the effective initial-file: explicit `--file` wins, otherwise
    /// the trailing positional if it ends in `.air` or `.py`.
    pub fn initial_file(&self) -> Option<String> {
        if let Some(f) = self.file.as_deref() {
            return Some(absolutize(f));
        }
        self.trailing
            .iter()
            .rev()
            .find(|a| (a.ends_with(".air") || a.ends_with(".py")) && !a.starts_with("--"))
            .map(|s| absolutize(s))
    }
}

fn absolutize(path: &str) -> String {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        path.to_string()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&p).to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string())
    }
}
