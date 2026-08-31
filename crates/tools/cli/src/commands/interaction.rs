//! Headless Compilation and Runtime Service clients owned by the command shell.

use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use anyhow::{Result, bail};
use apxm_compilation_client::CompilationClient;
use apxm_interaction_client::{
    CLIENT_RECORD_CONTRACT, ClientInteractionRecord, InteractionClient, ReadAuthorizationTransport,
};
use apxm_interaction_client::{
    ExecutionObservation, ProgramInvocationId, ProgramInvocationInspection, ReadPurpose,
    RuntimeResult,
};

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
    let started = start_instance(agent_package, artifact)?;
    persist_record(
        &started.artifact_digest,
        &started.instance,
        &started.invocation_id,
    )?;
    if force_tui || io::stdout().is_terminal() {
        tui::run_session(started.frame())?;
        return Ok(());
    }
    println!(
        "{}",
        serde_json::json!({
            "status": started.inspection.status,
            "program_instance_id": started.instance,
            "program_invocation_id": started.invocation_id,
            "artifact_digest": started.artifact_digest,
        })
    );
    Ok(())
}

/// `apxm interact` always opens the Interaction Client TUI.
pub fn interact_command(agent_package: Option<PathBuf>, artifact: Option<String>) -> Result<()> {
    let started = start_instance(agent_package, artifact)?;
    persist_record(
        &started.artifact_digest,
        &started.instance,
        &started.invocation_id,
    )?;
    tui::run_session(started.frame())
}

/// Canonical Event client methods.
pub fn event_command(action: EventAction) -> Result<()> {
    let mut runtime = spawn_runtime_client()?;
    match action {
        EventAction::List { owner_claim } => {
            let owner_claim =
                owner_claim.ok_or_else(|| anyhow::anyhow!("event list requires --owner-claim"))?;
            let owner_claim = apxm_interaction_client::RuntimeOwnerClaim { value: owner_claim };
            owner_claim
                .validate()
                .map_err(|error| anyhow::anyhow!("invalid owner claim: {error:?}"))?;
            let result = runtime
                .list_events(owner_claim)
                .map_err(|error| anyhow::anyhow!(error))?;
            println!("{}", serde_json::to_string(&result)?);
            Ok(())
        }
        EventAction::Inspect {
            event_id,
            generation,
            owner_claim,
        } => {
            runtime
                .set_event_owner_claim(apxm_interaction_client::RuntimeOwnerClaim {
                    value: owner_claim,
                })
                .map_err(|error| anyhow::anyhow!(error))?;
            let result = runtime
                .inspect_event(event_id, generation)
                .map_err(|error| anyhow::anyhow!(error))?;
            println!("{}", serde_json::to_string(&result)?);
            Ok(())
        }
        EventAction::Expire {
            event_id,
            generation,
            owner_claim,
        } => {
            runtime
                .set_event_owner_claim(apxm_interaction_client::RuntimeOwnerClaim {
                    value: owner_claim,
                })
                .map_err(|error| anyhow::anyhow!(error))?;
            let result = runtime
                .expire_event(event_id, generation)
                .map_err(|error| anyhow::anyhow!(error))?;
            println!("{}", serde_json::to_string(&result)?);
            Ok(())
        }
        EventAction::Cancel {
            event_id,
            generation,
            owner_claim,
        } => {
            runtime
                .set_event_owner_claim(apxm_interaction_client::RuntimeOwnerClaim {
                    value: owner_claim,
                })
                .map_err(|error| anyhow::anyhow!(error))?;
            let result = runtime
                .cancel_event(event_id, generation)
                .map_err(|error| anyhow::anyhow!(error))?;
            println!("{}", serde_json::to_string(&result)?);
            Ok(())
        }
        EventAction::Fulfill {
            event_id,
            generation,
            idempotency_key,
            occurrence_id,
            source_kind,
            mapping_digest,
            source_record,
            payload,
            owner_claim,
        } => {
            runtime
                .set_event_owner_claim(apxm_interaction_client::RuntimeOwnerClaim {
                    value: owner_claim,
                })
                .map_err(|error| anyhow::anyhow!(error))?;
            runtime
                .fulfill_event(
                    event_id,
                    generation,
                    apxm_interaction_client::EventOccurrence {
                        occurrence_id,
                        source_kind,
                        mapping_digest,
                        source_record,
                        payload: serde_json::from_str(&payload)
                            .map_err(|error| anyhow::anyhow!("invalid JSON payload: {error}"))?,
                    },
                    idempotency_key,
                )
                .map_err(|error| anyhow::anyhow!(error))?;
            Ok(())
        }
    }
}

/// Local Runtime Service supervise/connect. The CLI execs the service binary.
pub fn runtime_command(action: RuntimeAction) -> Result<()> {
    match action {
        RuntimeAction::Serve { socket } => {
            if let Some(path) = &socket {
                require_absolute_socket(path)?;
            }
            let program = runtime_service_bin()?;
            let mut cmd = Command::new(program);
            if let Some(path) = socket {
                cmd.arg("--socket").arg(path);
            }
            let status = cmd
                .status()
                .map_err(|error| anyhow::anyhow!("failed to exec apxm-runtime-service: {error}"))?;
            if !status.success() {
                bail!("apxm-runtime-service exited with {status}");
            }
            Ok(())
        }
        RuntimeAction::Connect { socket } => {
            require_absolute_socket(&socket)?;
            println!("connected");
            Ok(())
        }
    }
}

/// Inspect the invocation referenced by the latest client record.
pub fn resume_command(last: bool) -> Result<()> {
    if !last {
        bail!("apxm resume requires --last");
    }
    let record = load_last_record()?;
    let mut runtime = spawn_runtime_read_client()?;
    let invocation_id = ProgramInvocationId::new(record.program_invocation_id.clone())
        .map_err(|error| anyhow::anyhow!("invalid client invocation reference: {error}"))?;
    let inspection = runtime
        .inspect_invocation(
            caller_read_context("cli.resume", ReadPurpose::Inspection)
                .map_err(|error| anyhow::anyhow!("invalid canonical read context: {error}"))?,
            invocation_id.clone(),
        )
        .map_err(|error| anyhow::anyhow!("canonical invocation inspection unavailable: {error}"))?;
    println!(
        "{}",
        serde_json::json!({
            "contract": record.contract,
            "artifact_digest": record.artifact_digest,
            "program_instance_id": record.program_instance_id,
            "program_invocation_id": invocation_id,
            "inspection": inspection,
        })
    );
    Ok(())
}

struct StartedInteraction {
    artifact_digest: String,
    instance: String,
    invocation_id: ProgramInvocationId,
    inspection: ProgramInvocationInspection,
    observations: Vec<String>,
}

impl StartedInteraction {
    fn frame(self) -> tui::TuiFrame {
        tui::TuiFrame::from_protocol(
            self.instance,
            self.artifact_digest,
            self.inspection.status,
            self.observations,
        )
    }
}

fn start_instance(
    agent_package: Option<PathBuf>,
    artifact: Option<String>,
) -> Result<StartedInteraction> {
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
    let mut runtime = spawn_runtime_read_client()?;
    let instance = runtime
        .run_artifact(&artifact_digest)
        .map_err(|error| anyhow::anyhow!(error))?;
    let started = runtime
        .start_invocation(&instance, serde_json::json!({}))
        .map_err(|error| anyhow::anyhow!(error))?;
    let RuntimeResult::ProgramInvocationStarted {
        program_invocation_id,
        ..
    } = started
    else {
        return Err(anyhow::anyhow!(
            "runtime did not start a Program Invocation"
        ));
    };
    let invocation_id = ProgramInvocationId::new(program_invocation_id).map_err(|error| {
        anyhow::anyhow!("runtime returned an invalid invocation reference: {error}")
    })?;
    let inspection = runtime
        .inspect_invocation(
            caller_read_context("cli.run.inspect", ReadPurpose::Inspection)
                .map_err(|error| anyhow::anyhow!("invalid canonical read context: {error}"))?,
            invocation_id.clone(),
        )
        .map_err(|error| anyhow::anyhow!("canonical invocation inspection unavailable: {error}"))?;
    let observations = runtime
        .subscribe_observations(
            caller_read_context("cli.run.observations", ReadPurpose::Observation)
                .map_err(|error| anyhow::anyhow!("invalid canonical read context: {error}"))?,
            invocation_id.clone(),
            None,
            100,
        )
        .map(|page| observation_lines(&page.items))
        .map_err(|error| anyhow::anyhow!("canonical observation stream unavailable: {error}"))?;
    Ok(StartedInteraction {
        artifact_digest,
        instance,
        invocation_id,
        inspection,
        observations,
    })
}

fn observation_lines(observations: &[ExecutionObservation]) -> Vec<String> {
    observations
        .iter()
        .map(|observation| {
            let kind = serde_json::to_string(&observation.observation_kind)
                .unwrap_or_else(|_| "unknown".to_owned());
            format!("{} #{}", kind.trim_matches('"'), observation.sequence)
        })
        .collect()
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

fn spawn_runtime_read_client() -> Result<InteractionClient> {
    let dir = artifact_dir()?;
    let (program, args) = runtime_child()?;
    let authorization = ReadAuthorizationTransport::from_env()
        .map_err(|error| anyhow::anyhow!("runtime read authorization: {error}"))?;
    InteractionClient::spawn_stdio_with_read_authorization(program, &args, dir, Some(authorization))
        .map_err(|error| anyhow::anyhow!(error))
}

fn caller_read_context(
    request_id: &str,
    purpose: ReadPurpose,
) -> Result<apxm_interaction_client::ReadContext, String> {
    let authorization = ReadAuthorizationTransport::from_env()?;
    InteractionClient::read_context_with_correlation(
        request_id,
        authorization.scope_ref.as_str(),
        authorization.principal_ref.as_str(),
        authorization.grant_ref.as_str(),
        authorization
            .correlation_id
            .as_ref()
            .map(|value| value.as_str()),
        purpose,
    )
}

fn compilation_child() -> Result<(PathBuf, Vec<&'static str>)> {
    Ok((compilation_service_bin()?, compilation_child_args()))
}

fn runtime_child() -> Result<(PathBuf, Vec<&'static str>)> {
    Ok((runtime_service_bin()?, runtime_child_args()))
}

fn compilation_child_args() -> Vec<&'static str> {
    Vec::new()
}

fn runtime_child_args() -> Vec<&'static str> {
    Vec::new()
}

fn compilation_service_bin() -> Result<PathBuf> {
    resolve_service_bin("APXM_COMPILATION_SERVICE_BIN", "apxm-compilation-service")
}

fn runtime_service_bin() -> Result<PathBuf> {
    resolve_service_bin("APXM_RUNTIME_SERVICE_BIN", "apxm-runtime-service")
}

fn resolve_service_bin(env_key: &str, name: &str) -> Result<PathBuf> {
    if let Some(bin) = env_bin(env_key) {
        return Ok(bin);
    }
    if let Some(bin) = sibling_bin(name) {
        return Ok(bin);
    }
    bail!("{name} is not on PATH or next to apxm; build -p {name}")
}

fn require_absolute_socket(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        bail!("unix socket path must be absolute");
    }
    Ok(())
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

fn artifact_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("APXM_ARTIFACT_DIR")
        && !dir.trim().is_empty()
    {
        let path = PathBuf::from(dir);
        fs::create_dir_all(&path)?;
        return Ok(path);
    }
    let path = std::env::current_dir()?.join(".apxm/artifacts");
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn client_dir() -> PathBuf {
    PathBuf::from(".apxm/client")
}

fn persist_record(
    artifact_digest: &str,
    program_instance_id: &str,
    program_invocation_id: &ProgramInvocationId,
) -> Result<()> {
    let dir = client_dir();
    fs::create_dir_all(&dir)?;
    let record = ClientInteractionRecord {
        contract: CLIENT_RECORD_CONTRACT.to_owned(),
        artifact_digest: artifact_digest.to_owned(),
        program_instance_id: Some(program_instance_id.to_owned()),
        program_invocation_id: program_invocation_id.as_str().to_owned(),
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
        assert!(
            !runtime_child_args()
                .iter()
                .any(|arg| arg.contains("compilation")),
            "the Runtime child argv must not start Compilation"
        );
    }
}
