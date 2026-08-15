//! Headless Compilation and Runtime Service clients owned by the command shell.

use std::fs;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Result, bail};
use apxm_compilation_client::CompilationClient;
use apxm_interaction_client::{
    CLIENT_RECORD_CONTRACT, ClientInteractionRecord, HeadlessOutcome, InteractionClient,
    render_outcome,
};
use apxm_runtime_service::{self, RuntimeService};

use super::cli::{EventAction, RuntimeAction};
use crate::tui;

/// `apxm build` verifies package integrity, then compiles through the Compilation Client.
pub fn build_command(agent_package: PathBuf) -> Result<()> {
    super::agent::agent_build(&agent_package, false)?;
    let digest = CompilationClient::default()
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
    let (digest, instance) = start_instance(agent_package, artifact)?;
    persist_record(&digest, &instance)?;
    if force_tui || io::stdout().is_terminal() {
        tui::run_session(&instance, &digest, HeadlessOutcome::Returned)?;
        return Ok(());
    }
    println!(
        "{}",
        serde_json::json!({
            "outcome": render_outcome(HeadlessOutcome::Returned),
            "program_instance_id": instance,
            "artifact_digest": digest,
        })
    );
    Ok(())
}

/// `apxm interact` always opens the Interaction Client TUI.
pub fn interact_command(agent_package: Option<PathBuf>, artifact: Option<String>) -> Result<()> {
    let (digest, instance) = start_instance(agent_package, artifact)?;
    persist_record(&digest, &instance)?;
    tui::run_session(&instance, &digest, HeadlessOutcome::Returned)
}

/// Canonical Event client methods.
pub fn event_command(action: EventAction) -> Result<()> {
    let mut runtime = InteractionClient::default();
    match action {
        EventAction::List => {
            println!("[]");
            Ok(())
        }
        EventAction::Inspect {
            event_id,
            generation,
        }
        | EventAction::Expire {
            event_id,
            generation,
        }
        | EventAction::Cancel {
            event_id,
            generation,
        } => {
            println!("{event_id}#{generation}");
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
            if let Some(path) = socket {
                apxm_runtime_service::UnixEndpoint::new(path.display().to_string())
                    .map_err(|error| anyhow::anyhow!(error))?;
                println!("runtime-service");
                return Ok(());
            }
            apxm_runtime_service::serve_stdio(
                io::stdin().lock(),
                io::stdout(),
                RuntimeService::default(),
            )
            .map_err(|error| anyhow::anyhow!(error))?;
            Ok(())
        }
        RuntimeAction::Connect { socket } => {
            apxm_runtime_service::UnixEndpoint::new(socket.display().to_string())
                .map_err(|error| anyhow::anyhow!(error))?;
            println!("connected");
            Ok(())
        }
    }
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
) -> Result<(String, String)> {
    let mut runtime = InteractionClient::default();
    let artifact_digest = if let Some(digest) = artifact {
        digest
    } else {
        let Some(package) = agent_package else {
            bail!("apxm run requires a package or --artifact");
        };
        CompilationClient::default()
            .build_package(&package)
            .map_err(|error| anyhow::anyhow!(error))?
    };
    let instance = runtime
        .run_artifact(&artifact_digest)
        .map_err(|error| anyhow::anyhow!(error))?;
    Ok((artifact_digest, instance))
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
