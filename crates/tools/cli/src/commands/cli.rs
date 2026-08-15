//! CLI type definitions for APXM.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use super::implementations::parse_header;

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

#[derive(Parser)]
#[command(name = "apxm-dev")]
#[command(about = "Dekk-owned compiler and runtime conformance binary", long_about = None)]
pub struct DevCli {
    /// Optional config path
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// Output in JSON format
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: DevCommands,
}

#[derive(Subcommand)]
pub enum DevCommands {
    /// Compile a package to canonical AIR JSON (conformance fixture).
    CompileServiceCanonical {
        /// Agent directory
        agent_dir: PathBuf,
    },
    /// Execute canonical AIR through the Runtime Service fixture path.
    ExecuteCanonical {
        /// Canonical AIR JSON file.
        input: PathBuf,
        /// Invocation Admission JSON.
        #[arg(long, value_name = "PATH")]
        invocation_admission: PathBuf,
        /// Release bytes.
        #[arg(long, value_name = "PATH")]
        release: PathBuf,
        /// Provenance bytes.
        #[arg(long, value_name = "PATH")]
        provenance: PathBuf,
        /// Optional package root for handlers/skills.
        #[arg(long, value_name = "DIR")]
        package: Option<PathBuf>,
    },
    /// Generate frontend assets from Rust-owned registries.
    Codegen {
        #[command(subcommand)]
        action: CodegenAction,
    },
    /// Lower a FrontendGraph document to canonical AIR.
    CanonicalAir {
        /// Canonical frontend graph JSON file. Omit to read stdin.
        input: Option<PathBuf>,
    },
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
    /// Diagnose compiler/runtime dependencies
    Doctor,
    /// Manage registered inference backend endpoints
    Backend {
        #[command(subcommand)]
        action: BackendAction,
    },
    /// Manage agent teams from ~/.apxm/teams.toml
    Team {
        #[command(subcommand)]
        action: TeamAction,
    },
    /// Scaffold, sync, lint, build, and install agents.
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Scaffold, lint, and install organization packages.
    Org {
        #[command(subcommand)]
        action: OrgAction,
    },
    /// Browse AIS operations (the agent instruction set)
    Ops {
        #[command(subcommand)]
        action: OpsAction,
    },
    /// Validate canonical AIR source against the AIS contract
    Validate {
        /// Input workflow source (.air)
        input: PathBuf,
        /// Skip Tier 2 environment checks (registered backends, profiles, etc.)
        #[arg(long)]
        no_check_resources: bool,
    },
    /// Analyze an AIR workflow for parallelism, critical path, and execution phases
    Analyze {
        /// Input workflow source (.air)
        input: PathBuf,
    },
    /// Browse workflow templates (starter patterns)
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },
    /// Explain what a workflow does OR explain an error code
    Explain {
        /// Error code (e.g., E511) or path to workflow source (.air)
        target: String,
    },
    /// Open the Interaction Client TUI after a committed artifact.
    Interact {
        /// Agent package directory, unless `--artifact` is set.
        agent_package: Option<PathBuf>,
        /// Explicit artifact digest. Does not contact Compilation Service.
        #[arg(long)]
        artifact: Option<String>,
    },
    /// Manage and inspect execution sessions
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Inspect or stop APXM job processes
    Process {
        #[command(subcommand)]
        action: ProcessAction,
    },
    /// Manage the MemoCache (response memoization cache)
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Report standalone token-accounting availability for text
    Tokenize {
        /// Text to tokenize. Use --file for larger inputs.
        text: Option<String>,
        /// Read text from a file instead of the positional argument.
        #[arg(long, conflicts_with = "text")]
        file: Option<PathBuf>,
        /// Model name to echo in the diagnostic output.
        #[arg(long)]
        model: Option<String>,
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
    },
    /// List authorized pending EventRefs.
    List,
    /// Fulfill one EventRef from JSON/file/stdin.
    Fulfill {
        /// Event id.
        event_id: String,
        /// Ownership generation.
        #[arg(long)]
        generation: u64,
        /// Idempotency key.
        #[arg(long)]
        idempotency_key: String,
    },
    /// Expire one EventRef.
    Expire {
        /// Event id.
        event_id: String,
        /// Ownership generation.
        #[arg(long)]
        generation: u64,
    },
    /// Cancel one EventRef.
    Cancel {
        /// Event id.
        event_id: String,
        /// Ownership generation.
        #[arg(long)]
        generation: u64,
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
pub enum TemplateAction {
    /// List available workflow templates
    List,
    /// Show a specific template
    Show {
        /// Template name (e.g., ask, pipeline, fan-out, map-reduce)
        name: String,
    },
}

#[derive(Subcommand)]
pub enum CodegenAction {
    /// Generate the Python frontend bindings into apxm_program/_generated
    Frontend {
        /// Check that the generated Python files are up to date without writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate TypeScript types into the TypeScript authoring frontend
    Typescript {
        /// Output file path for generated TypeScript
        #[arg(long)]
        output: Option<PathBuf>,
        /// Check that the output is up to date without writing it
        #[arg(long)]
        check: bool,
    },
    /// Generate @apxm/frontend TypeScript metadata into src/generated
    TypescriptFrontend {
        /// Output directory for generated TypeScript frontend files
        #[arg(long)]
        output_dir: Option<PathBuf>,
        /// Check that the generated files are up to date without writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate TypeScript event kind constants into the Studio frontend
    EventKinds {
        /// Output file path for generated TypeScript event kinds
        #[arg(long)]
        output: Option<PathBuf>,
        /// Check that the output is up to date without writing it
        #[arg(long)]
        check: bool,
    },
    /// Generate the capability id catalogue into both authoring frontends
    Capabilities {
        /// Check that the generated catalogue files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the permission decision vocabulary into both authoring frontends
    Permissions {
        /// Check that the generated decision files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the source graph vocabulary into both authoring frontends
    FrontendVocabulary {
        /// Check that the generated vocabulary files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the FrontendGraph contract record types into both authoring
    /// frontends
    FrontendRecords {
        /// Check that the generated record files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the FrontendGraph contract record serializers into both
    /// authoring frontends
    FrontendSerializers {
        /// Check that the generated serializer files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the shared frontend-conformance harness into every authoring
    /// frontend and its test runner
    FrontendConformance {
        /// Check that the generated harness files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the authoring diagnostic codes into both authoring frontends
    Diagnostics {
        /// Check that the generated diagnostic files are up to date without
        /// writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the reference tables the authoring documentation carries
    Docs {
        /// Check that the generated documentation regions are up to date
        /// without writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate the op-spec AIS operation catalog + vectors fixture
    OpSpec {
        /// Output directory for the generated catalog + vectors files
        #[arg(long)]
        output_dir: Option<PathBuf>,
        /// Check that the generated files are up to date without writing them
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
pub enum SessionAction {
    /// List all sessions
    List {
        /// Filter by status (running/completed/failed)
        #[arg(long)]
        status: Option<String>,
        /// Limit number of sessions shown
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Explicit sessions root to inspect instead of local/global discovery
        #[arg(long)]
        session_root: Option<PathBuf>,
    },
    /// Show detailed information about a session
    Inspect {
        /// Session ID or path
        session: String,
    },
    /// Compare two sessions
    Diff {
        /// First session ID or path
        session1: String,
        /// Second session ID or path
        session2: String,
    },
    /// Clean aged sessions
    Clean {
        /// Remove sessions older than this duration (e.g., "7d", "30d", "1h")
        #[arg(long)]
        older_than: Option<String>,
        /// Remove all sessions
        #[arg(long)]
        all: bool,
        /// Dry run (show what would be deleted)
        #[arg(long)]
        dry_run: bool,
        /// Explicit sessions root to clean instead of local/global discovery
        #[arg(long)]
        session_root: Option<PathBuf>,
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
pub enum CacheAction {
    /// Show cache statistics (hit/miss rates, size, entries)
    Stats,
    /// Clear all cached entries
    Clear {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// Export cache to JSON
    Export {
        /// Output file (defaults to stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum OpsAction {
    /// List all AIS operations
    List {
        /// Filter by category (e.g., reasoning, memory, tools, control_flow)
        #[arg(long)]
        category: Option<String>,
    },
    /// Show detailed info for a specific operation
    Show {
        /// Operation name (e.g., ASK, THINK, INV, FLOW_CALL)
        name: String,
    },
    /// Show accumulated op-usage counts from real executions
    ///
    /// Reports how many times each AIS operation has actually been
    /// dispatched by `apxm execute-canonical`, so drift between "ops
    /// defined" (`apxm ops list`) and "ops actually used" is visible without
    /// a one-off corpus measurement.
    Usage,
}

#[derive(Subcommand)]
pub enum BackendAction {
    /// List all registered backends
    List {
        /// Output format (table or json)
        #[arg(long, default_value = "table")]
        format: String,
    },
    /// Register a new backend endpoint
    Add {
        /// Backend name (for example, "openai").
        name: String,
        /// Backend type (cloud, onprem, local). For Ollama, defaults to "local".
        #[arg(long, default_value = "")]
        r#type: String,
        /// Protocol name (openai, anthropic, google, or ollama).
        #[arg(long)]
        protocol: String,
        /// API endpoint URL
        #[arg(long)]
        endpoint: Option<String>,
        /// Optional backend API-key reference (`env:VAR`; raw keys are rejected)
        #[arg(long)]
        api_key: Option<String>,
        /// Extra headers as key=value pairs
        #[arg(long, value_parser = parse_header)]
        header: Vec<(String, String)>,
    },
    /// Remove a backend
    Remove {
        /// Backend name to remove
        name: String,
    },
    /// Test backend connectivity
    Test {
        /// Backend name to test (omit to test all)
        name: Option<String>,
    },
    /// Show backend registry status
    Status {
        /// Backend name (omit to show all)
        name: Option<String>,
    },
    /// Sync installed Ollama models into a registered backend
    SyncModels {
        /// Backend name
        name: String,
        /// Override Ollama endpoint (default: use backend's registered endpoint)
        #[arg(long)]
        endpoint: Option<String>,
    },
    /// Add a model to an existing backend
    AddModel {
        /// Backend to add the model to
        backend: String,
        /// Model identifier (sent to API)
        model_id: String,
        /// Maximum context window in tokens
        #[arg(long, default_value = "0")]
        context_window: usize,
        /// Model supports image inputs
        #[arg(long)]
        supports_vision: bool,
        /// Model supports function/tool calling
        #[arg(long)]
        supports_functions: bool,
        /// Model supports fine-tuning through this backend
        #[arg(long)]
        supports_fine_tuning: bool,
        /// Model supports extended thinking
        #[arg(long)]
        supports_thinking: bool,
        /// OpenAI-compatible requests use reasoning token fields
        #[arg(long)]
        uses_reasoning_token_fields: bool,
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
    /// Regenerate package metadata and integrity.toml.
    Build {
        /// Agent directory to build (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
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

#[derive(Subcommand)]
pub enum TeamAction {
    /// List all defined teams
    List,
    /// Show members of a specific team
    Show {
        /// Team name to display
        name: String,
    },
    /// Add a member to an existing team
    Add {
        /// Team name
        #[arg(long)]
        team: String,
        /// Role/agent name for the new member
        #[arg(long)]
        role: String,
        /// ACP profile for the member (e.g., "claude", "codex")
        #[arg(long)]
        profile: String,
        /// Optional system prompt for this member
        #[arg(long)]
        system_prompt: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn production_cli_rejects_compile_service_canonical() {
        assert!(Cli::try_parse_from(["apxm", "compile-service-canonical", "/tmp/agent"]).is_err());
    }

    #[test]
    fn production_cli_rejects_execute_canonical() {
        assert!(Cli::try_parse_from([
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
        .is_err());
    }

    #[test]
    fn compile_service_canonical_accepts_explicit_package_path() {
        let cli = DevCli::try_parse_from(["apxm-dev", "compile-service-canonical", "/tmp/agent"])
            .expect("compile-service-canonical parses");

        match cli.command {
            DevCommands::CompileServiceCanonical { agent_dir } => {
                assert_eq!(agent_dir, PathBuf::from("/tmp/agent"));
            }
            _ => panic!("expected compile-service-canonical command"),
        }
    }

    #[test]
    fn compile_service_canonical_rejects_missing_package_path() {
        let err = match DevCli::try_parse_from(["apxm-dev", "compile-service-canonical"]) {
            Ok(_) => panic!("compile-service-canonical requires an agent package path"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn execute_canonical_accepts_exact_admission_inputs() {
        let cli = DevCli::try_parse_from([
            "apxm-dev",
            "execute-canonical",
            "/tmp/program.air",
            "--invocation-admission",
            "/tmp/admission.json",
            "--release",
            "/tmp/release.json",
            "--provenance",
            "/tmp/provenance.json",
        ])
        .expect("execute-canonical parses");

        match cli.command {
            DevCommands::ExecuteCanonical {
                input,
                invocation_admission,
                release,
                provenance,
                package,
            } => {
                assert_eq!(input, PathBuf::from("/tmp/program.air"));
                assert_eq!(invocation_admission, PathBuf::from("/tmp/admission.json"));
                assert_eq!(release, PathBuf::from("/tmp/release.json"));
                assert_eq!(provenance, PathBuf::from("/tmp/provenance.json"));
                assert_eq!(package, None);
            }
            _ => panic!("expected execute-canonical command"),
        }
    }

    #[test]
    fn execute_canonical_rejects_missing_invocation_admission() {
        let error = match DevCli::try_parse_from(["apxm-dev", "execute-canonical", "/tmp/program.air"])
        {
            Ok(_) => panic!("execute-canonical must require exact host authority"),
            Err(error) => error,
        };
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }
}
