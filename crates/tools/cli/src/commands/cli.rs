//! CLI type definitions for APXM.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use super::implementations::parse_header;

#[derive(Parser)]
#[command(name = "apxm")]
#[command(about = "APxM CLI - compile AIR source and run APXM artifacts", long_about = None)]
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
    /// Initialize a new APXM project directory
    Init {
        /// Project name (creates a directory with this name)
        name: String,
    },
    /// Compile a canonical-authored agent session package to canonical
    /// `apxm.air.v1` (`AirModule`) JSON on stdout, through the canonical
    /// `apxm_program` frontend. Stdout contains only canonical AIR JSON;
    /// diagnostics use stderr and failures are nonzero.
    CompileServiceCanonical {
        /// Agent directory (contains agent.toml with a canonical [compile].entry)
        agent_dir: PathBuf,
    },
    /// Execute canonical `apxm.air.v1` JSON through the canonical runtime.
    ExecuteCanonical {
        /// Canonical AIR JSON file.
        input: PathBuf,
    },
    /// Diagnose compiler/runtime dependencies
    Doctor,
    /// Manage registered inference backend endpoints
    Backend {
        #[command(subcommand)]
        action: BackendAction,
    },
    /// Manage external tool/capability registrations for INV nodes
    Tool {
        #[command(subcommand)]
        action: ToolAction,
    },
    /// Manage ACP profiles for INV(acp) nodes
    Acp {
        #[command(subcommand)]
        action: AcpAction,
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
    /// Scaffold, lint, and install integration packages (apxm.integration-package.v1)
    Integration {
        #[command(subcommand)]
        action: IntegrationAction,
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
    /// Generate frontend assets from Rust-owned registries
    Codegen {
        #[command(subcommand)]
        action: CodegenAction,
    },
    /// Emit canonical AIR from frontend graph JSON.
    ///
    /// Reads the shared FrontendGraph DTO shape from a file or stdin and prints
    /// AIR through the Rust AirModule printer.
    EmitAir {
        /// Frontend graph JSON file. Omit to read stdin.
        input: Option<PathBuf>,
    },
    /// Emit canonical AIR from a canonical FrontendGraph via the native bridge.
    ///
    /// Reads an `apxm.frontend-graph.v1` document from a file or stdin and lowers
    /// it in-process to canonical `apxm.air.v1` JSON.
    CanonicalAir {
        /// Canonical frontend graph JSON file. Omit to read stdin.
        input: Option<PathBuf>,
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
    ///.F — stream a run's dispatch tree from
    /// `/v1/runs/<thread>/events/stream` and render it as monospace.
    Watch {
        /// Thread id (execution id) to attach to. Same id surfaced by
        /// `apxm rollout list` and by the chat panel's URL.
        thread_id: String,
        /// One-shot node expand: pulls `/v1/runs/<thread>/nodes/<id>`
        /// before the live stream starts and prints the detail to stderr.
        #[arg(long)]
        expand: Option<u64>,
    },
    ///.F — inspect, replay, and archive on-disk rollouts.
    Rollout {
        #[command(subcommand)]
        action: RolloutAction,
    },
    /// Interactive conversational REPL over a running apxm-server.
    ///
    /// Requires `--agent <id>` for server-backed chat
    /// (`POST /v1/agents/{{id}}/sessions`) or `--air <path>` for a
    /// custom in-graph artifact with an in-program recv loop.
    Chat {
        /// Agent id for thin server-backed chat. Starts
        /// `POST /v1/agents/{{id}}/sessions` and pipes stdin turns to the
        /// server session.
        #[arg(long = "agent", value_name = "ID", conflicts_with = "air")]
        agent: Option<String>,
        /// AIR graph with an in-program recv loop (path to a `.air` file).
        #[arg(long, conflicts_with = "agent")]
        air: Option<std::path::PathBuf>,
        /// apxm-server base URL (default $APXM_SERVER_BASE or
        /// http://127.0.0.1:18800).
        #[arg(long)]
        server: Option<String>,
        /// Reuse/resume a prior conversation by session id instead of minting
        /// a fresh one.
        #[arg(long)]
        session_id: Option<String>,
        /// Runtime-minted capability grant id for write-tool turns (repeatable).
        #[arg(long = "capability-grant-id", value_name = "GRANT_ID")]
        capability_grant_ids: Vec<String>,
        /// Skill library / id to import into the agent's visible catalogue
        /// (repeatable: `lib`, `lib::skill`, or `skill`). Empty = unrestricted.
        #[arg(long = "import", value_name = "LIB")]
        import: Vec<String>,
        /// Pin agent chat to a registered backend by name (as listed by
        /// `GET /v1/models`).
        #[arg(long, value_name = "NAME")]
        backend: Option<String>,
        /// Pin agent chat to a specific model id.
        #[arg(long, value_name = "ID")]
        model: Option<String>,
        /// Tenant/owner scope for credential resolution.
        #[arg(long = "owner", value_name = "OWNER")]
        owner: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum RolloutAction {
    /// List recent rollouts from the SQLite index.
    List {
        /// Filter by session id.
        #[arg(long)]
        session: Option<String>,
        /// Filter to rollouts that started on or after this RFC3339 ts.
        #[arg(long)]
        since: Option<String>,
        /// Filter by SessionMeta.agent_role.
        #[arg(long = "agent-role")]
        agent_role: Option<String>,
        /// Maximum rows to display.
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Replay a rollout JSONL from disk as a monospace tree.
    Replay {
        /// Thread id of the rollout to replay.
        thread_id: String,
    },
    /// Archive a rollout (JSONL + blobs + optional skill source) as a
    /// `.tar.gz` — the air-gapped reproducibility envelope.
    Archive {
        /// Thread id to bundle.
        thread_id: String,
        /// Output path (default `./apxm-rollout-<thread>.tar.gz`).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Optional directory holding source instruction files.
        #[arg(long = "skill-dir")]
        skill_dir: Option<PathBuf>,
    },
    /// Run the  retention/compaction pass: archive expired rollout
    /// bodies (index row survives with `status=archived`) and collect
    /// unreferenced spilled blobs. Never touches the memory tier.
    Compact {
        /// Override the rollout max-age policy, in days.
        #[arg(long = "max-age-days")]
        max_age_days: Option<u64>,
        /// Override the blob GC grace period, in hours.
        #[arg(long = "blob-grace-hours")]
        blob_grace_hours: Option<u64>,
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
    /// Generate the Python frontend bindings into apxm/_generated
    Frontend {
        /// Output directory for generated Python files
        #[arg(long)]
        output_dir: Option<PathBuf>,
        /// Check that the generated Python files are up to date without writing them
        #[arg(long)]
        check: bool,
    },
    /// Generate TypeScript types into the Studio frontend
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
    /// Generate the op-spec.v1 AIS operation catalog + vectors fixture
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
    List {
        /// Include long-running APXM services such as apxm-server and apxm-studio
        #[arg(long)]
        include_services: bool,
        /// Include repo-local vLLM server/controller processes
        #[arg(long)]
        include_vllm: bool,
    },
    /// Stop APXM compile/run/execute/workflow jobs
    Stop {
        /// Show matching processes without sending a signal
        #[arg(long)]
        dry_run: bool,
        /// Send SIGKILL instead of SIGTERM
        #[arg(long, short)]
        force: bool,
        /// Include long-running APXM services such as apxm-server and apxm-studio
        #[arg(long)]
        include_services: bool,
        /// Include repo-local vLLM server/controller processes
        #[arg(long)]
        include_vllm: bool,
    },
}

#[derive(Subcommand)]
pub enum WorkflowAction {
    /// Run a workflow file
    Run {
        /// Workflow file (.apxmw)
        file: PathBuf,
        /// Start the workflow in a detached APXM child process and return follow handles
        #[arg(long)]
        background: bool,
        /// Workflow arguments as a JSON object for machine callers
        #[arg(long, conflicts_with = "args")]
        args_json: Option<String>,
        /// Workflow arguments (name=value format)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
        /// Explicit sessions root for this workflow run
        #[arg(long)]
        session_root: Option<PathBuf>,
        /// Internal: exact workflow session directory to use
        #[arg(long, hide = true)]
        session_dir: Option<PathBuf>,
    },
    /// Validate a workflow file
    Validate {
        /// Workflow file (.apxmw)
        file: PathBuf,
    },
    /// Show execution phases and critical path for a workflow file
    Analyze {
        /// Workflow file (.apxmw)
        file: PathBuf,
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
    /// dispatched by `apxm execute` / `apxm run`, so drift between "ops
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
        /// Backend name (e.g., "openai", "local-vllm")
        name: String,
        /// Backend type (cloud, onprem, local). For Ollama, defaults to "local".
        #[arg(long, default_value = "")]
        r#type: String,
        /// Protocol (openai, anthropic, google, ollama, vllm)
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
        /// Alternative routing names
        #[arg(long)]
        alias: Vec<String>,
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
pub enum ToolAction {
    /// List registered tools
    List,
    /// Add a new external tool
    Add {
        /// Tool name (used in INV node's "capability" attribute)
        name: String,
        /// Tool description
        #[arg(long)]
        description: String,
        /// JSON Schema for tool input parameters
        #[arg(long)]
        schema: Option<String>,
    },
    /// Remove a registered tool
    Remove {
        /// Tool name to remove
        name: String,
    },
}

#[derive(Subcommand)]
pub enum AcpAction {
    /// List available ACP agent profiles
    List,
    /// Register an agent profile (from template or custom command)
    Add {
        /// Agent profile name (e.g., "claude" or "my-agent")
        name: String,
        /// Agent command to spawn (e.g., "my-agent --acp"). Optional for known templates.
        #[arg(long)]
        command: Option<String>,
        /// Permission mode (approve-all, approve-reads, deny-all)
        #[arg(long, default_value_t)]
        permissions: apxm_acp::PermissionMode,
        /// Grace period in ms after closing stdin before SIGTERM
        #[arg(long)]
        close_grace_ms: Option<u64>,
        /// Confine the agent (and any terminals it opens) under the host sandbox
        /// backend. Requires a functional network-capable OS sandbox; spawning
        /// fails closed if none is available.
        #[arg(long)]
        sandbox: bool,
        /// Skip spawn test (register without verifying the agent is reachable)
        #[arg(long)]
        no_test: bool,
    },
    /// Remove a user agent profile
    Remove {
        /// Agent profile name to remove
        name: String,
    },
    /// Test spawning an agent (runs initialize + session/new + close)
    Test {
        /// Agent profile name to test
        name: String,
    },
    /// List available built-in agent templates
    Templates,
}

#[derive(Subcommand)]
pub enum AgentAction {
    /// Scaffold a new agent folder tree (apxm.agent.v1).
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
    /// Regenerate aggregate manifests from capability/skill folders
    Sync {
        /// Agent directory to sync (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Validate an agent folder against the apxm.agent.v1 contract and
    /// check capability-set agreement across agent.toml/capabilities.toml/skills.
    Lint {
        /// Agent directory to validate (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// An org package directory whose capabilities/{capabilities,
        /// permissions}.toml are this agent's org-global capability set
        ///: a skill invoking one of these is not flagged as
        /// undeclared even though the agent itself never joins it.
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
}

#[derive(Subcommand)]
pub enum OrgAction {
    /// Scaffold a new organization-package folder tree (apxm.org-package.v1).
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
    /// Validate an org-package folder: member resolution, tree
    /// well-formedness, member-hierarchy/topology consistency, and
    /// capability-mask validity against the org's global capability set.
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
pub enum IntegrationAction {
    /// Scaffold a new integration-package folder tree (apxm.integration-package.v1).
    New {
        /// Integration id (also used as integration.toml's integration_id/provider).
        id: String,
        /// Destination directory (default: ./integrations/<id>).
        #[arg(long)]
        path: Option<PathBuf>,
        /// Display name for the provider (default: derived from the id).
        #[arg(long)]
        display_name: Option<String>,
    },
    /// Validate an integration-package folder: the six files exist and parse,
    /// and every capability has a joined permissions.toml policy entry
    /// (write-capable capabilities must default to approval-required).
    Lint {
        /// Integration-package directory to validate (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Install an integration package to `APXM_HOME/integrations/<id>/`,
    /// best-effort copy to `$APXM_WORKSPACE_ROOT/integrations/<id>/`
    /// (the root auth-ms/apxm-os/Studio scan) when that env var is set.
    Install {
        /// Integration-package directory to install (default: current directory).
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

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ToolEntry {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct ToolsFile {
    #[serde(default)]
    pub tools: Vec<ToolEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn compile_service_canonical_accepts_explicit_package_path() {
        let cli = Cli::try_parse_from(["apxm", "compile-service-canonical", "/tmp/agent"])
            .expect("compile-service-canonical parses");

        match cli.command {
            Commands::CompileServiceCanonical { agent_dir } => {
                assert_eq!(agent_dir, PathBuf::from("/tmp/agent"));
            }
            _ => panic!("expected compile-service-canonical command"),
        }
    }

    #[test]
    fn compile_service_canonical_rejects_missing_package_path() {
        let err = match Cli::try_parse_from(["apxm", "compile-service-canonical"]) {
            Ok(_) => panic!("compile-service-canonical requires an agent package path"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn execute_canonical_accepts_air_path() {
        let cli = Cli::try_parse_from(["apxm", "execute-canonical", "/tmp/program.air"])
            .expect("execute-canonical parses");

        match cli.command {
            Commands::ExecuteCanonical { input } => {
                assert_eq!(input, PathBuf::from("/tmp/program.air"));
            }
            _ => panic!("expected execute-canonical command"),
        }
    }
}
