//! Headless Compilation and Runtime Service clients owned by the command shell.

use anyhow::{Result, bail};
use apxm_compilation_client::CompilationClient;
use apxm_interaction_client::{
    CLIENT_RECORD_CONTRACT, ClientInteractionRecord, HeadlessOutcome, InteractionClient,
    render_outcome,
};
use apxm_kernel::event_api::CanonicalEventRef;
use apxm_source_port::{Frontend, PACKAGE_SNAPSHOT_CONTRACT, PackageSnapshot, SnapshotContent};

use super::cli::{EventAction, RuntimeAction};

/// `apxm build` over the Compilation Client.
pub fn build_command(agent_package: std::path::PathBuf) -> Result<()> {
    let mut client = CompilationClient::default();
    let digest = client
        .build(snapshot_for(&agent_package, Frontend::Python))
        .map_err(|error| anyhow::anyhow!(error))?;
    println!("{digest}");
    Ok(())
}

/// `apxm run` over the Interaction Client. `--artifact` never compiles.
pub fn run_command(
    agent_package: Option<std::path::PathBuf>,
    artifact: Option<String>,
) -> Result<()> {
    let mut runtime = InteractionClient::default();
    let artifact_digest = if let Some(digest) = artifact {
        digest
    } else {
        let Some(package) = agent_package else {
            bail!("apxm run requires a package or --artifact");
        };
        CompilationClient::default()
            .build(snapshot_for(&package, Frontend::Python))
            .map_err(|error| anyhow::anyhow!(error))?
    };
    let instance = runtime
        .run_artifact(&artifact_digest)
        .map_err(|error| anyhow::anyhow!(error))?;
    println!(
        "{}",
        serde_json::json!({
            "outcome": render_outcome(HeadlessOutcome::Returned),
            "program_instance_id": instance,
            "artifact_digest": artifact_digest,
        })
    );
    Ok(())
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
                .fulfill_event(
                    CanonicalEventRef {
                        event_id,
                        generation,
                    },
                    serde_json::json!({}),
                    idempotency_key,
                )
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
            }
            println!("runtime-service");
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
    let record = ClientInteractionRecord {
        contract: CLIENT_RECORD_CONTRACT.to_owned(),
        artifact_digest: "artifact:none".to_owned(),
        program_instance_id: last.then(|| "last".to_owned()),
    };
    println!("{}", serde_json::to_string(&record)?);
    Ok(())
}

fn snapshot_for(path: &std::path::Path, frontend: Frontend) -> PackageSnapshot {
    PackageSnapshot {
        contract: PACKAGE_SNAPSHOT_CONTRACT.to_owned(),
        frontend,
        entrypoint: "agent".to_owned(),
        contents: vec![SnapshotContent {
            path: path.display().to_string(),
            digest: "local".to_owned(),
        }],
        dependency_lock_digest: None,
        compatibility_set: "apxm.compatibility-set/local".to_owned(),
        snapshot_digest: path.display().to_string(),
    }
}
