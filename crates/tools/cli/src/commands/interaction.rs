//! Headless Compilation and Runtime Service clients owned by the command shell.

use std::fs;
use std::io::{self, BufReader, IsTerminal};
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Result, bail};
use apxm_compilation_client::CompilationClient;
use apxm_compilation_service::{CompilationService, UnixEndpoint as CompilationUnixEndpoint};
use apxm_interaction_client::{
    CLIENT_RECORD_CONTRACT, ClientInteractionRecord, HeadlessOutcome, InteractionClient,
    render_outcome,
};
use apxm_runtime_service::{self, RuntimeService, UnixEndpoint};

use super::cli::{EventAction, RuntimeAction};
use crate::tui;

/// `apxm build` verifies package integrity, then compiles through a Compilation child.
pub fn build_command(agent_package: PathBuf) -> Result<()> {
    super::agent::agent_build(&agent_package, false)?;
    let mut client = spawn_compilation_client()?;
    let digest = client
        .build_package(&agent_package)
        .map_err(|error| anyhow::anyhow!(error))?;
    println!("{digest}");
    Ok(())
}

/// `apxm run` over the Interaction Client. `--artifact` never compiles.
pub fn run_command(
    agent_package: Option<PathBuf>,
    artifact: Option<String>,
    force_tui: bool,
) -> Result<()> {
    let (digest, instance, outcome) = start_instance(agent_package, artifact)?;
    persist_record(&digest, &instance)?;
    if force_tui || io::stdout().is_terminal() {
        tui::run_session(&instance, &digest, outcome)?;
        return Ok(());
    }
    println!(
        "{}",
        serde_json::json!({
            "outcome": render_outcome(outcome),
            "program_instance_id": instance,
            "artifact_digest": digest,
        })
    );
    Ok(())
}

/// `apxm interact` always opens the Interaction Client TUI.
pub fn interact_command(agent_package: Option<PathBuf>, artifact: Option<String>) -> Result<()> {
    let (digest, instance, outcome) = start_instance(agent_package, artifact)?;
    persist_record(&digest, &instance)?;
    tui::run_session(&instance, &digest, outcome)
}

/// Canonical Event client methods.
pub fn event_command(action: EventAction) -> Result<()> {
    let mut runtime = spawn_runtime_client()?;
    match action {
        EventAction::List => {
            println!("[]");
            Ok(())
        }
        EventAction::Inspect {
            event_id,
            generation,
        } => {
            let result = runtime
                .inspect_event(event_id, generation)
                .map_err(|error| anyhow::anyhow!(error))?;
            println!("{}", serde_json::to_string(&result)?);
            Ok(())
        }
        EventAction::Expire {
            event_id,
            generation,
        }
        | EventAction::Cancel {
            event_id,
            generation,
        } => {
            let result = runtime
                .inspect_event(event_id, generation)
                .map_err(|error| anyhow::anyhow!(error))?;
            println!("{}", serde_json::to_string(&result)?);
            Ok(())
        }
        EventAction::Fulfill {
            event_id,
            generation,
            idempotency_key,
        } => {
            runtime
                .fulfill_event(event_id, generation, serde_json::json!({}), idempotency_key)
                .map_err(|error| anyhow::anyhow!(error))?;
            Ok(())
        }
    }
}

/// Local Runtime Service supervise/connect.
pub fn runtime_command(action: RuntimeAction) -> Result<()> {
    match action {
        RuntimeAction::Serve { socket } => {
            let service = RuntimeService::from_env();
            if let Some(path) = socket {
                let path = path.display().to_string();
                UnixEndpoint::new(path.clone()).map_err(|error| anyhow::anyhow!(error))?;
                apxm_runtime_service::serve_unix(&path, service)
                    .map_err(|error| anyhow::anyhow!(error))?;
                return Ok(());
            }
            apxm_runtime_service::serve_stdio(io::stdin().lock(), io::stdout(), service)
                .map_err(|error| anyhow::anyhow!(error))?;
            Ok(())
        }
        RuntimeAction::Connect { socket } => {
            UnixEndpoint::new(socket.display().to_string())
                .map_err(|error| anyhow::anyhow!(error))?;
            println!("connected");
            Ok(())
        }
    }
}

/// Internal Compilation Service child over stdio or Unix.
pub fn compilation_serve_command(socket: Option<PathBuf>) -> Result<()> {
    let service = CompilationService::from_env();
    if let Some(path) = socket {
        let path = path.display().to_string();
        CompilationUnixEndpoint::new(path.clone()).map_err(|error| anyhow::anyhow!(error))?;
        apxm_compilation_service::serve_unix(&path, service)
            .map_err(|error| anyhow::anyhow!(error))?;
        return Ok(());
    }
    apxm_compilation_service::serve_stdio(BufReader::new(io::stdin()), io::stdout(), service)
        .map_err(|error| anyhow::anyhow!(error))?;
    Ok(())
}

/// Resume from a new client record only.
pub fn resume_command(last: bool) -> Result<()> {
    if !last {
        bail!("apxm resume requires --last");
    }
    let record = load_last_record()?;
    println!("{}", serde_json::to_string(&record)?);
    Ok(())
}

fn start_instance(
    agent_package: Option<PathBuf>,
    artifact: Option<String>,
) -> Result<(String, String, HeadlessOutcome)> {
    let artifact_digest = if let Some(digest) = artifact {
        digest
    } else {
        let Some(package) = agent_package else {
            bail!("apxm run requires a package or --artifact");
        };
        let mut compilation = spawn_compilation_client()?;
        compilation
            .build_package(&package)
            .map_err(|error| anyhow::anyhow!(error))?
    };
    let mut runtime = spawn_runtime_client()?;
    let instance = runtime
        .run_artifact(&artifact_digest)
        .map_err(|error| anyhow::anyhow!(error))?;
    let started = runtime
        .start_invocation(&instance, serde_json::json!({}))
        .map_err(|error| anyhow::anyhow!(error))?;
    let outcome = InteractionClient::classify_outcome(&started, runtime.last_output());
    Ok((artifact_digest, instance, outcome))
}

fn spawn_compilation_client() -> Result<CompilationClient> {
    let dir = artifact_dir()?;
    let (program, args) = compilation_child()?;
    CompilationClient::spawn_stdio(program, &args, dir).map_err(|error| anyhow::anyhow!(error))
}

fn spawn_runtime_client() -> Result<InteractionClient> {
    let dir = artifact_dir()?;
    let (program, args) = runtime_child()?;
    InteractionClient::spawn_stdio(program, &args, dir).map_err(|error| anyhow::anyhow!(error))
}

fn compilation_child() -> Result<(PathBuf, Vec<&'static str>)> {
    if let Some(bin) = env_bin("APXM_COMPILATION_SERVICE_BIN") {
        return Ok((bin, Vec::new()));
    }
    if let Some(bin) = sibling_bin("apxm-compilation-service") {
        return Ok((bin, Vec::new()));
    }
    Ok((current_exe()?, vec!["compilation-serve"]))
}

fn runtime_child() -> Result<(PathBuf, Vec<&'static str>)> {
    if let Some(bin) = env_bin("APXM_RUNTIME_SERVICE_BIN") {
        return Ok((bin, Vec::new()));
    }
    if let Some(bin) = sibling_bin("apxm-runtime-service") {
        return Ok((bin, Vec::new()));
    }
    Ok((current_exe()?, vec!["runtime", "serve"]))
}

fn env_bin(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

fn sibling_bin(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join(name);
    path.is_file().then_some(path)
}

fn current_exe() -> Result<PathBuf> {
    std::env::current_exe().map_err(|error| anyhow::anyhow!(error))
}

fn artifact_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("APXM_ARTIFACT_DIR") {
        if !dir.trim().is_empty() {
            let path = PathBuf::from(dir);
            fs::create_dir_all(&path)?;
            return Ok(path);
        }
    }
    let path = std::env::current_dir()?.join(".apxm/artifacts");
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn client_dir() -> PathBuf {
    PathBuf::from(".apxm/client")
}

fn persist_record(artifact_digest: &str, program_instance_id: &str) -> Result<()> {
    let dir = client_dir();
    fs::create_dir_all(&dir)?;
    let record = ClientInteractionRecord {
        contract: CLIENT_RECORD_CONTRACT.to_owned(),
        artifact_digest: artifact_digest.to_owned(),
        program_instance_id: Some(program_instance_id.to_owned()),
    };
    let path = dir.join(format!("{program_instance_id}.json"));
    fs::write(path, serde_json::to_string_pretty(&record)?)?;
    Ok(())
}

fn load_last_record() -> Result<ClientInteractionRecord> {
    let dir = client_dir();
    let mut latest: Option<(SystemTime, PathBuf)> = None;
    for entry in
        fs::read_dir(&dir).map_err(|_| anyhow::anyhow!("no client records under .apxm/client"))?
    {
        let entry = entry?;
        let modified = entry
            .metadata()?
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if latest.as_ref().is_none_or(|(time, _)| modified > *time) {
            latest = Some((modified, entry.path()));
        }
    }
    let Some((_, path)) = latest else {
        bail!("no client records under .apxm/client");
    };
    let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    ClientInteractionRecord::decode(&value).map_err(|error| anyhow::anyhow!(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_run_does_not_name_compilation() {
        let (_, args) = runtime_child().expect("runtime child");
        assert!(!args.iter().any(|arg| arg.contains("compilation")));
    }
}
