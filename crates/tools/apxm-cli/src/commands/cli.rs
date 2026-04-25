//! CLI type definitions for APXM.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use super::implementations::parse_header;

#[derive(Parser)]
#[command(name = "apxm")]
#[command(about = "APxM CLI (minimal) - compile and run AirModule inputs", long_about = None)]
pub struct Cli {
    /// Optional config path (defaults to .apxm/config.toml or ~/.apxm/config.toml)
    #[arg(long)]
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
    /// Compile AirModule to an artifact
    Compile {
        /// Input graph file or directory (.json graph or .apxmobj artifact)
        input: PathBuf,
        /// Output artifact path
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Emit diagnostics JSON file with compilation statistics
        #[arg(long)]
        emit_diagnostics: Option<PathBuf>,
        /// Emit unified metrics JSON (schema_version=2) with compiler section
        #[arg(long)]
        emit_metrics: Option<PathBuf>,
        /// Optimization level (0 = no optimizations, 1-3 = increasing optimization)
        #[arg(short = 'O', long = "opt-level", default_value = "1")]
        opt_level: u8,
        /// Optimization target: latency, cost, tokens, parallelism, balanced
        #[arg(long, default_value = "balanced")]
        target: String,
        /// Skip CSE for LLM operations (useful with non-zero temperature)
        #[arg(long)]
        no_cse_llm: bool,
        /// Profile-guided optimization: path to execution profile JSON
        #[arg(long)]
        profile: Option<PathBuf>,
        /// Enable the diagnostic `unconsumed-value-warning` pass.
        /// Off by default — the pass is purely diagnostic with no IR mutation.
        #[arg(long, default_value_t = false)]
        warn: bool,
        /// Skip a named pass (repeatable). Useful for ablation studies.
        /// Applied after --pass-list (if both are provided).
        #[arg(long = "disable-pass", value_name = "PASS")]
        disable_passes: Vec<String>,
        /// Override the entire pass list with a comma-separated sequence.
        /// When set, --opt-level / --target / --no-cse-llm / --warn no longer
        /// determine pass selection — only ordering matters here.
        #[arg(long = "pass-list", value_name = "A,B,C", value_delimiter = ',')]
        pass_list_override: Option<Vec<String>>,
    },
    /// Decompile an artifact back to graph JSON
    Decompile {
        /// Input artifact file (.apxmobj)
        artifact: PathBuf,
        /// Output JSON file (defaults to stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Compile and execute an AirModule file through the runtime
    #[command(trailing_var_arg = true)]
    Execute {
        /// Input graph file (.json graph)
        input: PathBuf,
        /// Arguments to pass to the entry flow
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
        /// Optimization level (0 = no optimizations, 1-3 = increasing optimization)
        /// -O0 disables FuseReasoning, -O1+ enables it
        #[arg(short = 'O', long = "opt-level", default_value = "1")]
        opt_level: u8,
        /// Emit metrics JSON file with runtime execution statistics
        #[arg(long)]
        emit_metrics: Option<PathBuf>,
        /// Emit session output folder with all node results, events, metrics.
        /// Default: ON (auto-generates path under ApxmPaths::sessions_dir()).
        /// Pass an explicit path to override, or use --no-emit-session to disable.
        #[arg(long, value_name = "PATH", num_args = 0..=1, conflicts_with = "no_emit_session")]
        emit_session: Option<Option<PathBuf>>,
        /// Disable session output (opt-out of the default-on --emit-session behavior).
        #[arg(long)]
        no_emit_session: bool,
        /// Emit execution profile JSON for profile-guided optimization
        #[arg(long)]
        emit_profile: Option<PathBuf>,
    },
    /// Run a pre-compiled artifact (.apxmobj)
    Run {
        /// Input artifact file (.apxmobj)
        input: PathBuf,
        /// Arguments to pass to the entry flow
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
        /// Emit metrics JSON file with runtime execution statistics
        #[arg(long)]
        emit_metrics: Option<PathBuf>,
        /// Emit session output folder with all node results, events, metrics.
        /// Default: ON (auto-generates path under ApxmPaths::sessions_dir()).
        /// Pass an explicit path to override, or use --no-emit-session to disable.
        #[arg(long, value_name = "PATH", num_args = 0..=1, conflicts_with = "no_emit_session")]
        emit_session: Option<Option<PathBuf>>,
        /// Disable session output (opt-out of the default-on --emit-session behavior).
        #[arg(long)]
        no_emit_session: bool,
        /// Emit execution profile JSON for profile-guided optimization
        #[arg(long)]
        emit_profile: Option<PathBuf>,
    },
    /// Diagnose compiler/runtime dependencies
    Doctor,
    /// Print shell exports for MLIR/LLVM env setup
    Activate {
        /// Shell format (sh, zsh, bash, fish)
        #[arg(long, default_value = "sh")]
        shell: String,
    },
    /// Install or update the conda environment from environment.yaml
    Install,
    /// Manage inference backends (cloud/onprem/local)
    Backend {
        #[command(subcommand)]
        action: BackendAction,
    },
    /// Manage external tool/capability registrations for INV nodes
    Tool {
        #[command(subcommand)]
        action: ToolAction,
    },
    /// Manage ACP agent profiles for INV(acp) nodes
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Manage agent teams from ~/.apxm/teams.toml
    Team {
        #[command(subcommand)]
        action: TeamAction,
    },
    /// Browse AIS operations (the agent instruction set)
    Ops {
        #[command(subcommand)]
        action: OpsAction,
    },
    /// Validate an AirModule file against the AIS contract
    Validate {
        /// Input graph file (.json graph)
        input: PathBuf,
        /// Skip Tier 2 environment checks (registered backends, profiles, etc.)
        #[arg(long)]
        no_check_resources: bool,
    },
    /// Analyze an AirModule for parallelism, critical path, and execution phases
    Analyze {
        /// Input graph file (.json graph)
        input: PathBuf,
    },
    /// Browse graph templates (starter patterns)
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },
    /// Explain what a graph does OR explain an error code
    Explain {
        /// Error code (e.g., E511) or path to graph file (.json graph)
        target: String,
    },
    /// Compose graph fragments (tasks)
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Generate frontend assets from Rust-owned registries
    Codegen {
        #[command(subcommand)]
        action: CodegenAction,
    },
    /// Replay a session trace as a timeline
    Replay {
        /// Session directory path
        session: PathBuf,
    },
    /// Manage and inspect execution sessions
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Manage legacy multi-step workflow files
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },
    /// Manage the MemoCache (response memoization cache)
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Run the tier-3 quality-eval harness.
    /// Trailing args are forwarded verbatim — see `dekk apxm quality-eval -- --help`.
    #[command(name = "quality-eval", trailing_var_arg = true)]
    QualityEval {
        /// Arguments forwarded to the Python harness (--fixture / --all / --opt / ...)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Launch the web-based GUI dashboard
    Gui {
        /// Graph file to open on startup (.air)
        file: Option<PathBuf>,
        /// Port to listen on (default: 18801)
        #[arg(long, short, default_value = "18801")]
        port: u16,
        /// Open browser automatically
        #[arg(long)]
        open: bool,
    },
}

#[derive(Subcommand)]
pub enum TemplateAction {
    /// List available graph templates
    List,
    /// Show a specific template
    Show {
        /// Template name (e.g., ask, pipeline, fan-out, map-reduce)
        name: String,
    },
}

#[derive(Subcommand)]
pub enum TaskAction {
    /// Legacy graph-composition entrypoint
    Merge {
        /// Graph JSON files to merge
        #[arg(required = true)]
        graphs: Vec<PathBuf>,
        /// Name for the merged graph
        #[arg(long)]
        name: String,
        /// Output file (defaults to stdout with --json, or <name>.json)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum CodegenAction {
    /// Generate the Python frontend bindings into apxm/_generated
    Frontend {
        /// Output directory for generated Python files
        #[arg(long)]
        output_dir: Option<PathBuf>,
    },
    /// Generate TypeScript types into the GUI frontend
    Typescript {
        /// Output file path for generated TypeScript
        #[arg(long)]
        output: Option<PathBuf>,
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
    /// Clean old sessions
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
pub enum WorkflowAction {
    /// Execute a legacy workflow file
    Run {
        /// Legacy workflow file (.apxmw)
        file: PathBuf,
        /// Workflow arguments as a JSON object for machine callers
        #[arg(long, conflicts_with = "args")]
        args_json: Option<String>,
        /// Workflow arguments (name=value format)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
        /// Explicit sessions root for this workflow run
        #[arg(long)]
        session_root: Option<PathBuf>,
    },
    /// Validate a legacy workflow file
    Validate {
        /// Legacy workflow file (.apxmw)
        file: PathBuf,
    },
    /// Show execution phases and critical path for a legacy workflow file
    Analyze {
        /// Legacy workflow file (.apxmw)
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
}

#[derive(Subcommand)]
pub enum BackendAction {
    /// List all registered backends
    List {
        /// Output format (table or json)
        #[arg(long, default_value = "table")]
        format: String,
    },
    /// Add a new backend
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
        /// Optional backend API key (omit to read from env or enter interactively)
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
    /// Migrate from legacy credentials.toml
    Migrate {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// Start a local backend container
    Start {
        /// Backend name
        name: String,
    },
    /// Stop a local backend container
    Stop {
        /// Backend name
        name: String,
    },
    /// Show status of local backend containers
    Status {
        /// Backend name (omit to show all)
        name: Option<String>,
    },
    /// Show logs from a local backend container
    Logs {
        /// Backend name
        name: String,
        /// Number of lines to show (default: 50)
        #[arg(long, default_value = "50")]
        tail: usize,
    },
    /// Restart a local backend container
    Restart {
        /// Backend name
        name: String,
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
        /// Cost per 1K input tokens (USD)
        #[arg(long, default_value = "0.0")]
        cost_input: f64,
        /// Cost per 1K output tokens (USD)
        #[arg(long, default_value = "0.0")]
        cost_output: f64,
        /// Model supports image inputs
        #[arg(long)]
        supports_vision: bool,
        /// Model supports function/tool calling
        #[arg(long)]
        supports_functions: bool,
        /// Model supports extended thinking
        #[arg(long)]
        supports_thinking: bool,
        /// Classification tags
        #[arg(long)]
        tag: Vec<String>,
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
pub enum AgentAction {
    /// List registered ACP agents
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
        /// Skip spawn test (register without verifying the agent is reachable)
        #[arg(long)]
        no_test: bool,
    },
    /// Remove a registered agent profile
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
