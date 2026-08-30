//! CLI type definitions for APXM.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "apxm")]
#[command(about = "APxM CLI - canonical AIR authoring, compilation, and execution", long_about = None)]
pub struct Cli {
    /// Optional config path (defaults to .apxm/config.toml or ~/.apxm/config.toml)
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Enable runtime tracing (levels: trace, debug, info, warn, error)
    #[arg(long, global = true)]
    pub trace: Option<String>,

    /// Output in JSON format (machine-readable)
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Compile a package snapshot to a committed executable artifact.
    Build {
        /// Agent package directory.
        agent_package: PathBuf,
    },
    /// Headless Runtime Service client. Source packages build first.
    Run {
        /// Agent package directory, unless `--artifact` is set.
        agent_package: Option<PathBuf>,
        /// Explicit artifact digest. Does not contact Compilation Service.
        #[arg(long)]
        artifact: Option<String>,
        /// Force the TUI even when stdout is not a TTY.
        #[arg(long)]
        tui: bool,
    },
    /// Canonical Event ingress client.
    Event {
        #[command(subcommand)]
        action: EventAction,
    },
    /// Supervise or connect to a Runtime Service.
    Runtime {
        #[command(subcommand)]
        action: RuntimeAction,
    },
    /// Reopen a client interaction record against runtime truth.
    Resume {
        /// Resume the most recent client record.
        #[arg(long)]
        last: bool,
    },
    /// Diagnose environment and package-contract readiness.
    Doctor,
    /// Lint, sync, verify, and install agents.
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Scaffold, lint, and install organization packages.
    Org {
        #[command(subcommand)]
        action: OrgAction,
    },
    /// Open the Interaction Client TUI after a committed artifact.
    Interact {
        /// Agent package directory, unless `--artifact` is set.
        agent_package: Option<PathBuf>,
        /// Explicit artifact digest. Does not contact Compilation Service.
        #[arg(long)]
        artifact: Option<String>,
    },
    /// Inspect or stop Compilation and Runtime Service children.
    Process {
        #[command(subcommand)]
        action: ProcessAction,
    },
}

#[derive(Subcommand)]
pub enum EventAction {
    /// Inspect one EventRef.
    Inspect {
        /// Event id.
        event_id: String,
        /// Ownership generation.
        #[arg(long)]
        generation: u64,
        /// Exact reservation claim returned by `event reserve`.
        #[arg(long)]
        owner_claim: String,
    },
    /// List authorized pending EventRefs.
    List {
        /// Exact reservation claim returned by `event reserve`.
        #[arg(long)]
        owner_claim: Option<String>,
    },
    /// Fulfill one EventRef with an explicit source occurrence and JSON payload.
    Fulfill {
        /// Event id.
        event_id: String,
        /// Ownership generation.
        #[arg(long)]
        generation: u64,
        /// Idempotency key.
        #[arg(long)]
        idempotency_key: String,
        /// Source occurrence identity, distinct from the EventRef.
        #[arg(long)]
        occurrence_id: String,
        /// Source contract kind that produced the occurrence.
        #[arg(long)]
        source_kind: String,
        /// Digest of the admitted source-contract mapping.
        #[arg(long)]
        mapping_digest: String,
        /// Source-stable record or sequence identity.
        #[arg(long)]
        source_record: String,
        /// JSON payload reduced by the source contract.
        #[arg(long)]
        payload: String,
        /// Exact reservation claim returned by `event reserve`.
        #[arg(long)]
        owner_claim: String,
    },
    /// Expire one EventRef.
    Expire {
        /// Event id.
        event_id: String,
        /// Ownership generation.
        #[arg(long)]
        generation: u64,
        /// Exact reservation claim returned by `event reserve`.
        #[arg(long)]
        owner_claim: String,
    },
    /// Cancel one EventRef.
    Cancel {
        /// Event id.
        event_id: String,
        /// Ownership generation.
        #[arg(long)]
        generation: u64,
        /// Exact reservation claim returned by `event reserve`.
        #[arg(long)]
        owner_claim: String,
    },
}

#[derive(Subcommand)]
pub enum RuntimeAction {
    /// Supervise a local Runtime Service.
    Serve {
        /// Optional Unix socket path.
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Connect to an existing Runtime Service.
    Connect {
        /// Unix socket path.
        #[arg(long)]
        socket: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum ProcessAction {
    /// List APXM job processes visible from this worktree
    List {},
    /// Stop canonical APXM CLI job processes
    Stop {
        /// Show matching processes without sending a signal
        #[arg(long)]
        dry_run: bool,
        /// Send SIGKILL instead of SIGTERM
        #[arg(long, short)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum AgentAction {
    /// Scaffold a new agent folder tree (apxm.agent).
    New {
        /// Agent id.
        id: String,
        /// Destination directory (default: ./agents/<id>).
        #[arg(long)]
        path: Option<PathBuf>,
        /// Display name for the agent (default: derived from the id).
        #[arg(long)]
        display_name: Option<String>,
        /// Scaffold template (`looped-agent`, an examples/agents name, or a directory path).
        #[arg(long, default_value = "looped-agent")]
        template: String,
    },
    /// Recompile the package's TypeScript handlers into capabilities/handlers/tools.json.
    Sync {
        /// Agent directory to sync (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Validate an agent folder against the apxm.agent contract: manifest
    /// shape, the compiled source declaration, the permission layer stack,
    /// and the published folder contract.
    Lint {
        /// Agent directory to validate (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// An org package directory whose `org.toml [permissions]` keys are
        /// this agent's org-global capability set. Those ids extend what this
        /// package can supply, so `agent.toml [permissions]` may state a
        /// decision for a capability the org provides rather than one this
        /// package ships itself.
        #[arg(long)]
        org: Option<PathBuf>,
    },
    /// Install a built agent to `APXM_HOME/agents/<id>/`.
    Install {
        /// Agent directory to install (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Overwrite an existing install at the destination.
        #[arg(long)]
        force: bool,
    },
    /// Verify a built agent folder against its generated integrity chain.
    Verify {
        /// Agent directory to verify (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum OrgAction {
    /// Scaffold a new organization-package folder tree (apxm.org).
    New {
        /// Org id (also used as org.toml's org_id).
        id: String,
        /// Destination directory (default: ./orgs/<id>).
        #[arg(long)]
        path: Option<PathBuf>,
        /// Display name for the org (default: derived from the id).
        #[arg(long)]
        display_name: Option<String>,
    },
    /// Validate an org package: member resolution, tree well-formedness,
    /// member-hierarchy/topology consistency, and capability-mask validity
    /// against `org.toml [permissions]`.
    Lint {
        /// Org-package directory to validate (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Install an org package to `APXM_HOME/orgs/<id>/`.
    Install {
        /// Org-package directory to install (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Overwrite an existing install at the destination.
        #[arg(long)]
        force: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn production_cli_rejects_agent_build() {
        assert!(Cli::try_parse_from(["apxm", "agent", "build", "."]).is_err());
    }

    #[test]
    fn production_cli_rejects_compile_service_canonical() {
        assert!(Cli::try_parse_from(["apxm", "compile-service-canonical", "/tmp/agent"]).is_err());
    }

    #[test]
    fn production_cli_rejects_execute_canonical() {
        assert!(
            Cli::try_parse_from([
                "apxm",
                "execute-canonical",
                "/tmp/program.air",
                "--invocation-admission",
                "/tmp/admission.json",
                "--release",
                "/tmp/release.json",
                "--provenance",
                "/tmp/provenance.json",
            ])
            .is_err()
        );
    }
}
