//! APXM process inspection and cleanup.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde::Serialize;

use super::cli::ProcessAction;

const CORE_JOB_COMMANDS: &[&str] = &["compile", "execute", "run"];
const WORKFLOW_COMMAND: &str = "workflow";
const WORKFLOW_RUN_COMMAND: &str = "run";
const APXM_BINARY: &str = "apxm";
const DEKK_BINARY: &str = "dekk";
const APXM_SERVER_BINARY: &str = "apxm-server";
const APXM_GUI_BINARY: &str = "apxm-gui";
const VLLM_ENTRYPOINT: &str = "vllm.entrypoints.cli.main";
const VLLM_SCRIPT: &str = "tools/scripts/vllm.py";

#[derive(Clone, Debug)]
struct ProcessScope {
    include_services: bool,
    include_vllm: bool,
}

#[derive(Debug, Serialize)]
struct ProcessReport {
    pid: i32,
    ppid: i32,
    pgid: i32,
    kind: String,
    cwd: Option<PathBuf>,
    command: String,
}

#[derive(Debug)]
struct ProcEntry {
    pid: i32,
    ppid: i32,
    pgid: i32,
    cmdline: Vec<String>,
    cwd: Option<PathBuf>,
}

pub fn process_command(action: ProcessAction, json: bool) -> Result<()> {
    match action {
        ProcessAction::List {
            include_services,
            include_vllm,
        } => {
            let scope = ProcessScope {
                include_services,
                include_vllm,
            };
            let matches = matching_processes(&scope)?;
            emit_process_list(&matches, json)
        }
        ProcessAction::Stop {
            dry_run,
            force,
            include_services,
            include_vllm,
        } => {
            let scope = ProcessScope {
                include_services,
                include_vllm,
            };
            let matches = matching_processes(&scope)?;
            stop_processes(&matches, dry_run, force, json)
        }
    }
}

fn matching_processes(scope: &ProcessScope) -> Result<Vec<ProcessReport>> {
    let project_root = project_root_from_current_dir()?;
    let excluded = current_lineage()?;
    let mut matches = Vec::new();

    for entry in proc_entries()? {
        if excluded.contains(&entry.pid) {
            continue;
        }
        if !process_belongs_to_project(&entry, &project_root) {
            continue;
        }
        let Some(kind) = classify_process(&entry.cmdline, scope) else {
            continue;
        };
        matches.push(ProcessReport {
            pid: entry.pid,
            ppid: entry.ppid,
            pgid: entry.pgid,
            kind,
            cwd: entry.cwd,
            command: entry.cmdline.join(" "),
        });
    }

    matches.sort_by_key(|process| (process.kind.clone(), process.pid));
    Ok(matches)
}

fn project_root_from_current_dir() -> Result<PathBuf> {
    let current_dir = std::env::current_dir()
        .context("failed to read current directory")?
        .canonicalize()
        .context("failed to canonicalize current directory")?;
    find_project_root(&current_dir).with_context(|| {
        format!(
            "failed to resolve APXM project root from {}",
            current_dir.display()
        )
    })
}

fn find_project_root(start: &Path) -> Option<PathBuf> {
    for candidate in start.ancestors() {
        if is_project_root(candidate) {
            return Some(candidate.to_path_buf());
        }
    }
    Some(start.to_path_buf())
}

fn is_project_root(path: &Path) -> bool {
    (path.join(".dekk.toml").is_file() || path.join("Cargo.toml").is_file())
        && path.join("crates").is_dir()
}

#[cfg(unix)]
fn proc_entries() -> Result<Vec<ProcEntry>> {
    let mut entries = Vec::new();
    for dir_entry in std::fs::read_dir("/proc").context("failed to read /proc")? {
        let dir_entry = dir_entry?;
        let file_name = dir_entry.file_name();
        let Some(pid_text) = file_name.to_str() else {
            continue;
        };
        let Ok(pid) = pid_text.parse::<i32>() else {
            continue;
        };
        let proc_dir = dir_entry.path();
        let cmdline = read_cmdline(&proc_dir.join("cmdline"));
        if cmdline.is_empty() {
            continue;
        }
        let (ppid, pgid) = read_stat_ids(&proc_dir.join("stat")).unwrap_or((0, 0));
        let cwd = std::fs::read_link(proc_dir.join("cwd")).ok();
        entries.push(ProcEntry {
            pid,
            ppid,
            pgid,
            cmdline,
            cwd,
        });
    }
    Ok(entries)
}

#[cfg(not(unix))]
fn proc_entries() -> Result<Vec<ProcEntry>> {
    anyhow::bail!("process cleanup is currently supported on Unix-like systems")
}

fn read_cmdline(path: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}

fn read_stat_ids(path: &Path) -> Option<(i32, i32)> {
    let text = std::fs::read_to_string(path).ok()?;
    let close = text.rfind(") ")?;
    let fields: Vec<&str> = text[close + 2..].split_whitespace().collect();
    let ppid = fields.get(1)?.parse().ok()?;
    let pgid = fields.get(2)?.parse().ok()?;
    Some((ppid, pgid))
}

fn current_lineage() -> Result<HashSet<i32>> {
    let mut excluded = HashSet::new();
    let mut pid = std::process::id() as i32;
    loop {
        if pid <= 0 || !excluded.insert(pid) {
            break;
        }
        let stat_path = PathBuf::from(format!("/proc/{pid}/stat"));
        let Some((ppid, _pgid)) = read_stat_ids(&stat_path) else {
            break;
        };
        pid = ppid;
    }
    Ok(excluded)
}

fn process_belongs_to_project(entry: &ProcEntry, project_root: &Path) -> bool {
    if entry
        .cwd
        .as_ref()
        .is_some_and(|cwd| cwd.starts_with(project_root))
    {
        return true;
    }
    let root = project_root.to_string_lossy();
    entry.cmdline.iter().any(|arg| arg.contains(root.as_ref()))
}

fn classify_process(cmdline: &[String], scope: &ProcessScope) -> Option<String> {
    if matches_direct_apxm_job(cmdline) {
        return Some("apxm-job".to_string());
    }
    if matches_dekk_apxm_job(cmdline) {
        return Some("dekk-apxm-job".to_string());
    }
    if scope.include_services && matches_apxm_service(cmdline) {
        return Some("apxm-service".to_string());
    }
    if scope.include_vllm && matches_vllm_process(cmdline) {
        return Some("vllm".to_string());
    }
    None
}

fn matches_direct_apxm_job(cmdline: &[String]) -> bool {
    for idx in 0..cmdline.len() {
        if basename(&cmdline[idx]) == APXM_BINARY && is_apxm_job_command(&cmdline[idx + 1..]) {
            return true;
        }
    }
    false
}

fn matches_dekk_apxm_job(cmdline: &[String]) -> bool {
    for idx in 0..cmdline.len() {
        if basename(&cmdline[idx]) != DEKK_BINARY {
            continue;
        }
        let rest = &cmdline[idx + 1..];
        if rest.first().is_some_and(|arg| arg == APXM_BINARY) && is_apxm_job_command(&rest[1..]) {
            return true;
        }
    }
    false
}

fn is_apxm_job_command(args: &[String]) -> bool {
    let Some(command) = args.first().map(String::as_str) else {
        return false;
    };
    if CORE_JOB_COMMANDS.contains(&command) {
        return true;
    }
    command == WORKFLOW_COMMAND
        && args
            .get(1)
            .is_some_and(|subcommand| subcommand == WORKFLOW_RUN_COMMAND)
}

fn matches_apxm_service(cmdline: &[String]) -> bool {
    cmdline.iter().any(|arg| {
        let name = basename(arg);
        name == APXM_SERVER_BINARY || name == APXM_GUI_BINARY
    }) || cmdline
        .windows(2)
        .any(|window| basename(&window[0]) == APXM_BINARY && window[1].as_str() == "gui")
}

fn matches_vllm_process(cmdline: &[String]) -> bool {
    cmdline
        .iter()
        .any(|arg| arg.contains(VLLM_ENTRYPOINT) || arg.ends_with(VLLM_SCRIPT))
}

fn basename(text: &str) -> &str {
    Path::new(text)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(text)
}

fn emit_process_list(processes: &[ProcessReport], json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(processes)?);
        return Ok(());
    }
    if processes.is_empty() {
        println!("No matching APXM job processes found");
        return Ok(());
    }
    println!("Matching APXM processes:");
    for process in processes {
        println!(
            "  pid={} pgid={} kind={} cwd={} cmd={}",
            process.pid,
            process.pgid,
            process.kind,
            process
                .cwd
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<unknown>".to_string()),
            process.command
        );
    }
    Ok(())
}

fn stop_processes(
    processes: &[ProcessReport],
    dry_run: bool,
    force: bool,
    json: bool,
) -> Result<()> {
    if dry_run {
        return emit_process_list(processes, json);
    }
    if processes.is_empty() {
        if json {
            println!("{{\"stopped\":[],\"failed\":[]}}");
        } else {
            println!("No matching APXM job processes found");
        }
        return Ok(());
    }

    let signal = if force { "KILL" } else { "TERM" };
    let mut stopped = Vec::new();
    let mut failed = Vec::new();
    for process in processes {
        match signal_pid(process.pid, signal) {
            Ok(true) => stopped.push(process.pid),
            Ok(false) => failed.push(process.pid),
            Err(_) => failed.push(process.pid),
        }
    }

    if json {
        println!(
            "{}",
            serde_json::json!({
                "signal": signal,
                "stopped": stopped,
                "failed": failed,
            })
        );
    } else {
        println!(
            "Sent SIG{} to {} APXM process{}",
            signal,
            stopped.len(),
            if stopped.len() == 1 { "" } else { "es" }
        );
        if !failed.is_empty() {
            println!(
                "Failed to signal {} process(es): {:?}",
                failed.len(),
                failed
            );
        }
    }

    if failed.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("failed to signal {} APXM process(es)", failed.len())
    }
}

#[cfg(unix)]
fn signal_pid(pid: i32, signal: &str) -> Result<bool> {
    let status = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .status()
        .with_context(|| format!("failed to invoke kill for pid {pid}"))?;
    Ok(status.success())
}

#[cfg(not(unix))]
fn signal_pid(_pid: i32, _signal: &str) -> Result<bool> {
    anyhow::bail!("process cleanup is currently supported on Unix-like systems")
}
