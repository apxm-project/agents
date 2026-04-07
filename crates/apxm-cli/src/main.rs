//! APXM command-line interface.
//!
//! Provides the `apxm` binary with subcommands for building, compiling,
//! running agent workflows, environment setup (`install`, `doctor`), and
//! LLM credential management (`register`).
//!
//! TODO(refactor): This file is 4449 lines. Split into commands/ modules:
//! - commands/execute.rs — execute + run
//! - commands/compile.rs — compile + decompile
//! - commands/validate.rs — validate + explain + analyze
//! - commands/backend.rs — backend add/list/test/remove/start/stop
//! - commands/agent.rs — agent add/list/test/remove
//! - commands/model.rs — models list/health
//! - commands/workflow.rs — workflow commands
//! Each file should be ≤400 lines. Keep main.rs as thin dispatcher (<100 lines).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use apxm_core::utils::build::MlirEnvReport;
use apxm_credentials::docker::{ContainerStatus, DockerManager};
#[cfg(feature = "driver")]
use apxm_driver::compiler::Compiler;
#[cfg(feature = "driver")]
use apxm_driver::{ApXmConfig, ConfigError, Linker, LinkerConfig};
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::collections::{HashMap, HashSet};
use std::env;

/// Initialize the tracing subscriber based on the --trace flag or RUST_LOG env var.
/// If neither is provided, no subscriber is registered (zero overhead).
#[cfg(feature = "driver")]
fn initialize_tracing(level: &Option<String>) {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let filter_str = match level {
        Some(lvl) => format!(
            "apxm={lvl},apxm_runtime={lvl},apxm_driver={lvl},apxm_core={lvl},apxm_acp={lvl},apxm_backends={lvl},apxm_server={lvl}"
        ),
        None => match std::env::var("RUST_LOG") {
            Ok(val) if !val.is_empty() => val,
            _ => return, // No subscriber = no overhead
        },
    };

    let filter = EnvFilter::try_new(&filter_str).unwrap_or_else(|_| EnvFilter::new("apxm=info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_thread_ids(true)
                .with_target(true)
                .with_ansi(true)
                .with_writer(std::io::stderr),
        )
        .init();
}

#[derive(Parser)]
#[command(name = "apxm")]
#[command(about = "APxM CLI (minimal) - compile and run ApxmGraph inputs", long_about = None)]
struct Cli {
    /// Optional config path (defaults to .apxm/config.toml or ~/.apxm/config.toml)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Enable runtime tracing (levels: trace, debug, info, warn, error)
    #[arg(long, global = true)]
    trace: Option<String>,

    /// Output in JSON format (machine-readable)
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new APXM project directory
    Init {
        /// Project name (creates a directory with this name)
        name: String,
    },
    /// Compile ApxmGraph to an artifact
    Compile {
        /// Input graph file or directory (.json graph or .apxmobj artifact)
        input: PathBuf,
        /// Output artifact path
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Emit diagnostics JSON file with compilation statistics
        #[arg(long)]
        emit_diagnostics: Option<PathBuf>,
        /// Optimization level (0 = no optimizations, 1-3 = increasing optimization)
        #[arg(short = 'O', long = "opt-level", default_value = "1")]
        opt_level: u8,
        /// Optimization target: latency, cost, tokens, parallelism, balanced
        #[arg(long, default_value = "balanced")]
        target: String,
        /// Skip CSE for LLM operations (useful with non-zero temperature)
        #[arg(long)]
        no_cse_llm: bool,
    },
    /// Decompile an artifact back to graph JSON
    Decompile {
        /// Input artifact file (.apxmobj)
        artifact: PathBuf,
        /// Output JSON file (defaults to stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Compile and execute an ApxmGraph file through the runtime
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
        /// Emit session output folder with all node results, events, metrics
        #[arg(long)]
        emit_session: Option<Option<PathBuf>>,
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
        /// Emit session output folder with all node results, events, metrics
        #[arg(long)]
        emit_session: Option<Option<PathBuf>>,
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
    /// Validate an ApxmGraph file against the AIS contract
    Validate {
        /// Input graph file (.json graph)
        input: PathBuf,
        /// Skip Tier 2 environment checks (registered backends, profiles, etc.)
        #[arg(long)]
        no_check_resources: bool,
    },
    /// Analyze an ApxmGraph for parallelism, critical path, and execution phases
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
    /// Manage and execute multi-graph workflows
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },
    /// Manage the MemoCache (response memoization cache)
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
}

#[derive(Subcommand)]
enum TemplateAction {
    /// List available graph templates
    List,
    /// Show a specific template
    Show {
        /// Template name (e.g., ask, pipeline, fan-out, map-reduce)
        name: String,
    },
}

#[derive(Subcommand)]
enum TaskAction {
    /// Merge multiple graph files into a single composed workflow
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
enum CodegenAction {
    /// Generate the Python frontend bindings into apxm/_generated
    Frontend {
        /// Output directory for generated Python files
        #[arg(long)]
        output_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    /// List all sessions
    List {
        /// Filter by status (running/completed/failed)
        #[arg(long)]
        status: Option<String>,
        /// Limit number of sessions shown
        #[arg(long, default_value = "20")]
        limit: usize,
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
    },
}

#[derive(Subcommand)]
enum WorkflowAction {
    /// Execute a workflow file
    Run {
        /// Workflow file (.apxmw)
        file: PathBuf,
        /// Workflow arguments (name=value format)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Validate a workflow file
    Validate {
        /// Workflow file (.apxmw)
        file: PathBuf,
    },
    /// Show execution phases and critical path
    Analyze {
        /// Workflow file (.apxmw)
        file: PathBuf,
    },
}

#[derive(Subcommand)]
enum CacheAction {
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
enum OpsAction {
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
enum BackendAction {
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
        /// API key (omit to read from env or enter interactively)
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
enum ToolAction {
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
enum AgentAction {
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
enum TeamAction {
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
struct ToolEntry {
    name: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct ToolsFile {
    #[serde(default)]
    tools: Vec<ToolEntry>,
}

fn tools_path() -> PathBuf {
    let mut p = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push(".apxm");
    p.push("tools.json");
    p
}

fn load_tools() -> Result<ToolsFile> {
    let path = tools_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ToolsFile::default()),
        Err(e) => return Err(anyhow::anyhow!("Failed to read {}: {e}", path.display())),
    };
    let tf: ToolsFile = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse {}: {e}", path.display()))?;
    Ok(tf)
}

fn save_tools(tf: &ToolsFile) -> Result<()> {
    let path = tools_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("Failed to create {}: {e}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(tf)
        .map_err(|e| anyhow::anyhow!("Failed to serialize tools: {e}"))?;
    std::fs::write(&path, content)
        .map_err(|e| anyhow::anyhow!("Failed to write {}: {e}", path.display()))?;
    Ok(())
}

async fn agent_command(action: AgentAction, json_output: bool) -> Result<()> {
    use apxm_acp::constants::registry::{json_keys, sources};
    match action {
        AgentAction::List => {
            let reg = apxm_acp::AgentRegistry::load();
            let list = reg.list();
            if json_output {
                let entries: Vec<serde_json::Value> = list
                    .iter()
                    .map(|(name, profile, from_template)| {
                        serde_json::json!({
                            (json_keys::NAME): name,
                            (json_keys::COMMAND): profile.command,
                            (json_keys::SOURCE): if *from_template { sources::TEMPLATE } else { sources::CUSTOM },
                            (json_keys::CLOSE_GRACE_MS): profile.close_grace_ms,
                            (json_keys::SESSION_CREATE_TIMEOUT_MS): profile.session_create_timeout_ms,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&entries)
                        .map_err(|e| anyhow::anyhow!("JSON: {e}"))?
                );
                return Ok(());
            }

            if list.is_empty() {
                println!(
                    "No agents registered. Run 'apxm agent add <agent>' or 'apxm agent templates' to see available templates."
                );
                return Ok(());
            }

            print_section_header("Registered ACP Agents");
            let max_name = list.iter().map(|(n, _, _)| n.len()).max().unwrap_or(8);
            let max_source = 8; // "template"
            println!(
                "  {:<name_w$}  {:<src_w$}  {}",
                "AGENT",
                "SOURCE",
                "COMMAND",
                name_w = max_name,
                src_w = max_source,
            );
            for (name, profile, from_template) in &list {
                let source = if *from_template {
                    sources::TEMPLATE
                } else {
                    sources::CUSTOM
                };
                println!(
                    "  {:<name_w$}  {:<src_w$}  {}",
                    name.bold(),
                    source.dimmed(),
                    profile.command,
                    name_w = max_name,
                    src_w = max_source,
                );
            }
            println!();
            println!(
                "{} agent{} registered",
                list.len(),
                if list.len() == 1 { "" } else { "s" }
            );
        }
        AgentAction::Add {
            name,
            command,
            permissions,
            close_grace_ms,
            no_test,
        } => {
            use apxm_acp::constants::timeouts as acp_timeouts;

            let mut reg = apxm_acp::AgentRegistry::load();

            let profile = match command {
                Some(cmd) => {
                    // Custom registration with explicit command
                    apxm_acp::AgentProfile {
                        command: cmd,
                        close_grace_ms: close_grace_ms
                            .unwrap_or(acp_timeouts::DEFAULT_CLOSE_GRACE_MS),
                        session_create_timeout_ms: acp_timeouts::DEFAULT_SESSION_TIMEOUT_MS,
                        permission_mode: permissions,
                        env: Default::default(),
                        default_mode: None,
                        default_model: None,
                        system_prompt: None,
                        skip_preamble: false,
                        capabilities: Vec::new(),
                    }
                }
                None => {
                    // Template-based registration
                    let mut profile = reg
                        .get_template(&name)
                        .ok_or_else(|| {
                            anyhow::anyhow!("Unknown template '{name}'. Run: apxm agent templates")
                        })?
                        .clone();
                    // Apply overrides
                    profile.permission_mode = permissions;
                    if let Some(grace) = close_grace_ms {
                        profile.close_grace_ms = grace;
                    }
                    profile
                }
            };

            // Spawn test: verify the agent is reachable before persisting
            if !no_test {
                println!("Testing agent '{}'...", name.bold());
                println!("  Command: {}", profile.command);
                let cwd = std::env::current_dir().unwrap_or_default();
                let start = std::time::Instant::now();
                let aam_context = apxm_core::types::aam::AamContext::default();
                match apxm_acp::AcpSession::spawn(&name, &profile, &cwd, &aam_context).await {
                    Ok(session) => {
                        let elapsed = start.elapsed();
                        println!(
                            "  {}",
                            format!("Connected in {:.1}s", elapsed.as_secs_f64()).green()
                        );
                        session.close().await;
                    }
                    Err(e) => {
                        print_status_line(&name, Status::Error, &format!("{e}"));
                        return Err(anyhow::anyhow!(
                            "Agent '{}' is not reachable. Is the tool installed?\n\
                             Use --no-test to register without testing.",
                            name
                        ));
                    }
                }
            }

            let display_cmd = profile.command.clone();
            reg.add(name.clone(), profile)
                .map_err(|e| anyhow::anyhow!("Failed to save agent profile: {e}"))?;
            print_section_header("Agent Registered");
            print_status_line(&name, Status::Ok, &display_cmd);
        }
        AgentAction::Remove { name } => {
            let mut reg = apxm_acp::AgentRegistry::load();
            match reg.remove(&name) {
                Ok(true) => {
                    print_section_header("Agent Removed");
                    print_status_line(&name, Status::Ok, "removed");
                }
                Ok(false) => {
                    return Err(anyhow::anyhow!(
                        "Agent '{name}' not found. Run: apxm agent list"
                    ));
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to remove agent: {e}"));
                }
            }
        }
        AgentAction::Test { name } => {
            let reg = apxm_acp::AgentRegistry::load();
            let profile = reg.get(&name).ok_or_else(|| {
                anyhow::anyhow!(
                    "Agent '{name}' not registered. Register with: apxm agent add {name}"
                )
            })?;
            println!("Testing agent '{}'...", name.bold());
            println!("  Command: {}", profile.command);
            let cwd = std::env::current_dir().unwrap_or_default();
            let start = std::time::Instant::now();
            let aam_context = apxm_core::types::aam::AamContext::default();
            match apxm_acp::AcpSession::spawn(&name, profile, &cwd, &aam_context).await {
                Ok(session) => {
                    let elapsed = start.elapsed();
                    println!("  Session ID: {}", session.session_id());
                    if let Some(agent_sid) = session.agent_session_id() {
                        println!("  Agent Session ID: {agent_sid}");
                    }
                    println!(
                        "  {}",
                        format!("Connected in {:.1}s", elapsed.as_secs_f64()).green()
                    );
                    session.close().await;
                    print_status_line(&name, Status::Ok, "agent reachable");
                }
                Err(e) => {
                    print_status_line(&name, Status::Error, &format!("{e}"));
                    return Err(anyhow::anyhow!("Agent test failed: {e}"));
                }
            }
        }
        AgentAction::Templates => {
            let reg = apxm_acp::AgentRegistry::load();
            let templates = reg.list_templates();
            if json_output {
                let entries: Vec<serde_json::Value> = templates
                    .iter()
                    .map(|(name, profile)| {
                        serde_json::json!({
                            (json_keys::NAME): name,
                            (json_keys::COMMAND): profile.command,
                            (json_keys::CLOSE_GRACE_MS): profile.close_grace_ms,
                            (json_keys::SESSION_CREATE_TIMEOUT_MS): profile.session_create_timeout_ms,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&entries)
                        .map_err(|e| anyhow::anyhow!("JSON: {e}"))?
                );
                return Ok(());
            }

            print_section_header("Built-in Agent Templates");
            let max_name = templates.iter().map(|(n, _)| n.len()).max().unwrap_or(8);
            println!(
                "  {:<name_w$}  {:<8}  {}",
                "TEMPLATE",
                "TIMEOUT",
                "COMMAND",
                name_w = max_name,
            );
            for (name, profile) in &templates {
                println!(
                    "  {:<name_w$}  {:<8}  {}",
                    name.bold(),
                    format!("{}ms", profile.session_create_timeout_ms).dimmed(),
                    profile.command,
                    name_w = max_name,
                );
            }
            println!();
            println!(
                "{} templates available. Register with: apxm agent add <name>",
                templates.len()
            );
        }
    }
    Ok(())
}

fn team_command(action: TeamAction, json_output: bool) -> Result<()> {
    use apxm_runtime::team::TeamRegistry;

    match action {
        TeamAction::List => {
            let registry = TeamRegistry::load_from_default_path();
            let teams = registry.list();

            if json_output {
                let team_list: Vec<serde_json::Value> = teams
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "name": t.name,
                            "description": t.description,
                            "members": t.members.len(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&team_list)?);
                return Ok(());
            }

            if teams.is_empty() {
                println!("No teams defined in ~/.apxm/teams.toml.");
                println!("Copy docs/examples/teams.toml to ~/.apxm/teams.toml to get started.");
                return Ok(());
            }

            print_section_header("Agent Teams");
            for team in &teams {
                println!("  {} - {}", team.name.bold(), team.description);
                println!("    Members: {}", team.members.len());
                for member in &team.members {
                    println!("      • {} ({})", member.role, member.profile);
                }
                println!();
            }
            println!("Use 'apxm team show <name>' to view full team configuration.");
            Ok(())
        }

        TeamAction::Show { name } => {
            let registry = TeamRegistry::load_from_default_path();
            let team = registry.get(&name).ok_or_else(|| {
                anyhow::anyhow!(
                    "Team '{}' not found. Use 'apxm team list' to see available teams.",
                    name
                )
            })?;

            if json_output {
                println!("{}", serde_json::to_string_pretty(&team)?);
                return Ok(());
            }

            print_section_header(&format!("Team: {}", team.name));
            println!("  Description: {}", team.description);
            println!("\n  Members:");
            for member in &team.members {
                println!("\n    Role:    {}", member.role.bold());
                println!("    Profile: {}", member.profile);
                if let Some(ref prompt) = member.system_prompt {
                    let line_count = prompt.lines().count();
                    println!("    Prompt:  {}", prompt.lines().next().unwrap_or(""));
                    if line_count > 1 {
                        println!("             (+ {} more lines)", line_count - 1);
                    }
                }
            }
            println!();
            Ok(())
        }

        TeamAction::Add {
            team,
            role,
            profile,
            system_prompt,
        } => {
            let mut registry = TeamRegistry::load_from_default_path();
            registry.add_member(&team, role.clone(), profile.clone(), system_prompt)?;

            println!(
                "Added member '{}' (profile: {}) to team '{}'.",
                role, profile, team
            );
            println!("Team definition saved to ~/.apxm/teams.toml");
            Ok(())
        }
    }
}

#[allow(dead_code)] // Planned for human-readable metrics formatting
fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn tool_command(action: ToolAction, json_output: bool) -> Result<()> {
    match action {
        ToolAction::List => {
            let tf = load_tools()?;
            if json_output {
                let json = serde_json::to_string_pretty(&tf.tools)
                    .map_err(|e| anyhow::anyhow!("JSON serialization failed: {e}"))?;
                println!("{json}");
                return Ok(());
            }
            if tf.tools.is_empty() {
                println!("No tools registered.");
                println!("Add one with: apxm tool add <name> --description \"...\"");
                return Ok(());
            }
            print_section_header("Registered Tools");
            let max_name = tf.tools.iter().map(|t| t.name.len()).max().unwrap_or(8);
            for tool in &tf.tools {
                println!(
                    "  {:<width$}    {}",
                    tool.name.bold(),
                    tool.description,
                    width = max_name
                );
            }
            println!();
            println!(
                "{} tool{} registered",
                tf.tools.len(),
                if tf.tools.len() == 1 { "" } else { "s" }
            );
        }
        ToolAction::Add {
            name,
            description,
            schema,
        } => {
            let mut tf = load_tools()?;
            if tf.tools.iter().any(|t| t.name == name) {
                return Err(anyhow::anyhow!(
                    "Tool '{}' is already registered. Remove it first with: apxm tool remove {}",
                    name,
                    name
                ));
            }
            tf.tools.push(ToolEntry {
                name: name.clone(),
                description: description.clone(),
                schema,
            });
            save_tools(&tf)?;
            print_section_header("Tool Registered");
            print_status_line(&name, Status::Ok, &description);
        }
        ToolAction::Remove { name } => {
            let mut tf = load_tools()?;
            let before = tf.tools.len();
            tf.tools.retain(|t| t.name != name);
            if tf.tools.len() == before {
                return Err(anyhow::anyhow!("Tool '{}' not found", name));
            }
            save_tools(&tf)?;
            print_section_header("Tool Removed");
            print_status_line(&name, Status::Ok, "removed");
        }
    }
    Ok(())
}

#[cfg(feature = "driver")]
fn parse_opt_level(level: u8) -> apxm_core::types::OptimizationLevel {
    use apxm_core::types::OptimizationLevel;
    match level {
        0 => OptimizationLevel::O0,
        1 => OptimizationLevel::O1,
        2 => OptimizationLevel::O2,
        _ => OptimizationLevel::O3,
    }
}

#[allow(dead_code)] // Python frontend integration - not yet wired to compile/execute commands
fn is_python_graph_input(input: &Path) -> bool {
    input.extension().and_then(|ext| ext.to_str()) == Some("py")
}

#[allow(dead_code)] // Python frontend integration - not yet wired to compile/execute commands
fn emit_air_from_python(input: &Path) -> Result<tempfile::NamedTempFile> {
    use std::io::Write;

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let python_frontend = repo_root.join("crates/apxm-frontend/python");

    let mut pythonpath_entries = vec![python_frontend, repo_root];
    if let Some(parent) = input.parent() {
        pythonpath_entries.push(parent.to_path_buf());
    }
    if let Some(existing) = env::var_os("PYTHONPATH") {
        pythonpath_entries.extend(env::split_paths(&existing));
    }

    let pythonpath = env::join_paths(pythonpath_entries)
        .context("Failed to build PYTHONPATH for APXM Python frontend")?;
    let mut output = None;
    for candidate in ["python3", "python"] {
        match std::process::Command::new(candidate)
            .arg(input)
            .env("PYTHONPATH", &pythonpath)
            .output()
        {
            Ok(result) => {
                output = Some(result);
                break;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "Failed to run Python workflow {} with {}: {}",
                    input.display(),
                    candidate,
                    err
                ));
            }
        }
    }

    let output = output.ok_or_else(|| {
        anyhow::anyhow!("Python interpreter not found on PATH (tried python3, python)")
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "Python workflow {} failed: {}",
            input.display(),
            stderr.trim()
        ));
    }

    let air = String::from_utf8(output.stdout)
        .with_context(|| format!("Python workflow {} did not emit valid UTF-8", input.display()))?;
    let trimmed = air.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!(
            "Python workflow {} produced no .air output",
            input.display()
        ));
    }
    if !(trimmed.starts_with(';') || trimmed.starts_with('%')) {
        return Err(anyhow::anyhow!(
            "Python workflow {} did not emit recognizable .air text on stdout",
            input.display()
        ));
    }

    let mut tmp = tempfile::Builder::new()
        .suffix(".air")
        .tempfile()
        .context("Failed to create temporary .air file")?;
    tmp.write_all(air.as_bytes())
        .context("Failed to write emitted .air to temporary file")?;
    tmp.flush()
        .context("Failed to flush temporary .air file")?;
    Ok(tmp)
}

#[allow(dead_code)] // Python frontend integration - not yet wired to compile/execute commands
fn prepare_graph_input(
    input: &Path,
) -> Result<(PathBuf, Option<tempfile::NamedTempFile>)> {
    if is_python_graph_input(input) {
        let tmp = emit_air_from_python(input)?;
        return Ok((tmp.path().to_path_buf(), Some(tmp)));
    }

    Ok((input.to_path_buf(), None))
}

fn category_str(cat: apxm_core::types::OperationCategory) -> &'static str {
    use apxm_core::types::OperationCategory;
    match cat {
        OperationCategory::Metadata => "metadata",
        OperationCategory::Memory => "memory",
        OperationCategory::Reasoning => "reasoning",
        OperationCategory::Tools => "tools",
        OperationCategory::ControlFlow => "control_flow",
        OperationCategory::Synchronization => "synchronization",
        OperationCategory::ErrorHandling => "error_handling",
        OperationCategory::Communication => "communication",
        OperationCategory::Internal => "internal",
        OperationCategory::Coordination => "coordination",
        OperationCategory::Identity => "identity",
    }
}

fn latency_to_ms(lat: apxm_core::types::OperationLatency) -> u64 {
    use apxm_core::types::OperationLatency;
    match lat {
        OperationLatency::None => 10,
        OperationLatency::Low => 100,
        OperationLatency::Medium => 1000,
        OperationLatency::High => 5000,
    }
}

fn find_op_spec(op: &str) -> Option<&'static apxm_core::types::OperationSpec> {
    use apxm_core::types::AIS_OPERATIONS;
    AIS_OPERATIONS.iter().find(|s| s.op_type.to_string() == op)
}

fn op_latency_ms(op: &str) -> u64 {
    find_op_spec(op).map_or(100, |s| latency_to_ms(s.latency))
}

fn parse_header(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid header: no '=' found in '{s}'"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

#[cfg(feature = "driver")]
#[tokio::main]
async fn main() -> Result<()> {
    run_cli().await
}

#[cfg(not(feature = "driver"))]
fn main() -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_cli_no_driver())
}

#[cfg(feature = "driver")]
async fn run_cli() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing if --trace flag is provided
    initialize_tracing(&cli.trace);

    match cli.command {
        Commands::Init { name } => init_command(&name),
        Commands::Compile {
            input,
            output,
            emit_diagnostics,
            opt_level,
            target,
            no_cse_llm,
        } => compile_command(input, output, emit_diagnostics, opt_level, target, no_cse_llm),
        Commands::Decompile { artifact, output } => decompile_command(artifact, output),
        Commands::Execute {
            input,
            args,
            opt_level,
            emit_metrics,
            emit_session,
        } => {
            execute_command(
                input,
                args,
                opt_level,
                cli.config,
                emit_metrics,
                emit_session,
            )
            .await
        }
        Commands::Run {
            input,
            args,
            emit_metrics,
            emit_session,
        } => run_command(input, args, cli.config, emit_metrics, emit_session).await,
        Commands::Doctor => doctor_command(cli.config, cli.json),
        Commands::Activate { shell } => activate_command(&shell),
        Commands::Install => install_command(),
        Commands::Backend { action } => backend_command(action, cli.json).await,
        Commands::Tool { action } => tool_command(action, cli.json),
        Commands::Agent { action } => agent_command(action, cli.json).await,
        Commands::Team { action } => team_command(action, cli.json),
        Commands::Ops { action } => ops_command(action, cli.json),
        Commands::Validate {
            input,
            no_check_resources,
        } => validate_command(input, cli.json, no_check_resources),
        Commands::Analyze { input } => analyze_command(input, cli.json),
        Commands::Template { action } => template_command(action, cli.json),
        Commands::Explain { target } => explain_command(&target, cli.json),
        Commands::Task { action } => task_command(action, cli.json),
        Commands::Codegen { action } => codegen_command(action, cli.json),
        Commands::Replay { session } => replay_command(session),
        Commands::Session { action } => session_command(action, cli.json),
        Commands::Workflow { action } => workflow_command(action, cli.json).await,
        Commands::Cache { action } => cache_command(action, cli.json),
    }
}

#[cfg(not(feature = "driver"))]
async fn run_cli_no_driver() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { name } => init_command(&name),
        Commands::Doctor => doctor_command(cli.config, cli.json),
        Commands::Activate { shell } => activate_command(&shell),
        Commands::Install => install_command(),
        Commands::Tool { action } => tool_command(action, cli.json),
        Commands::Agent { action } => agent_command(action, cli.json).await,
        Commands::Team { action } => team_command(action, cli.json),
        Commands::Ops { action } => ops_command(action, cli.json),
        Commands::Validate {
            input,
            no_check_resources,
        } => validate_command(input, cli.json, no_check_resources),
        Commands::Analyze { input } => analyze_command(input, cli.json),
        Commands::Template { action } => template_command(action, cli.json),
        Commands::Explain { target } => explain_command(&target, cli.json),
        Commands::Task { action } => task_command(action, cli.json),
        Commands::Codegen { action } => codegen_command(action, cli.json),
        Commands::Replay { session } => replay_command(session),
        Commands::Session { action } => session_command(action, cli.json),
        Commands::Workflow { action } => workflow_command_no_driver(action, cli.json),
        Commands::Cache { action } => cache_command(action, cli.json),
        _ => Err(anyhow::anyhow!(
            "Command requires the `driver` feature. Re-run with: cargo run -p apxm-cli --features driver -- <command>"
        )),
    }
}

fn activate_command(shell: &str) -> Result<()> {
    let prefix = detect_conda_prefix().ok_or_else(|| {
        anyhow::anyhow!("Could not detect conda prefix. Activate your env or install it first.")
    })?;

    match shell {
        "sh" | "bash" | "zsh" => {
            println!(
                "export MLIR_DIR={}",
                prefix.join("lib/cmake/mlir").display()
            );
            println!(
                "export LLVM_DIR={}",
                prefix.join("lib/cmake/llvm").display()
            );
            println!("export MLIR_PREFIX={}", prefix.display());
            println!("export LLVM_PREFIX={}", prefix.display());
            println!("export PATH={}/bin:$PATH", prefix.display());
        }
        "fish" => {
            println!(
                "set -gx MLIR_DIR {}",
                prefix.join("lib/cmake/mlir").display()
            );
            println!(
                "set -gx LLVM_DIR {}",
                prefix.join("lib/cmake/llvm").display()
            );
            println!("set -gx MLIR_PREFIX {}", prefix.display());
            println!("set -gx LLVM_PREFIX {}", prefix.display());
            println!("set -gx PATH {}/bin $PATH", prefix.display());
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported shell '{}'. Use sh, bash, zsh, or fish.",
                shell
            ));
        }
    }

    Ok(())
}

fn install_command() -> Result<()> {
    print_section_header("APXM Install");

    if !command_available("mamba") {
        print_status_line("mamba", Status::Error, "not found");
        return Err(anyhow::anyhow!(
            "mamba not found in PATH. Install mamba first."
        ));
    }

    print_status_line("mamba", Status::Ok, "found");
    let installer = "mamba";

    print_status_line("env", Status::Ok, "creating/updating");

    let create_status = std::process::Command::new(installer)
        .args(["env", "create", "-f", "environment.yaml"])
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to run {installer}: {e}"))?;

    if !create_status.success() {
        let update_status = std::process::Command::new(installer)
            .args(["env", "update", "-f", "environment.yaml", "-n", "apxm"])
            .status()
            .map_err(|e| anyhow::anyhow!("Failed to run {installer}: {e}"))?;

        if !update_status.success() {
            return Err(anyhow::anyhow!(
                "{installer} env create/update failed. Check output."
            ));
        }
    }

    print_status_line("env", Status::Ok, "ready");
    println!();
    print_subsection_header("Next Steps");
    println!("conda activate apxm");
    println!("eval \"$(cargo run -p apxm-cli -- activate)\"");

    Ok(())
}

fn command_available(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn init_command(name: &str) -> Result<()> {
    let base = PathBuf::from(name);
    if base.exists() {
        return Err(anyhow::anyhow!("Directory '{}' already exists", name));
    }

    let dirs = ["agents", "flows", "nodes", "prompts", "tools"];
    for d in &dirs {
        std::fs::create_dir_all(base.join(d))
            .map_err(|e| anyhow::anyhow!("Failed to create {}/{}: {}", name, d, e))?;
    }

    let toml_content = format!(
        r#"[project]
name = "{name}"
version = "0.1.0"

[build]
opt_level = "O1"

[runtime]
max_parallel = 4
"#
    );
    std::fs::write(base.join("apxm.toml"), toml_content)
        .map_err(|e| anyhow::anyhow!("Failed to write {}/apxm.toml: {}", name, e))?;

    println!("Initialized APXM project '{}'", name);
    for d in &dirs {
        println!("  {}/{}/", name, d);
    }
    println!("  {}/apxm.toml", name);
    Ok(())
}

#[cfg(feature = "driver")]
fn compile_command(
    input: PathBuf,
    output: Option<PathBuf>,
    emit_diagnostics: Option<PathBuf>,
    opt_level: u8,
    target: String,
    no_cse_llm: bool,
) -> Result<()> {
    use apxm_core::constants::diagnostics;
    use apxm_core::types::{OptimizationTarget, PipelineConfig};

    let opt = parse_opt_level(opt_level);
    let opt_target: OptimizationTarget = target.parse()
        .with_context(|| format!("Invalid optimization target: {}", target))?;
    let (graph_input, _python_air) = if input.is_dir() {
        (input.clone(), None)
    } else {
        prepare_graph_input(&input)?
    };

    let compile_start = std::time::Instant::now();
    let compiler = Compiler::with_opt_level(opt).context("Failed to initialize compiler")?;

    // Check if this is a new-format .air file (valid MLIR)
    let is_new_air = if !input.is_dir() && graph_input.extension().and_then(|e| e.to_str()) == Some("air") {
        if let Ok(text) = std::fs::read_to_string(&graph_input) {
            text.trim_start().starts_with("module") || text.trim_start().starts_with("func.func")
        } else {
            false
        }
    } else {
        false
    };

    // For new .air format (valid MLIR), compile directly without ApxmGraph
    if is_new_air {
        // NEW PATH: .air file is valid MLIR — compile directly
        // (no-cse-llm and diagnostics are graph-level optimizations, not applicable here)
        let module = compiler
            .compile(&graph_input)
            .map_err(|e| anyhow::anyhow!("Failed to compile MLIR: {e}"))?;
        let compile_time = compile_start.elapsed();

        let artifact_start = std::time::Instant::now();
        let bytes = module
            .generate_artifact_bytes()
            .context("Failed to generate artifact")?;
        let artifact_time = artifact_start.elapsed();

        let out_path = output.unwrap_or_else(|| {
            graph_input.with_extension(apxm_core::constants::extensions::ARTIFACT)
        });
        std::fs::write(&out_path, &bytes)
            .with_context(|| format!("Failed to write {}", out_path.display()))?;

        println!(
            "{} Compiled to {}",
            apxm_core::constants::ui::icons::SUCCESS,
            out_path.display()
        );
        println!("  Compilation: {:?}", compile_time);
        println!("  Artifact generation: {:?}", artifact_time);
        println!("  Artifact size: {} bytes", bytes.len());

        return Ok(());
    }

    // OLD PATH: ApxmGraph-based compilation
    let graph = if input.is_dir() {
        load_graph_from_directory(&input)?
    } else {
        compiler
            .load_graph(&graph_input)
            .map_err(|e| anyhow::anyhow!("Failed to parse graph: {e}"))?
    };

    // Validate model allowlist if configured
    if let Ok(config) = ApXmConfig::load_default() {
        Compiler::validate_model_allowlist(&graph, config.models.allowlist.as_ref())?;
    }

    // When diagnostics are requested, use the per-pass metrics path.
    // Otherwise use the fast bulk-run path.
    let (module, pass_diagnostics) = if emit_diagnostics.is_some() {
        let config = PipelineConfig {
            opt_level: opt,
            target: opt_target,
            verify: true,
            no_cse_llm,
            ..Default::default()
        };
        let (m, d) = compiler
            .compile_graph_with_config_and_diagnostics(&graph, config)
            .map_err(|e| anyhow::anyhow!("Failed to compile graph: {e}"))?;
        (m, Some(d))
    } else if no_cse_llm || opt_target != OptimizationTarget::Balanced {
        let config = PipelineConfig {
            opt_level: opt,
            target: opt_target,
            verify: true,
            no_cse_llm,
            ..Default::default()
        };
        let m = compiler
            .compile_graph_with_config(&graph, config)
            .map_err(|e| anyhow::anyhow!("Failed to compile graph: {e}"))?;
        (m, None)
    } else {
        let m = compiler
            .compile_graph(&graph)
            .map_err(|e| anyhow::anyhow!("Failed to compile graph: {e}"))?;
        (m, None)
    };
    let compile_time = compile_start.elapsed();

    let artifact_start = std::time::Instant::now();
    let bytes = module
        .generate_artifact_bytes()
        .context("Failed to generate artifact")?;
    let artifact_time = artifact_start.elapsed();

    let out_path = output.unwrap_or_else(|| {
        if input.is_dir() {
            input.join(format!(
                "{}.{}",
                graph.name,
                apxm_core::constants::extensions::ARTIFACT
            ))
        } else {
            input.with_extension(apxm_core::constants::extensions::ARTIFACT)
        }
    });
    std::fs::write(&out_path, &bytes)
        .with_context(|| format!("Failed to write {}", out_path.display()))?;

    // Emit diagnostics if requested
    if let Some(diag_path) = emit_diagnostics {
        let artifact = apxm_artifact::Artifact::from_bytes(&bytes)
            .map_err(|e| anyhow::anyhow!("Failed to parse artifact: {}", e))?;
        let dag = artifact
            .dag()
            .ok_or_else(|| anyhow::anyhow!("Artifact contains no DAGs"))?;

        // Build per-pass metrics array from diagnostics
        let per_pass_metrics: Vec<serde_json::Value> = pass_diagnostics
            .as_ref()
            .map(|d| {
                d.passes
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "pass_name": p.pass_name,
                            "duration_ms": p.duration_ms,
                            "ops_before": p.ops_before,
                            "ops_after": p.ops_after,
                            "ops_delta": p.ops_delta
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let diagnostics_json = serde_json::json!({
            "input": input.display().to_string(),
            "mode": diagnostics::MODE_GRAPH,
            "graph_name": graph.name,
            "optimization_level": format!("O{}", opt_level),
            "compilation_phases": {
                "total_ms": compile_time.as_secs_f64() * 1000.0,
                "artifact_gen_ms": artifact_time.as_secs_f64() * 1000.0,
                "passes_ms": pass_diagnostics.as_ref().map(|d| d.total_duration_ms).unwrap_or(0.0)
            },
            "dag_statistics": {
                "total_nodes": dag.nodes.len(),
                "entry_nodes": dag.entry_nodes.len(),
                "exit_nodes": dag.exit_nodes.len(),
                "total_edges": dag.edges.len()
            },
            "pass_metrics": per_pass_metrics,
            "pass_summary": {
                "total_passes": pass_diagnostics.as_ref().map(|d| d.pass_count()).unwrap_or(0),
                "initial_ops": pass_diagnostics.as_ref().map(|d| d.initial_ops).unwrap_or(0),
                "final_ops": pass_diagnostics.as_ref().map(|d| d.final_ops).unwrap_or(0),
                "total_ops_eliminated": pass_diagnostics.as_ref().map(|d| d.total_ops_eliminated()).unwrap_or(0),
                "active_passes": pass_diagnostics.as_ref().map(|d| d.active_passes().iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap_or_default()
            }
        });

        std::fs::write(&diag_path, serde_json::to_string_pretty(&diagnostics_json)?)
            .with_context(|| format!("Failed to write diagnostics to {}", diag_path.display()))?;

        println!("Wrote diagnostics to {}", diag_path.display());
    }

    println!("Wrote graph artifact to {}", out_path.display());
    println!(
        "Compiled in {:.2}ms, artifact generated in {:.2}ms",
        compile_time.as_secs_f64() * 1000.0,
        artifact_time.as_secs_f64() * 1000.0
    );
    Ok(())
}

#[cfg(feature = "driver")]
fn decompile_command(artifact_path: PathBuf, output: Option<PathBuf>) -> Result<()> {
    let bytes = std::fs::read(&artifact_path)
        .with_context(|| format!("Failed to read {}", artifact_path.display()))?;
    let artifact = apxm_artifact::Artifact::from_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("Failed to parse artifact: {}", e))?;
    let dag = artifact
        .dag()
        .ok_or_else(|| anyhow::anyhow!("Artifact contains no DAGs"))?;

    let graph = dag_to_graph(dag);
    let json = serde_json::to_string_pretty(&graph)?;

    if let Some(out_path) = output {
        std::fs::write(&out_path, &json)
            .with_context(|| format!("Failed to write {}", out_path.display()))?;
        println!("Decompiled to {}", out_path.display());
    } else {
        println!("{}", json);
    }
    Ok(())
}

#[cfg(feature = "driver")]
fn dag_to_graph(dag: &apxm_core::types::execution::ExecutionDag) -> apxm_graph::ApxmGraph {
    use apxm_graph::{GraphEdge, GraphNode};

    let nodes: Vec<GraphNode> = dag
        .nodes
        .iter()
        .map(|n| GraphNode {
            id: n.id,
            name: format!("node_{}", n.id),
            op: n.op_type,
            attributes: n.attributes.clone(),
        })
        .collect();

    let edges: Vec<GraphEdge> = dag
        .edges
        .iter()
        .map(|e| GraphEdge {
            from: e.from,
            to: e.to,
            dependency: e.dependency_type.clone(),
        })
        .collect();

    let parameters: Vec<apxm_graph::Parameter> = dag
        .metadata
        .parameters
        .iter()
        .map(|p| apxm_graph::Parameter {
            name: p.name.clone(),
            type_name: p.type_name.clone(),
        })
        .collect();

    apxm_graph::ApxmGraph {
        name: dag
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "decompiled".to_string()),
        nodes,
        edges,
        parameters,
        metadata: std::collections::HashMap::new(),
    }
}

#[cfg(feature = "driver")]
fn load_graph_from_directory(dir: &std::path::Path) -> Result<apxm_graph::ApxmGraph> {
    let mut graphs = Vec::new();
    let subdirs = ["flows", "nodes", ""];

    for subdir in &subdirs {
        let search_dir = if subdir.is_empty() {
            dir.to_path_buf()
        } else {
            dir.join(subdir)
        };
        if !search_dir.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&search_dir)? {
            let entry = entry?;
            let path = entry.path();
            if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("json") // .apxm removed; json kept for internal decompile only
            ) {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read {}", path.display()))?;
                if text.contains("\"nodes\"") {
                    let graph = apxm_graph::ApxmGraph::from_json(&text).map_err(|e| {
                        anyhow::anyhow!("Failed to parse {}: {}", path.display(), e)
                    })?;
                    graphs.push(graph);
                }
            }
        }
    }

    if graphs.is_empty() {
        return Err(anyhow::anyhow!(
            "No graph files (.apxm) found in directory '{}'",
            dir.display()
        ));
    }

    if graphs.len() == 1 {
        return Ok(graphs.into_iter().next().unwrap());
    }

    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("merged")
        .to_string();
    Ok(apxm_graph::ApxmGraph::merge(&name, &graphs))
}

/// Convert an ExecutionDag back to an ApxmGraph for session output.
#[cfg(feature = "driver")]
fn graph_from_execution_dag(
    dag: &apxm_core::types::execution::ExecutionDag,
) -> Option<apxm_graph::ApxmGraph> {
    use apxm_graph::{GraphEdge, GraphNode, Parameter};
    use std::collections::HashMap;

    let nodes = dag
        .nodes
        .iter()
        .map(|node| GraphNode {
            id: node.id,
            name: node
                .metadata
                .name
                .clone()
                .unwrap_or_else(|| format!("node_{}", node.id)),
            op: node.op_type,
            attributes: node.attributes.clone(),
        })
        .collect::<Vec<_>>();

    let edges = dag
        .edges
        .iter()
        .map(|edge| GraphEdge {
            from: edge.from,
            to: edge.to,
            dependency: edge.dependency_type.clone(),
        })
        .collect::<Vec<_>>();

    let parameters = dag
        .metadata
        .parameters
        .iter()
        .map(|param| Parameter {
            name: param.name.clone(),
            type_name: param.type_name.clone(),
        })
        .collect::<Vec<_>>();

    let mut metadata = HashMap::new();
    if dag.metadata.is_entry {
        metadata.insert(
            "is_entry".to_string(),
            apxm_core::types::values::Value::Bool(true),
        );
    }

    Some(apxm_graph::ApxmGraph {
        name: dag
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "artifact".to_string()),
        nodes,
        edges,
        parameters,
        metadata,
    })
}

#[cfg(feature = "driver")]
fn load_graph_for_session(input: &std::path::Path) -> Result<apxm_graph::ApxmGraph> {
    use apxm_driver::compiler::Compiler;

    // Try to load via compiler first so .air gets its dedicated error message.
    if let Ok(compiler) = Compiler::new() {
        if let Ok(graph) = compiler.load_graph(input) {
            return Ok(graph);
        }
    }

    // Fallback: parse as JSON directly when the compiler is unavailable.
    let text = std::fs::read_to_string(input).context("Failed to read graph file")?;
    apxm_graph::ApxmGraph::from_json(&text)
        .map_err(|e| anyhow::anyhow!("Failed to parse graph: {}", e))
}

#[cfg(feature = "driver")]
fn setup_session(
    emit_session: &Option<Option<PathBuf>>,
    input: &std::path::Path,
    default_stem: &str,
    input_graph: Option<&apxm_graph::ApxmGraph>,
) -> Result<(
    Option<apxm_driver::session_output::SessionOutputWriter>,
    Option<std::sync::Arc<apxm_driver::session_output::SessionEventEmitter>>,
    Option<String>,
)> {
    use apxm_core::paths::ApxmPaths;
    use apxm_driver::session_output::{SessionEventEmitter, SessionOutputWriter};

    let Some(custom_path) = emit_session else {
        return Ok((None, None, None));
    };

    let base_dir = match custom_path {
        Some(p) => p.clone(),
        None => ApxmPaths::discover()
            .context("Failed to discover APXM paths")?
            .sessions_dir()
            .context("Failed to create sessions directory")?,
    };

    let exec_id = format!(
        "{}-{}",
        input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(default_stem),
        chrono::Utc::now().format("%Y%m%dT%H%M%S")
    );

    let w = SessionOutputWriter::new(&base_dir, &exec_id)
        .context("Failed to create session output directory")?;

    let graph_name = input.file_stem().and_then(|s| s.to_str());
    w.write_manifest(
        &exec_id,
        graph_name,
        apxm_core::constants::session::status::RUNNING,
        0,
        0,
        false,
    )
    .context("Failed to write manifest")?;

    if let Some(graph) = input_graph {
        w.write_input_graph(graph)
            .context("Failed to write input graph")?;
    }

    eprintln!("Session: {}", w.session_dir().display());

    let project_root = std::env::current_dir().ok();
    let emitter = std::sync::Arc::new(
        SessionEventEmitter::new(
            w.session_dir(),
            exec_id.clone(),
            input_graph,
            project_root.as_deref(),
        )
        .context("Failed to create session event emitter")?,
    );

    // Set total node count for progress tracking in live.json
    if let Some(graph) = input_graph {
        emitter.set_total_nodes(graph.nodes.len() as u64);
    }

    Ok((Some(w), Some(emitter), Some(exec_id)))
}

#[cfg(feature = "driver")]
fn context_stack_config_from_graph(
    session_dir: &std::path::Path,
    graph: &apxm_graph::ApxmGraph,
) -> Option<apxm_runtime::context_stack::ContextStackConfig> {
    let node_metadata = graph
        .nodes
        .iter()
        .map(|node| {
            (
                node.id,
                apxm_runtime::context_stack::NodeMetadata {
                    name: node.name.clone(),
                    op_type: format!("{:?}", node.op),
                },
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let graph_edges = graph
        .edges
        .iter()
        .map(|edge| (edge.from, edge.to))
        .collect();

    Some(apxm_runtime::context_stack::ContextStackConfig {
        session_dir: session_dir.to_path_buf(),
        node_metadata,
        graph_edges,
    })
}

#[cfg(feature = "driver")]
async fn execute_command(
    input: PathBuf,
    args: Vec<String>,
    opt_level: u8,
    config: Option<PathBuf>,
    emit_metrics: Option<PathBuf>,
    emit_session: Option<Option<PathBuf>>,
) -> Result<()> {
    let apxm_config = load_config(config).context("Failed to load configuration")?;
    let opt = parse_opt_level(opt_level);
    let mut linker_config = LinkerConfig::from_apxm_config(apxm_config).with_opt_level(opt);
    let (graph_input, _python_air) = prepare_graph_input(&input)?;

    // Enable all-outputs collection when session output is requested
    if emit_session.is_some() {
        linker_config
            .runtime_config
            .scheduler_config
            .collect_all_outputs = true;
    }

    // Load input graph for session output
    let input_graph = if emit_session.is_some() {
        load_graph_for_session(&graph_input).ok()
    } else {
        None
    };

    // Set up session output + live emitter BEFORE execution
    let (writer, emitter, execution_id) =
        setup_session(&emit_session, &input, "graph", input_graph.as_ref())?;

    if let (Some(graph), Some(writer)) = (input_graph.as_ref(), writer.as_ref()) {
        linker_config.runtime_config.context_stack =
            context_stack_config_from_graph(writer.session_dir(), graph);
    }

    let linker = Linker::new(linker_config)
        .await
        .context("Failed to initialize runtime")?;

    // Set memory system on emitter so context assembler can query episodic history
    if let Some(ref e) = emitter {
        e.set_memory(linker.runtime_executor().memory_system());
    }

    // Coerce Arc<SessionEventEmitter> → Arc<dyn ExecutionEventEmitter> for run_graph
    let emitter_dyn: Option<std::sync::Arc<dyn apxm_runtime::ExecutionEventEmitter>> = emitter
        .as_ref()
        .map(|e| e.clone() as std::sync::Arc<dyn apxm_runtime::ExecutionEventEmitter>);

    // Background ticker: updates live.json elapsed_ms every second so it stays
    // current even during long LLM calls between operation events.
    let ticker_handle = emitter.as_ref().map(|e| {
        let e = e.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                e.tick();
            }
        })
    });

    let result = match linker
        .run_graph(
            &graph_input,
            args,
            emitter_dyn,
            writer.as_ref().map(|w| w.session_dir()),
        )
        .await
    {
        Ok(r) => r,
        Err(err) => {
            // Stop the ticker and finalize live.json + manifest as failed.
            if let Some(h) = ticker_handle {
                h.abort();
            }
            if let Some(ref w) = writer {
                let exec_id = execution_id.as_deref();
                let graph_name = input.file_stem().and_then(|s| s.to_str());
                let _ = w.finalize_live_with_id(false, exec_id, graph_name);
            }
            // Also call emitter.finalize_live so elapsed/completed are preserved.
            if let Some(ref e) = emitter {
                let _ = e.finalize_live(false);
            }
            eprintln!("{}", err);
            return Err(anyhow::anyhow!("Execution failed"));
        }
    };

    // Stop the background ticker now that execution is done.
    if let Some(h) = ticker_handle {
        h.abort();
    }

    // Emit metrics JSON if requested
    #[allow(unused_mut)]
    let mut metrics_json = serde_json::json!({
        "input": input.display().to_string(),
        "optimization_level": format!("O{}", opt_level),
        "execution": {
            "nodes_executed": result.execution.stats.executed_nodes,
            "nodes_failed": result.execution.stats.failed_nodes,
            "duration_ms": result.execution.stats.duration_ms,
            "status": if result.execution.stats.failed_nodes == 0 { "success" } else { "partial_failure" }
        },
        "scheduler": result.execution.scheduler_metrics.to_json()
    });

    #[cfg(feature = "metrics")]
    {
        let llm_metrics = &result.execution.llm_metrics;
        metrics_json["llm"] = serde_json::json!({
            "total_requests": llm_metrics.total_requests,
            "total_input_tokens": llm_metrics.total_input_tokens,
            "total_output_tokens": llm_metrics.total_output_tokens,
            "avg_latency_ms": llm_metrics.average_latency.as_millis(),
            "p50_latency_ms": llm_metrics.p50_latency.as_millis(),
            "p99_latency_ms": llm_metrics.p99_latency.as_millis()
        });

        let link_metrics = &result.metrics;
        metrics_json["link_phases"] = serde_json::json!({
            "compile_ms": link_metrics.compile_time.as_secs_f64() * 1000.0,
            "runtime_ms": link_metrics.runtime_time.as_secs_f64() * 1000.0
        });
    }

    if let Some(metrics_path) = emit_metrics {
        std::fs::write(&metrics_path, serde_json::to_string_pretty(&metrics_json)?)
            .with_context(|| format!("Failed to write metrics to {}", metrics_path.display()))?;

        println!("Wrote metrics to {}", metrics_path.display());
    }

    // Finalize session output after execution
    if let Some(writer) = writer {
        let graph_name = input.file_stem().and_then(|s| s.to_str());
        let exec_id = execution_id.as_deref().unwrap_or("unknown");
        let stats = &result.execution.stats;

        // Query episodic entries for this execution
        let episodic_entries = linker
            .runtime_executor()
            .memory_system()
            .query_episodes(exec_id)
            .await
            .ok();

        writer
            .finalize(
                exec_id,
                graph_name,
                stats.duration_ms,
                stats.executed_nodes + stats.failed_nodes,
                stats.failed_nodes == 0,
                result.execution.all_outputs.as_ref(),
                result.execution.node_output_map.as_ref(),
                &result.execution.results,
                &metrics_json,
                &stats.node_statuses,
                episodic_entries.as_deref(),
            )
            .context("Failed to finalize session")?;

        eprintln!("Session complete: {}", writer.session_dir().display());
    }

    // Print workflow outputs
    if result.execution.results.is_empty() {
        eprintln!("Warning: No output values");
    } else {
        for (key, value) in &result.execution.results {
            // Skip printing null values (void operations like PRINT)
            if matches!(value, apxm_core::types::values::Value::Null) {
                continue;
            }

            if result.execution.results.len() == 1 {
                // Single result: print just the value
                println!("{}", value);
            } else {
                // Multiple results: print key=value pairs
                println!("{}={}", key, value);
            }
        }
    }

    Ok(())
}

#[cfg(feature = "driver")]
async fn run_command(
    input: PathBuf,
    args: Vec<String>,
    config: Option<PathBuf>,
    emit_metrics: Option<PathBuf>,
    emit_session: Option<Option<PathBuf>>,
) -> Result<()> {
    use apxm_artifact::Artifact;
    use apxm_driver::runtime::RuntimeExecutor;

    // Validate file extension
    if input.extension().and_then(|e| e.to_str())
        != Some(apxm_core::constants::extensions::ARTIFACT)
    {
        return Err(anyhow::anyhow!(
            "Expected .apxmobj artifact file. Use 'execute' command for graph source files."
        ));
    }

    // Load artifact
    let artifact_bytes = std::fs::read(&input)
        .with_context(|| format!("Failed to read artifact {}", input.display()))?;
    let artifact = Artifact::from_bytes(&artifact_bytes)
        .map_err(|e| anyhow::anyhow!("Failed to parse artifact: {}", e))?;

    // Initialize runtime
    let apxm_config = load_config(config).context("Failed to load configuration")?;
    let mut linker_config = LinkerConfig::from_apxm_config(apxm_config);

    // Enable all-outputs collection when session output is requested
    if emit_session.is_some() {
        linker_config
            .runtime_config
            .scheduler_config
            .collect_all_outputs = true;
    }

    let artifact_graph = if emit_session.is_some() {
        artifact.entry_dag().and_then(graph_from_execution_dag)
    } else {
        None
    };

    // Set up session output + live emitter BEFORE execution
    let (writer, emitter, execution_id) =
        setup_session(&emit_session, &input, "artifact", artifact_graph.as_ref())?;

    if let (Some(graph), Some(writer)) = (artifact_graph.as_ref(), writer.as_ref()) {
        linker_config.runtime_config.context_stack =
            context_stack_config_from_graph(writer.session_dir(), graph);
    }

    let runtime = RuntimeExecutor::new(&linker_config)
        .await
        .context("Failed to initialize runtime")?;

    // Coerce Arc<SessionEventEmitter> → Arc<dyn ExecutionEventEmitter>
    let emitter_dyn: Option<std::sync::Arc<dyn apxm_runtime::ExecutionEventEmitter>> = emitter
        .as_ref()
        .map(|e| e.clone() as std::sync::Arc<dyn apxm_runtime::ExecutionEventEmitter>);

    // Execute artifact with args + emitter
    let result = match runtime
        .execute_artifact_with_emitter(
            artifact,
            args,
            emitter_dyn,
            writer
                .as_ref()
                .map(|w| w.session_dir().to_string_lossy().to_string()),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            // Finalize live.json + manifest as failed.
            if let Some(ref w) = writer {
                let exec_id = execution_id.as_deref();
                let graph_name = input.file_stem().and_then(|s| s.to_str());
                let _ = w.finalize_live_with_id(false, exec_id, graph_name);
            }
            return Err(anyhow::anyhow!("Execution failed: {}", e));
        }
    };

    // Build metrics JSON
    let metrics_json = serde_json::json!({
        "input": input.display().to_string(),
        "execution": {
            "nodes_executed": result.stats.executed_nodes,
            "nodes_failed": result.stats.failed_nodes,
            "duration_ms": result.stats.duration_ms,
            "status": if result.stats.failed_nodes == 0 { "success" } else { "partial_failure" }
        },
        "scheduler": result.scheduler_metrics.to_json()
    });

    // Emit metrics JSON if requested
    if let Some(metrics_path) = emit_metrics {
        std::fs::write(&metrics_path, serde_json::to_string_pretty(&metrics_json)?)
            .with_context(|| format!("Failed to write metrics to {}", metrics_path.display()))?;

        println!("Wrote metrics to {}", metrics_path.display());
    }

    // Finalize session output after execution
    if let Some(writer) = writer {
        let graph_name = input.file_stem().and_then(|s| s.to_str());
        let exec_id = execution_id.as_deref().unwrap_or("unknown");
        let stats = &result.stats;

        // Query episodic entries for this execution
        let episodic_entries = runtime
            .memory_system()
            .query_episodes(exec_id)
            .await
            .ok();

        writer
            .finalize(
                exec_id,
                graph_name,
                stats.duration_ms,
                stats.executed_nodes + stats.failed_nodes,
                stats.failed_nodes == 0,
                result.all_outputs.as_ref(),
                result.node_output_map.as_ref(),
                &result.results,
                &metrics_json,
                &stats.node_statuses,
                episodic_entries.as_deref(),
            )
            .context("Failed to finalize session")?;

        eprintln!("Session complete: {}", writer.session_dir().display());
    }

    Ok(())
}

#[allow(dead_code)] // Ollama integration - reserved for future use
const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";

/// Fetch installed models from a running Ollama instance and register them.
///
/// Returns `(added, skipped)` counts. `existing` model IDs are skipped.
#[allow(dead_code)] // Ollama integration - not yet wired to backend commands
fn ollama_model_caps(base_url: &str, model_name: &str) -> (bool, bool, usize) {
    // Query /api/show for real capabilities — no hardcoding model family names.
    // Returns (supports_functions, supports_vision, context_window).
    let client = reqwest::blocking::Client::new();
    let Ok(resp) = client
        .post(&format!("{base_url}/api/show"))
        .json(&serde_json::json!({"model": model_name}))
        .send()
    else {
        return (false, false, 128000);
    };
    if !resp.status().is_success() {
        return (false, false, 128000);
    }
    let Ok(json) = resp.json::<serde_json::Value>() else {
        return (false, false, 128000);
    };

    // capabilities: ["completion", "tools", "vision", "thinking", ...]
    let caps = json
        .get("capabilities")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    let supports_functions = caps.iter().any(|c| c.as_str() == Some("tools"));
    let supports_vision = caps.iter().any(|c| c.as_str() == Some("vision"));

    // context_window from model_info — key ends with ".context_length"
    let ctx_window = json
        .get("model_info")
        .and_then(|info| info.as_object())
        .and_then(|obj| {
            obj.iter()
                .find(|(k, _)| k.ends_with(".context_length"))
                .and_then(|(_, v)| v.as_u64())
                .map(|n| n as usize)
        })
        .unwrap_or(128000);

    (supports_functions, supports_vision, ctx_window)
}

#[allow(dead_code)] // Ollama integration - not yet wired to backend commands
fn sync_ollama_models(
    store: &apxm_credentials::backend::BackendStore,
    backend_name: &str,
    base_url: &str,
    existing: &std::collections::HashSet<String>,
) -> Result<(usize, usize)> {
    let resp = reqwest::blocking::get(&format!("{base_url}/api/tags")).map_err(|e| {
        anyhow::anyhow!(
            "Cannot reach Ollama at {base_url}: {e}\nMake sure it is running: ollama serve"
        )
    })?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "Ollama API error ({}): {}",
            resp.status(),
            resp.text().unwrap_or_default()
        );
    }

    let body: serde_json::Value = resp
        .json()
        .map_err(|e| anyhow::anyhow!("Failed to parse Ollama response: {e}"))?;

    let models_arr = body
        .get("models")
        .and_then(|m| m.as_array())
        .ok_or_else(|| anyhow::anyhow!("Unexpected Ollama response format"))?;

    let mut added = 0usize;
    let mut skipped = 0usize;

    for m in models_arr {
        if let Some(model_name) = m.get("name").and_then(|n| n.as_str()) {
            if existing.contains(model_name) {
                skipped += 1;
                continue;
            }
            let (supports_functions, supports_vision, ctx_window) =
                ollama_model_caps(base_url, model_name);
            let model = apxm_core::types::ModelConfig {
                id: model_name.to_string(),
                aliases: vec![],
                context_window: ctx_window,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                supports_vision,
                supports_functions,
                supports_thinking: false,
                tags: vec!["local".to_string(), "ollama".to_string()],
            };
            store
                .add_model(backend_name, model)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            added += 1;
        }
    }

    Ok((added, skipped))
}

#[allow(dead_code)] // Backend CLI - planned replacement for current implementation
async fn backend_command(action: BackendAction, json_output: bool) -> Result<()> {
    use apxm_core::types::{BackendConfig, BackendType, ProviderProtocol};
    use apxm_credentials::backend::BackendStore;
    use std::str::FromStr;

    let store = BackendStore::open().map_err(|e| anyhow::anyhow!("{e}"))?;

    match action {
        BackendAction::Add {
            name,
            r#type,
            protocol,
            endpoint,
            api_key,
            header,
        } => {
            // Parse protocol first (needed for Ollama smart defaults)
            let protocol =
                ProviderProtocol::from_str(&protocol).map_err(|e| anyhow::anyhow!("{e}"))?;
            let is_ollama = protocol == ProviderProtocol::Ollama;

            // Ollama smart defaults: type=local if not specified
            let backend_type = if r#type.is_empty() && is_ollama {
                BackendType::Local
            } else if r#type.is_empty() {
                anyhow::bail!("--type is required (cloud, onprem, or local)");
            } else {
                BackendType::from_str(&r#type).map_err(|e| anyhow::anyhow!("{e}"))?
            };

            // Ollama smart defaults: endpoint
            let endpoint = if endpoint.is_none() && is_ollama {
                Some(DEFAULT_OLLAMA_ENDPOINT.to_string())
            } else {
                endpoint
            };

            // API key not needed for Ollama or local backends
            let api_key = if api_key.is_some() || backend_type == BackendType::Local || is_ollama {
                api_key
            } else {
                eprint!("Enter API key for {name} (or press Enter to skip): ");
                let key = rpassword::read_password()
                    .map_err(|e| anyhow::anyhow!("Failed to read API key: {e}"))?;
                if key.is_empty() { None } else { Some(key) }
            };

            let headers: std::collections::HashMap<String, String> = header.into_iter().collect();

            let backend = BackendConfig {
                name: name.clone(),
                backend_type,
                protocol,
                endpoint: endpoint.clone(),
                api_key,
                headers,
                models: vec![],
                docker: None,
            };

            store.add(backend).map_err(|e| anyhow::anyhow!("{e}"))?;

            // For Ollama: best-effort auto-sync of installed models
            let synced_count = if is_ollama {
                let base = endpoint.as_deref().unwrap_or(DEFAULT_OLLAMA_ENDPOINT);
                let empty = HashSet::new();
                sync_ollama_models(&store, &name, base, &empty)
                    .map(|(added, _)| added)
                    .unwrap_or(0)
            } else {
                0
            };

            if json_output {
                println!(
                    "{{\"status\":\"ok\",\"backend\":\"{name}\",\"synced_models\":{synced_count}}}"
                );
            } else {
                print_section_header("Backend Registered");
                print_status_line("Name", Status::Ok, &name);
                print_status_line("Type", Status::Ok, &format!("{backend_type}"));
                print_status_line("Protocol", Status::Ok, &format!("{protocol}"));
                print_status_line("Store", Status::Ok, &store.path().display().to_string());
                if is_ollama {
                    if synced_count > 0 {
                        print_status_line(
                            "Models synced",
                            Status::Ok,
                            &format!("{synced_count} installed models registered"),
                        );
                    } else {
                        print_status_line(
                            "Models",
                            Status::Warning,
                            "Ollama not reachable — run: apxm backend sync-models after starting Ollama",
                        );
                    }
                }
            }
        }
        BackendAction::List { format } => {
            let backends = store.list().map_err(|e| anyhow::anyhow!("{e}"))?;

            if json_output || format == "json" {
                println!("{}", serde_json::to_string_pretty(&backends)?);
                return Ok(());
            }

            if backends.is_empty() {
                println!("No backends registered.");
                println!("Add one with: apxm backend add <name> --type cloud --protocol openai");
                return Ok(());
            }

            print_section_header("Registered Backends");
            for backend in &backends {
                let key_display = backend
                    .api_key
                    .as_deref()
                    .map(|k| apxm_credentials::mask::mask_key(k))
                    .unwrap_or_else(|| "<none>".to_string());

                println!(
                    "  {:<16} {:<8} {:<10} key={}{}{}",
                    backend.name.bold(),
                    format!("{}", backend.backend_type),
                    format!("{}", backend.protocol),
                    key_display,
                    backend
                        .endpoint
                        .as_ref()
                        .map(|e| format!("  endpoint={e}"))
                        .unwrap_or_default(),
                    if !backend.models.is_empty() {
                        format!("  +{} models", backend.models.len())
                    } else {
                        String::new()
                    }
                );
            }
            println!();
            println!("Store: {}", store.path().display());
        }
        BackendAction::Remove { name } => {
            store.remove(&name).map_err(|e| anyhow::anyhow!("{e}"))?;

            if json_output {
                println!("{{\"status\":\"ok\",\"action\":\"removed\",\"backend\":\"{name}\"}}");
            } else {
                print_section_header("Backend Removed");
                print_status_line(&name, Status::Ok, "removed");
            }
        }
        BackendAction::Test { name } => {
            let backends_to_test: Vec<BackendConfig> = match name {
                Some(ref n) => {
                    let backend = store
                        .get(n)
                        .map_err(|e| anyhow::anyhow!("{e}"))?
                        .ok_or_else(|| anyhow::anyhow!("Backend '{n}' not found"))?;
                    vec![backend]
                }
                None => store.list().map_err(|e| anyhow::anyhow!("{e}"))?,
            };

            if backends_to_test.is_empty() {
                println!("No backends to test.");
                return Ok(());
            }

            print_section_header("Testing Backends");
            let mut all_ok = true;
            for backend in &backends_to_test {
                match apxm_credentials::validate::validate_backend(backend).await {
                    Ok(msg) => print_status_line(&backend.name, Status::Ok, &msg),
                    Err(e) => {
                        print_status_line(&backend.name, Status::Error, &e.to_string());
                        all_ok = false;
                    }
                }
            }
            if !all_ok {
                return Err(anyhow::anyhow!("Some backends failed validation"));
            }
        }
        BackendAction::Migrate { yes } => {
            if !yes {
                eprintln!(
                    "This will migrate credentials from ~/.apxm/credentials.toml to ~/.apxm/config.toml"
                );
                eprint!("Continue? [y/N] ");
                use std::io::{self, BufRead};
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line)?;
                if !line.trim().eq_ignore_ascii_case("y") {
                    println!("Aborted.");
                    return Ok(());
                }
            }

            let count = store
                .migrate_from_credentials()
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            if json_output {
                println!("{{\"status\":\"ok\",\"migrated\":{count}}}");
            } else {
                print_section_header("Migration Complete");
                print_status_line("Migrated", Status::Ok, &format!("{count} credentials"));
                if count > 0 {
                    println!();
                    println!("Your legacy credentials.toml can now be safely removed.");
                    println!("To view the migrated backends: apxm backend list");
                }
            }
        }
        BackendAction::Start { name } => {
            let backend = store
                .get(&name)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .ok_or_else(|| anyhow::anyhow!("Backend '{name}' not found"))?;

            let container_id = DockerManager::start(&backend)
                .map_err(|e| anyhow::anyhow!("Failed to start backend: {e}"))?;

            if json_output {
                println!(
                    "{{\"status\":\"ok\",\"backend\":\"{name}\",\"container_id\":\"{container_id}\"}}"
                );
            } else {
                print_section_header("Backend Started");
                print_status_line("Backend", Status::Ok, &name);
                print_status_line("Container ID", Status::Ok, &container_id);
            }
        }
        BackendAction::Stop { name } => {
            DockerManager::stop_by_name(&name)
                .map_err(|e| anyhow::anyhow!("Failed to stop backend: {e}"))?;

            if json_output {
                println!("{{\"status\":\"ok\",\"backend\":\"{name}\",\"action\":\"stopped\"}}");
            } else {
                print_section_header("Backend Stopped");
                print_status_line("Backend", Status::Ok, &name);
            }
        }
        BackendAction::Status { name } => {
            let backends_to_check: Vec<BackendConfig> = match name {
                Some(ref n) => {
                    let backend = store
                        .get(n)
                        .map_err(|e| anyhow::anyhow!("{e}"))?
                        .ok_or_else(|| anyhow::anyhow!("Backend '{n}' not found"))?;
                    vec![backend]
                }
                None => store.list().map_err(|e| anyhow::anyhow!("{e}"))?,
            };

            if json_output {
                let mut statuses = Vec::new();
                for backend in &backends_to_check {
                    if backend.backend_type == BackendType::Local {
                        let status = DockerManager::status(&backend.name)
                            .unwrap_or(ContainerStatus::NotFound);
                        statuses.push(serde_json::json!({
                            "backend": backend.name,
                            "status": status.to_string()
                        }));
                    }
                }
                println!("{}", serde_json::to_string_pretty(&statuses)?);
            } else {
                print_section_header("Backend Status");
                for backend in &backends_to_check {
                    if backend.backend_type == BackendType::Local {
                        let status = DockerManager::status(&backend.name)
                            .unwrap_or(ContainerStatus::NotFound);
                        let status_display = match status {
                            ContainerStatus::Running => Status::Ok,
                            ContainerStatus::Stopped => Status::Warning,
                            ContainerStatus::NotFound => Status::Error,
                        };
                        print_status_line(&backend.name, status_display, &status.to_string());
                    }
                }
            }
        }
        BackendAction::Logs { name, tail } => {
            let container_id = DockerManager::get_container_id(&name)
                .map_err(|e| anyhow::anyhow!("Failed to get container ID: {e}"))?
                .ok_or_else(|| anyhow::anyhow!("Container not found for backend '{name}'"))?;

            let logs = DockerManager::logs(&container_id, tail)
                .map_err(|e| anyhow::anyhow!("Failed to get logs: {e}"))?;

            if json_output {
                println!(
                    "{{\"status\":\"ok\",\"backend\":\"{name}\",\"logs\":{}}}",
                    serde_json::to_string(&logs)?
                );
            } else {
                println!("{}", logs);
            }
        }
        BackendAction::Restart { name } => {
            let container_id = DockerManager::get_container_id(&name)
                .map_err(|e| anyhow::anyhow!("Failed to get container ID: {e}"))?
                .ok_or_else(|| anyhow::anyhow!("Container not found for backend '{name}'"))?;

            DockerManager::restart(&container_id)
                .map_err(|e| anyhow::anyhow!("Failed to restart backend: {e}"))?;

            if json_output {
                println!("{{\"status\":\"ok\",\"backend\":\"{name}\",\"action\":\"restarted\"}}");
            } else {
                print_section_header("Backend Restarted");
                print_status_line("Backend", Status::Ok, &name);
            }
        }
        BackendAction::SyncModels { name, endpoint } => {
            let backend_cfg = store
                .get(&name)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .ok_or_else(|| anyhow::anyhow!("Backend '{}' not found", name))?;

            if backend_cfg.protocol != ProviderProtocol::Ollama {
                anyhow::bail!(
                    "sync-models only works for Ollama backends. '{}' uses protocol '{}'.",
                    name,
                    backend_cfg.protocol
                );
            }

            let base = endpoint
                .or_else(|| backend_cfg.endpoint.clone())
                .unwrap_or_else(|| DEFAULT_OLLAMA_ENDPOINT.to_string());

            let existing: HashSet<String> =
                backend_cfg.models.iter().map(|m| m.id.clone()).collect();

            let (added, skipped) = sync_ollama_models(&store, &name, &base, &existing)?;

            if json_output {
                println!(
                    "{{\"status\":\"ok\",\"backend\":\"{name}\",\"added\":{added},\"skipped\":{skipped}}}"
                );
            } else {
                print_section_header("Ollama Models Synced");
                print_status_line("Backend", Status::Ok, &name);
                print_status_line("Endpoint", Status::Ok, &base);
                if added > 0 {
                    print_status_line("Added", Status::Ok, &format!("{added} new models"));
                }
                if skipped > 0 {
                    print_status_line(
                        "Skipped",
                        Status::Warning,
                        &format!("{skipped} already registered"),
                    );
                }
                if added == 0 && skipped == 0 {
                    println!("  No models found. Install one with: ollama pull llama3.3");
                }
            }
        }
        BackendAction::AddModel {
            backend,
            model_id,
            alias,
            context_window,
            cost_input,
            cost_output,
            supports_vision,
            supports_functions,
            supports_thinking,
            tag,
        } => {
            use apxm_core::types::ModelConfig;

            let model = ModelConfig {
                id: model_id.clone(),
                aliases: alias,
                context_window,
                cost_per_1k_input: cost_input,
                cost_per_1k_output: cost_output,
                supports_vision,
                supports_functions,
                supports_thinking,
                tags: tag,
            };

            store
                .add_model(&backend, model)
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            if json_output {
                println!(
                    "{{\"status\":\"ok\",\"backend\":\"{backend}\",\"model\":\"{model_id}\"}}"
                );
            } else {
                print_section_header("Model Added");
                print_status_line("Backend", Status::Ok, &backend);
                print_status_line("Model", Status::Ok, &model_id);
            }
        }
    }

    Ok(())
}

#[cfg(feature = "driver")]
fn build_semantic_context() -> apxm_graph::semantic::SemanticContext {
    use apxm_core::constants::capabilities;
    use apxm_core::types::identifiers::{BackendId, CapabilityName, ModelId, ProfileId};
    use apxm_graph::semantic::SemanticContext;

    let backends = apxm_credentials::BackendStore::open()
        .and_then(|s| s.list())
        .unwrap_or_default();

    let registry = apxm_acp::AgentRegistry::load();
    let profiles_list = registry.list();

    let model_count: usize = backends
        .iter()
        .map(|b| b.models.len() + b.models.iter().map(|m| m.aliases.len()).sum::<usize>())
        .sum();

    let mut ctx = SemanticContext::with_capacity(
        profiles_list.len(),
        backends.len(),
        model_count,
        capabilities::BUILTINS.len(),
    );

    for (name, _, _) in &profiles_list {
        ctx.profiles.insert(ProfileId::from(name.as_str()));
    }

    for b in &backends {
        ctx.backends.insert(BackendId::from(b.name.as_str()));
        for m in &b.models {
            ctx.models.insert(ModelId::from(m.id.as_str()));
            for a in &m.aliases {
                ctx.models.insert(ModelId::from(a.as_str()));
            }
        }
    }

    for name in capabilities::BUILTINS {
        ctx.capabilities.insert(CapabilityName::from(*name));
    }

    ctx
}

#[allow(unused_variables)]
fn validate_command(input: PathBuf, json_output: bool, no_check_resources: bool) -> Result<()> {
    use apxm_core::types::AIS_OPERATIONS;
    use std::collections::HashSet;

    #[derive(serde::Deserialize)]
    struct RawGraph {
        #[serde(default)]
        name: String,
        #[serde(default)]
        nodes: Vec<RawNode>,
        #[serde(default)]
        edges: Vec<RawEdge>,
        #[serde(default)]
        parameters: Vec<RawParam>,
    }

    #[derive(serde::Deserialize)]
    struct RawNode {
        #[serde(default)]
        id: u64,
        #[serde(default)]
        name: String,
        #[serde(default)]
        op: String,
        #[serde(default)]
        attributes: HashMap<String, serde_json::Value>,
    }

    #[derive(serde::Deserialize)]
    struct RawEdge {
        #[serde(default)]
        from: u64,
        #[serde(default)]
        to: u64,
        #[serde(default = "RawEdge::default_dependency")]
        dependency: String,
    }

    impl RawEdge {
        fn default_dependency() -> String {
            "Data".to_string()
        }
    }

    #[derive(serde::Deserialize)]
    struct RawParam {
        #[serde(default)]
        name: String,
        #[serde(default)]
        type_name: String,
    }

    let content = std::fs::read_to_string(&input)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", input.display()))?;

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // Check file extension - .air files need to be parsed differently
    let is_air = input.extension().and_then(|e| e.to_str()) == Some("air");

    let raw: RawGraph = if is_air {
        // Parse .air format into ApxmGraph, then serialize back to JSON for validation
        #[cfg(feature = "driver")]
        {
            let graph = apxm_graph::ApxmGraph::from_air(&content)
                .map_err(|e| anyhow::anyhow!("Failed to parse .air file {}: {e}", input.display()))?;
            let json = graph.to_json()
                .map_err(|e| anyhow::anyhow!("Failed to serialize graph: {e}"))?;
            serde_json::from_str(&json)
                .map_err(|e| anyhow::anyhow!("Internal error serializing graph: {e}"))?
        }
        #[cfg(not(feature = "driver"))]
        {
            return Err(anyhow::anyhow!(
                "Driver feature required to validate .air files. Build with --features driver."
            ));
        }
    } else {
        serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Invalid JSON in {}: {e}", input.display()))?
    };

    if raw.name.is_empty() {
        errors.push("graph name must not be empty".to_string());
    }

    if raw.nodes.is_empty() {
        errors.push("graph must contain at least one node".to_string());
    }

    let mut node_ids: HashSet<u64> = HashSet::new();
    let valid_ops: HashSet<String> = AIS_OPERATIONS
        .iter()
        .map(|s| s.op_type.to_string())
        .collect();

    for node in &raw.nodes {
        let id = node.id;
        let name = &node.name;
        let op = &node.op;

        if id == 0 {
            errors.push(format!("node '{name}' has invalid id (0 or missing)"));
        }
        if !node_ids.insert(id) {
            errors.push(format!("duplicate node id {id}"));
        }
        if name.is_empty() {
            errors.push(format!("node id={id} has empty name"));
        }
        if op.is_empty() {
            errors.push(format!("node '{name}' (id={id}) has empty op"));
        } else if !valid_ops.contains(op.as_str()) {
            errors.push(format!(
                "node '{name}' (id={id}) has unknown op '{op}'. Run 'apxm ops list' for valid ops."
            ));
        } else {
            let spec = AIS_OPERATIONS.iter().find(|s| s.op_type.to_string() == *op);
            if let Some(spec) = spec {
                for field in spec.fields.iter().filter(|f| f.required) {
                    if !node.attributes.contains_key(field.name) {
                        errors.push(format!(
                            "node '{name}' (id={id}, op={op}) missing required attribute '{}'",
                            field.name
                        ));
                    }
                }
            }
        }
    }

    for edge in &raw.edges {
        let from = edge.from;
        let to = edge.to;
        let dep = &edge.dependency;

        if from == to {
            errors.push(format!("edge {from}->{to} is a self-loop"));
        }
        if !matches!(dep.as_str(), "Data" | "Control" | "Effect") {
            errors.push(format!(
                "edge {from}->{to} has invalid dependency type '{dep}'"
            ));
        }
        if !node_ids.contains(&from) {
            errors.push(format!("edge references non-existent source node {from}"));
        }
        if !node_ids.contains(&to) {
            errors.push(format!("edge references non-existent target node {to}"));
        }
    }

    if !node_ids.is_empty() && !raw.edges.is_empty() {
        let mut in_degree: HashMap<u64, usize> = node_ids.iter().map(|&id| (id, 0)).collect();
        let mut adjacency: HashMap<u64, Vec<u64>> = HashMap::new();

        for edge in &raw.edges {
            let from = edge.from;
            let to = edge.to;
            if node_ids.contains(&from) && node_ids.contains(&to) {
                adjacency.entry(from).or_default().push(to);
                *in_degree.entry(to).or_insert(0) += 1;
            }
        }

        let mut queue: std::collections::VecDeque<u64> = in_degree
            .iter()
            .filter_map(|(&id, &deg)| if deg == 0 { Some(id) } else { None })
            .collect();
        let mut visited = 0usize;
        while let Some(node_id) = queue.pop_front() {
            visited += 1;
            if let Some(neighbors) = adjacency.get(&node_id) {
                for &neighbor in neighbors {
                    if let Some(current) = in_degree.get_mut(&neighbor) {
                        *current = current.saturating_sub(1);
                        if *current == 0 {
                            queue.push_back(neighbor);
                        }
                    }
                }
            }
        }
        if visited != node_ids.len() {
            errors.push(format!(
                "graph contains a cycle ({} nodes involved)",
                node_ids.len() - visited
            ));
        }
    }

    let valid_types: HashSet<&str> = ["str", "int", "float", "bool", "json"]
        .into_iter()
        .collect();
    let mut param_names: HashSet<String> = HashSet::new();
    for param in &raw.parameters {
        let pname = &param.name;
        let ptype = &param.type_name;
        if pname.is_empty() {
            errors.push("parameter with empty name".to_string());
        }
        if !param_names.insert(pname.to_string()) {
            errors.push(format!("duplicate parameter name '{pname}'"));
        }
        if !valid_types.contains(ptype.as_str()) {
            warnings.push(format!(
                "parameter '{pname}' has non-standard type_name '{ptype}'"
            ));
        }
    }

    // Also attempt full Rust-side parse+validate for deeper checks (when driver feature available)
    #[allow(unused_mut)]
    let mut semantic_errors: Vec<String> = Vec::new();
    #[allow(unused_mut)]
    let mut semantic_warnings: Vec<String> = Vec::new();
    #[cfg(feature = "driver")]
    {
        let parse_result = if is_air {
            apxm_graph::ApxmGraph::from_air(&content)
        } else {
            apxm_graph::ApxmGraph::from_json(&content)
        };

        match parse_result {
            Ok(graph) => {
                // Semantic validation: Tier 1 always, Tier 2 unless --no-check-resources
                let ctx = if no_check_resources {
                    apxm_graph::semantic::SemanticContext::default()
                } else {
                    build_semantic_context()
                };
                let sem_errors = apxm_graph::semantic::validate_semantic(&graph, &ctx);
                for err in &sem_errors {
                    let msg = err.short_message();
                    if err.code.is_warning() {
                        semantic_warnings.push(msg);
                    } else {
                        semantic_errors.push(msg);
                    }
                }
            }
            Err(e) => {
                let msg = e.to_string();
                if !errors
                    .iter()
                    .any(|existing| msg.contains(&existing[..existing.len().min(30)]))
                {
                    errors.push(format!("graph validation: {msg}"));
                }
            }
        }
    }

    errors.extend(semantic_errors);
    warnings.extend(semantic_warnings);
    let valid = errors.is_empty();

    if json_output {
        let result = serde_json::json!({
            "file": input.display().to_string(),
            "valid": valid,
            "errors": errors,
            "warnings": warnings,
        });
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
        if !valid {
            return Err(anyhow::anyhow!("{} error(s) found", errors.len()));
        }
    } else if valid {
        print_status_line(&input.display().to_string(), Status::Ok, "valid");
        if !warnings.is_empty() {
            for w in &warnings {
                println!(
                    "  {} {}",
                    apxm_core::constants::ui::icons::CAUTION.yellow(),
                    w
                );
            }
        }
    } else {
        print_status_line(&input.display().to_string(), Status::Error, "invalid");
        for e in &errors {
            println!("  {} {}", apxm_core::constants::ui::icons::FAILED.red(), e);
        }
        for w in &warnings {
            println!(
                "  {} {}",
                apxm_core::constants::ui::icons::CAUTION.yellow(),
                w
            );
        }
        return Err(anyhow::anyhow!("{} error(s) found", errors.len()));
    }

    Ok(())
}

/// Parsed graph topology used by analyze and explain commands.
struct GraphAnalysis<'a> {
    graph: &'a apxm_graph::ApxmGraph,
    node_index: HashMap<u64, usize>,
    edge_count: usize,
    node_ids: HashSet<u64>,
    successors: HashMap<u64, Vec<u64>>,
    predecessors: HashMap<u64, Vec<u64>>,
    entry_nodes: Vec<u64>,
    exit_nodes: Vec<u64>,
    phases: Vec<Vec<u64>>,
}

impl<'a> GraphAnalysis<'a> {
    fn from_graph(graph: &'a apxm_graph::ApxmGraph) -> Self {
        let node_index: HashMap<u64, usize> = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i))
            .collect();
        let node_ids: HashSet<u64> = graph.nodes.iter().map(|n| n.id).collect();
        let edge_count = graph.edges.len();

        let mut successors: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut predecessors: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut in_degree: HashMap<u64, usize> = node_ids.iter().map(|&id| (id, 0)).collect();

        for edge in &graph.edges {
            successors.entry(edge.from).or_default().push(edge.to);
            predecessors.entry(edge.to).or_default().push(edge.from);
            *in_degree.entry(edge.to).or_insert(0) += 1;
        }

        let entry_nodes: Vec<u64> = in_degree
            .iter()
            .filter_map(|(&id, &deg)| if deg == 0 { Some(id) } else { None })
            .collect();

        let exit_nodes: Vec<u64> = node_ids
            .iter()
            .filter(|&&id| successors.get(&id).is_none_or(|s| s.is_empty()))
            .copied()
            .collect();

        let mut phases: Vec<Vec<u64>> = Vec::new();
        let mut remaining_in: HashMap<u64, usize> = in_degree.clone();
        let mut current_layer: Vec<u64> = entry_nodes.clone();
        current_layer.sort();

        while !current_layer.is_empty() {
            phases.push(current_layer.clone());
            let mut next_layer = Vec::new();
            for &nid in &current_layer {
                if let Some(succs) = successors.get(&nid) {
                    for &succ in succs {
                        if let Some(deg) = remaining_in.get_mut(&succ) {
                            *deg = deg.saturating_sub(1);
                            if *deg == 0 {
                                next_layer.push(succ);
                            }
                        }
                    }
                }
            }
            next_layer.sort();
            next_layer.dedup();
            current_layer = next_layer;
        }

        Self {
            graph,
            node_index,
            edge_count,
            node_ids,
            successors,
            predecessors,
            entry_nodes,
            exit_nodes,
            phases,
        }
    }

    fn node_by_id(&self, id: u64) -> Option<&apxm_graph::GraphNode> {
        self.node_index.get(&id).map(|&i| &self.graph.nodes[i])
    }

    fn node_op(&self, id: u64) -> String {
        self.node_by_id(id)
            .map(|n| n.op.to_string())
            .unwrap_or_else(|| "?".to_string())
    }

    fn node_name(&self, id: u64) -> &str {
        self.node_by_id(id).map(|n| n.name.as_str()).unwrap_or("?")
    }

    fn node_latency_ms(&self, id: u64) -> u64 {
        op_latency_ms(&self.node_op(id))
    }

    fn max_parallelism(&self) -> usize {
        self.phases.iter().map(|p| p.len()).max().unwrap_or(1)
    }

    fn parallel_ms(&self) -> u64 {
        self.phases
            .iter()
            .map(|layer| {
                layer
                    .iter()
                    .map(|&id| self.node_latency_ms(id))
                    .max()
                    .unwrap_or(0)
            })
            .sum()
    }

    fn sequential_ms(&self) -> u64 {
        self.node_ids
            .iter()
            .map(|&id| self.node_latency_ms(id))
            .sum()
    }

    fn speedup(&self) -> f64 {
        let par = self.parallel_ms();
        if par > 0 {
            self.sequential_ms() as f64 / par as f64
        } else {
            1.0
        }
    }

    fn critical_path(&self) -> (Vec<u64>, u64) {
        let mut dist: HashMap<u64, u64> = HashMap::new();
        let mut prev: HashMap<u64, u64> = HashMap::new();
        for phase in &self.phases {
            for &nid in phase {
                let lat = self.node_latency_ms(nid);
                let max_pred = self
                    .predecessors
                    .get(&nid)
                    .and_then(|preds| preds.iter().filter_map(|&p| dist.get(&p)).max().copied())
                    .unwrap_or(0);
                dist.insert(nid, max_pred + lat);
                if let Some(preds) = self.predecessors.get(&nid)
                    && let Some(&best) = preds.iter().max_by_key(|&&p| dist.get(&p).unwrap_or(&0))
                {
                    prev.insert(nid, best);
                }
            }
        }

        let critical_end = dist.iter().max_by_key(|&(_, &d)| d).map(|(&id, _)| id);
        let mut path = Vec::new();
        if let Some(mut node) = critical_end {
            path.push(node);
            while let Some(&p) = prev.get(&node) {
                path.push(p);
                node = p;
            }
            path.reverse();
        }
        let ms = path.iter().map(|&id| self.node_latency_ms(id)).sum();
        (path, ms)
    }
}

fn analyze_command(input: PathBuf, json_output: bool) -> Result<()> {
    let content = std::fs::read_to_string(&input)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", input.display()))?;
    let graph: apxm_graph::ApxmGraph = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Invalid JSON in {}: {e}", input.display()))?;

    let ga = GraphAnalysis::from_graph(&graph);
    let (critical_path, critical_ms) = ga.critical_path();
    let sequential_ms = ga.sequential_ms();
    let parallel_ms = ga.parallel_ms();
    let max_parallelism = ga.max_parallelism();
    let speedup = ga.speedup();

    // Build suggestions (used by both JSON and human-readable output)
    let mut suggestions: Vec<String> = Vec::new();
    if max_parallelism > 1 {
        let parallel_phases: Vec<usize> = ga
            .phases
            .iter()
            .enumerate()
            .filter(|(_, p)| p.len() > 1)
            .map(|(i, _)| i + 1)
            .collect();
        suggestions.push(format!(
            "Phases {:?} can execute in parallel (up to {} concurrent operations)",
            parallel_phases, max_parallelism
        ));
    } else {
        suggestions.push("Graph is fully sequential — no parallelism opportunities".to_string());
    }
    if speedup > 1.2 {
        suggestions.push(format!(
            "Estimated {:.1}x speedup from parallel execution vs sequential",
            speedup
        ));
    }
    if critical_path.len() >= 3
        && let Some(&bn) = critical_path
            .iter()
            .max_by_key(|&&id| ga.node_latency_ms(id))
    {
        suggestions.push(format!(
            "Critical path bottleneck: node {} ('{}', op={})",
            bn,
            ga.node_name(bn),
            ga.node_op(bn)
        ));
    }

    if json_output {
        let phase_json: Vec<serde_json::Value> = ga.phases.iter().enumerate().map(|(i, layer)| {
            let max_lat = layer.iter().map(|&id| ga.node_latency_ms(id)).max().unwrap_or(0);
            let node_details: Vec<serde_json::Value> = layer.iter().map(|&id| {
                serde_json::json!({"id": id, "name": ga.node_name(id), "op": ga.node_op(id), "latency_ms": ga.node_latency_ms(id)})
            }).collect();
            serde_json::json!({
                "phase": i + 1,
                "parallel": layer.len() > 1,
                "parallelism_degree": layer.len(),
                "estimated_ms": max_lat,
                "nodes": node_details,
            })
        }).collect();

        let result = serde_json::json!({
            "file": input.display().to_string(),
            "graph_name": ga.graph.name,
            "node_count": ga.graph.nodes.len(),
            "edge_count": ga.edge_count,
            "entry_nodes": ga.entry_nodes,
            "exit_nodes": ga.exit_nodes,
            "depth": ga.phases.len(),
            "max_parallelism": max_parallelism,
            "execution_phases": phase_json,
            "critical_path": {
                "nodes": critical_path,
                "length": critical_path.len(),
                "estimated_ms": critical_ms,
            },
            "speedup": {
                "sequential_ms": sequential_ms,
                "parallel_ms": parallel_ms,
                "estimated_speedup": format!("{:.2}x", speedup),
            },
            "suggestions": suggestions,
        });
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        print_section_header(&format!("Analysis: {}", ga.graph.name));
        println!(
            "  {} nodes, {} edges, {} phases, max parallelism {}",
            ga.graph.nodes.len(),
            ga.edge_count,
            ga.phases.len(),
            max_parallelism
        );
        println!();

        for (i, layer) in ga.phases.iter().enumerate() {
            let tag = if layer.len() > 1 {
                format!("({}x parallel)", layer.len()).green().to_string()
            } else {
                "(sequential)".dimmed().to_string()
            };
            println!(
                "  {} Phase {} {}",
                apxm_core::constants::ui::icons::STARTED.cyan(),
                i + 1,
                tag
            );
            for &id in layer {
                println!(
                    "    {} {} {} [{}ms]",
                    format!("#{id}").dimmed(),
                    ga.node_name(id).bold(),
                    ga.node_op(id).cyan(),
                    ga.node_latency_ms(id)
                );
            }
        }

        println!();
        println!(
            "  {} Critical path: {} nodes, ~{}ms",
            apxm_core::constants::ui::icons::LIGHTNING.yellow(),
            critical_path.len(),
            critical_ms
        );
        println!(
            "  {} Speedup: {:.2}x (sequential {}ms {} parallel {}ms)",
            apxm_core::constants::ui::icons::ROCKET,
            speedup,
            sequential_ms,
            apxm_core::constants::ui::icons::ARROW_RIGHT,
            parallel_ms
        );

        if !suggestions.is_empty() {
            println!();
            println!("  {}", "Suggestions:".bold());
            for s in &suggestions {
                println!(
                    "    {} {s}",
                    apxm_core::constants::ui::icons::BULLET.dimmed()
                );
            }
        }
    }

    Ok(())
}

fn explain_command(target: &str, json_output: bool) -> Result<()> {
    // Check if target looks like an error code (e.g., E511, e511, 511)
    let trimmed = target.trim();
    let is_error_code = trimmed.starts_with('E')
        || trimmed.starts_with('e')
        || trimmed.chars().all(|c| c.is_ascii_digit());

    if is_error_code {
        // Extract the numeric part
        let code_str = if trimmed.starts_with('E') || trimmed.starts_with('e') {
            &trimmed[1..]
        } else {
            trimmed
        };

        let code_num: u32 = code_str
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid error code: {}", target))?;

        let error_code = apxm_core::error::ErrorCode::from_u32(code_num)
            .ok_or_else(|| anyhow::anyhow!("Unknown error code: E{}", code_num))?;

        if json_output {
            let output = serde_json::json!({
                "code": error_code.as_str(),
                "component": error_code.component(),
                "is_warning": error_code.is_warning(),
                "documentation_url": error_code.documentation_url(),
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("{}", "=".repeat(70).bright_cyan());
            println!(
                "{} {}",
                "Error Code:".bright_yellow(),
                error_code.as_str().bright_white().bold()
            );
            println!(
                "{} {}",
                "Component:".bright_yellow(),
                error_code.component().bright_white()
            );
            println!(
                "{} {}",
                "Severity:".bright_yellow(),
                if error_code.is_warning() {
                    "Warning".bright_yellow()
                } else {
                    "Error".bright_red()
                }
            );
            println!("{}", "=".repeat(70).bright_cyan());
            println!();

            // Print description based on the error code
            match error_code {
                apxm_core::error::ErrorCode::DeadNode => {
                    println!("{}", "Description:".bright_blue().bold());
                    println!("  A node produces output that is never used (no path to return).");
                    println!();
                    println!("{}", "How to fix:".bright_green().bold());
                    println!("  - Remove the node if it's not needed, OR");
                    println!("  - Connect it to the graph's output path");
                }
                apxm_core::error::ErrorCode::MissingReturnValue => {
                    println!("{}", "Description:".bright_blue().bold());
                    println!("  Graph has no exit node (all nodes have outgoing edges).");
                    println!();
                    println!("{}", "How to fix:".bright_green().bold());
                    println!(
                        "  Ensure at least one node has zero outgoing edges to serve as the return value."
                    );
                }
                apxm_core::error::ErrorCode::CommunicateBeforeSpawn => {
                    println!("{}", "Description:".bright_blue().bold());
                    println!("  COMMUNICATE node has no path FROM its SPAWN_AGENT.");
                    println!();
                    println!("{}", "Why this is a problem:".bright_blue().bold());
                    println!("  Without a dependency edge, COMMUNICATE may run in parallel with");
                    println!("  (or before) SPAWN_AGENT, causing a race condition.");
                    println!();
                    println!("{}", "How to fix:".bright_green().bold());
                    println!("  Add a Control or Data edge from SPAWN_AGENT to COMMUNICATE");
                    println!("  to ensure correct ordering.");
                }
                apxm_core::error::ErrorCode::EmptyTemplate => {
                    println!("{}", "Description:".bright_blue().bold());
                    println!(
                        "  ASK/THINK/REASON node has an empty or whitespace-only template_str."
                    );
                    println!();
                    println!("{}", "How to fix:".bright_green().bold());
                    println!("  Provide a non-empty template string.");
                }
                _ => {
                    println!("{}", "Description:".bright_blue().bold());
                    println!("  See documentation for details.");
                }
            }
            println!();
            println!(
                "{} {}",
                "Documentation:".bright_yellow(),
                error_code.documentation_url()
            );
        }

        return Ok(());
    }

    // Otherwise, treat as a graph file path
    let file = PathBuf::from(target);
    let content = std::fs::read_to_string(&file)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", file.display()))?;
    let graph: apxm_graph::ApxmGraph = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Invalid JSON in {}: {e}", file.display()))?;

    let ga = GraphAnalysis::from_graph(&graph);
    let critical_ms = ga.parallel_ms();
    let max_parallelism = ga.max_parallelism();
    let depth = ga.phases.len();
    let parallelizable = max_parallelism > 1;

    let notable_attrs = |id: u64| -> Vec<(String, String)> {
        let mut attrs = Vec::new();
        if let Some(node) = ga.node_by_id(id) {
            let interesting = [
                "template_str",
                "capability",
                "claim",
                "evidence",
                "token",
                "value",
                "label_true",
                "label_false",
                "key",
                "tokens",
                "flow_name",
                "budget",
                "store",
                "namespace",
                "target_task",
            ];
            for &key in &interesting {
                if let Some(val) = node.attributes.get(key) {
                    let display = match val {
                        apxm_core::types::Value::String(s) => {
                            if s.len() > 60 {
                                format!("{}...", &s[..57])
                            } else {
                                s.clone()
                            }
                        }
                        other => other.to_string(),
                    };
                    attrs.push((key.to_string(), display));
                }
            }
        }
        attrs
    };

    if json_output {
        let phase_json: Vec<serde_json::Value> = ga
            .phases
            .iter()
            .enumerate()
            .map(|(i, layer)| {
                let node_details: Vec<serde_json::Value> = layer
                    .iter()
                    .map(|&id| {
                        let op = ga.node_op(id);
                        let spec = find_op_spec(&op);
                        let required_attrs: Vec<String> = spec
                            .map(|s| {
                                s.fields
                                    .iter()
                                    .filter(|f| f.required)
                                    .map(|f| f.name.to_string())
                                    .collect()
                            })
                            .unwrap_or_default();
                        let feeds: Vec<u64> = ga.successors.get(&id).cloned().unwrap_or_default();
                        let depends_on: Vec<u64> =
                            ga.predecessors.get(&id).cloned().unwrap_or_default();

                        serde_json::json!({
                            "id": id,
                            "name": ga.node_name(id),
                            "op": op,
                            "category": spec.map(|s| category_str(s.category)).unwrap_or("unknown"),
                            "description": spec.map(|s| s.description).unwrap_or(""),
                            "latency": spec.map(|s| s.latency.as_str()).unwrap_or("unknown"),
                            "latency_ms": ga.node_latency_ms(id),
                            "produces_output": spec.map(|s| s.produces_output).unwrap_or(false),
                            "required_attributes": required_attrs,
                            "feeds": feeds,
                            "depends_on": depends_on,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "phase": i + 1,
                    "parallel": layer.len() > 1,
                    "nodes": node_details,
                })
            })
            .collect();

        let result = serde_json::json!({
            "file": file.display().to_string(),
            "graph_name": ga.graph.name,
            "node_count": ga.graph.nodes.len(),
            "edge_count": ga.edge_count,
            "depth": depth,
            "execution_flow": phase_json,
            "summary": {
                "max_parallelism": max_parallelism,
                "critical_path_steps": depth,
                "estimated_ms": critical_ms,
            },
        });
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        println!();
        println!("  {} {}", "Graph:".bold().cyan(), ga.graph.name.bold(),);
        println!(
            "  Nodes: {} | Edges: {} | Depth: {}",
            ga.graph.nodes.len(),
            ga.edge_count,
            depth,
        );
        println!();
        println!("  {}", "Execution Flow:".bold().cyan());
        println!(
            "  {}",
            apxm_core::constants::ui::icons::HRULE_DOUBLE
                .repeat(15)
                .dimmed()
        );

        for (i, layer) in ga.phases.iter().enumerate() {
            println!();
            if layer.len() > 1 {
                println!(
                    "  {} ({} parallel):",
                    format!("Phase {}", i + 1).bold(),
                    layer.len(),
                );
            } else {
                println!("  {}:", format!("Phase {}", i + 1).bold());
            }

            for &id in layer {
                let op = ga.node_op(id);
                let spec = find_op_spec(&op);
                let cat = spec.map(|s| category_str(s.category)).unwrap_or("unknown");
                let lat_val = ga.node_latency_ms(id);
                let desc = spec.map(|s| s.description).unwrap_or("");

                println!();
                println!(
                    "    {} \"{}\" {} {} ({}, ~{}ms)",
                    format!("[{}]", id).dimmed(),
                    ga.node_name(id).bold(),
                    apxm_core::constants::ui::icons::EM_DASH.dimmed(),
                    op.cyan().bold(),
                    cat,
                    lat_val,
                );
                println!("        {}", desc.dimmed());

                // Show notable attributes
                for (key, val) in notable_attrs(id) {
                    let label = key.chars().next().unwrap_or(' ').to_uppercase().to_string()
                        + &key[1..].replace('_', " ");
                    println!("        {}: \"{}\"", label.bold(), val);
                }

                // Dependency info
                let deps: Vec<u64> = ga.predecessors.get(&id).cloned().unwrap_or_default();
                let feeds: Vec<u64> = ga.successors.get(&id).cloned().unwrap_or_default();
                if !deps.is_empty() {
                    let dep_strs: Vec<String> = deps.iter().map(|d| d.to_string()).collect();
                    println!(
                        "        {} depends on: [{}]",
                        apxm_core::constants::ui::icons::ARROW_LEFT.dimmed(),
                        dep_strs.join(", "),
                    );
                }
                if !feeds.is_empty() {
                    let feed_strs: Vec<String> = feeds.iter().map(|f| f.to_string()).collect();
                    println!(
                        "        {} feeds: [{}]",
                        apxm_core::constants::ui::icons::ARROW_RIGHT.dimmed(),
                        feed_strs.join(", "),
                    );
                }
            }
        }

        println!();
        println!("  {}", "Summary:".bold().cyan());
        println!(
            "    Max parallelism: {}{}",
            max_parallelism,
            if parallelizable {
                format!(
                    " (phase {})",
                    ga.phases
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| p.len() > 1)
                        .map(|(i, _)| (i + 1).to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                String::new()
            },
        );
        println!("    Critical path: {} steps (~{}ms)", depth, critical_ms);
        println!(
            "    Parallelizable: {}",
            if parallelizable { "yes" } else { "no" }
        );
    }

    Ok(())
}

fn template_command(action: TemplateAction, json_output: bool) -> Result<()> {
    struct Template {
        name: &'static str,
        description: &'static str,
        graph_json: &'static str,
    }

    let templates = &[
        Template {
            name: "ask",
            description: "Single LLM call — the simplest possible graph",
            graph_json: r#"{
  "name": "simple-ask",
  "nodes": [
    {"id": 1, "name": "prompt", "op": "ASK", "attributes": {"template_str": "Explain quantum computing in one sentence"}}
  ],
  "edges": [],
  "parameters": [],
  "metadata": {}
}"#,
        },
        Template {
            name: "pipeline",
            description: "Sequential chain — each step feeds the next (draft → review → refine)",
            graph_json: r#"{
  "name": "pipeline",
  "nodes": [
    {"id": 1, "name": "draft", "op": "ASK", "attributes": {"template_str": "Write a short blog post about Rust"}},
    {"id": 2, "name": "review", "op": "THINK", "attributes": {"template_str": "Review this draft for clarity and accuracy: {{node_1}}"}},
    {"id": 3, "name": "refine", "op": "ASK", "attributes": {"template_str": "Improve the draft based on this review feedback: {{node_2}}"}}
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"},
    {"from": 2, "to": 3, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}"#,
        },
        Template {
            name: "fan-out",
            description: "Parallel execution — multiple independent tasks run concurrently then synchronize",
            graph_json: r#"{
  "name": "fan-out",
  "nodes": [
    {"id": 1, "name": "research-a", "op": "ASK", "attributes": {"template_str": "Research topic A"}},
    {"id": 2, "name": "research-b", "op": "ASK", "attributes": {"template_str": "Research topic B"}},
    {"id": 3, "name": "research-c", "op": "ASK", "attributes": {"template_str": "Research topic C"}},
    {"id": 4, "name": "merge", "op": "WAIT_ALL", "attributes": {"tokens": ["{{node_1}}", "{{node_2}}", "{{node_3}}"]}}
  ],
  "edges": [
    {"from": 1, "to": 4, "dependency": "Data"},
    {"from": 2, "to": 4, "dependency": "Data"},
    {"from": 3, "to": 4, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}"#,
        },
        Template {
            name: "map-reduce",
            description: "Fan-out then synthesize — parallel work followed by aggregation",
            graph_json: r#"{
  "name": "map-reduce",
  "nodes": [
    {"id": 1, "name": "analyze-1", "op": "ASK", "attributes": {"template_str": "Analyze aspect 1 of the problem"}},
    {"id": 2, "name": "analyze-2", "op": "ASK", "attributes": {"template_str": "Analyze aspect 2 of the problem"}},
    {"id": 3, "name": "analyze-3", "op": "ASK", "attributes": {"template_str": "Analyze aspect 3 of the problem"}},
    {"id": 4, "name": "sync", "op": "WAIT_ALL", "attributes": {"tokens": ["{{node_1}}", "{{node_2}}", "{{node_3}}"]}},
    {"id": 5, "name": "synthesize", "op": "ASK", "attributes": {"template_str": "Synthesize all analyses into a final report: {{node_4}}"}}
  ],
  "edges": [
    {"from": 1, "to": 4, "dependency": "Data"},
    {"from": 2, "to": 4, "dependency": "Data"},
    {"from": 3, "to": 4, "dependency": "Data"},
    {"from": 4, "to": 5, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}"#,
        },
        Template {
            name: "verify",
            description: "Claim + verification — generate then fact-check",
            graph_json: r#"{
  "name": "verify",
  "nodes": [
    {"id": 1, "name": "generate", "op": "ASK", "attributes": {"template_str": "State 3 facts about the solar system"}},
    {"id": 2, "name": "check", "op": "VERIFY", "attributes": {"claim": "{{node_1}}", "evidence": "Common astronomical knowledge"}}
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}"#,
        },
        Template {
            name: "conditional",
            description: "Branch on a condition — route to different paths based on comparison",
            graph_json: r#"{
  "name": "conditional",
  "nodes": [
    {"id": 1, "name": "classify", "op": "ASK", "attributes": {"template_str": "Is this a technical question? Answer only 'yes' or 'no'"}},
    {"id": 2, "name": "branch", "op": "BRANCH_ON_VALUE", "attributes": {"token": "{{node_1}}", "value": "yes", "label_true": "3", "label_false": "4"}},
    {"id": 3, "name": "technical-path", "op": "ASK", "attributes": {"template_str": "Give a detailed technical answer"}},
    {"id": 4, "name": "general-path", "op": "ASK", "attributes": {"template_str": "Give a friendly general answer"}}
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"},
    {"from": 2, "to": 3, "dependency": "Control"},
    {"from": 2, "to": 4, "dependency": "Control"}
  ],
  "parameters": [],
  "metadata": {}
}"#,
        },
    ];

    let user_templates = load_user_templates();

    match action {
        TemplateAction::List => {
            if json_output {
                let mut items: Vec<serde_json::Value> = templates
                    .iter()
                    .map(|t| serde_json::json!({"name": t.name, "description": t.description}))
                    .collect();
                for ut in &user_templates {
                    items.push(serde_json::json!({"name": ut.name, "description": ut.description, "source": "user"}));
                }
                println!("{}", serde_json::to_string_pretty(&items).unwrap());
            } else {
                print_section_header("Graph Templates");
                for t in templates {
                    println!("  {:<16} {}", t.name.bold(), t.description);
                }
                if !user_templates.is_empty() {
                    println!();
                    println!(
                        "  {} User templates (from ~/.apxm/templates.json):",
                        "~".dimmed()
                    );
                    for ut in &user_templates {
                        println!("  {:<16} {}", ut.name.bold(), ut.description);
                    }
                }
                println!();
                println!(
                    "  Use {} for the full graph JSON",
                    "apxm template show <name>".bold()
                );
            }
        }
        TemplateAction::Show { name } => {
            if let Some(tpl) = templates
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&name))
            {
                if json_output {
                    println!("{}", tpl.graph_json);
                } else {
                    print_section_header(&format!("Template: {}", tpl.name));
                    println!("  {}", tpl.description);
                    println!();
                    println!("{}", tpl.graph_json);
                    println!();
                    println!(
                        "  {} pipe to validate: {} | apxm validate /dev/stdin",
                        apxm_core::constants::ui::icons::INFO.cyan(),
                        format!("apxm template show {} --json", tpl.name).dimmed()
                    );
                }
            } else if let Some(ut) = user_templates
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&name))
            {
                if json_output {
                    println!("{}", ut.graph_json);
                } else {
                    print_section_header(&format!("Template: {} (user)", ut.name));
                    println!("  {}", ut.description);
                    println!();
                    println!("{}", ut.graph_json);
                }
            } else {
                return Err(anyhow::anyhow!(
                    "Unknown template '{}'. Run 'apxm template list' to see available templates.",
                    name
                ));
            }
        }
    }

    Ok(())
}

#[derive(serde::Deserialize)]
struct UserTemplateEntry {
    name: String,
    description: String,
    graph_json: String,
}

fn load_user_templates() -> Vec<UserTemplateEntry> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let templates_path = home.join(".apxm").join("templates.json");
    if !templates_path.exists() {
        return Vec::new();
    }
    let Ok(content) = std::fs::read_to_string(&templates_path) else {
        return Vec::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn ops_command(action: OpsAction, json_output: bool) -> Result<()> {
    use apxm_core::types::{AIS_OPERATIONS, OperationCategory};

    fn parse_category(s: &str) -> Option<OperationCategory> {
        match s.to_lowercase().as_str() {
            "metadata" => Some(OperationCategory::Metadata),
            "memory" => Some(OperationCategory::Memory),
            "reasoning" | "llm" => Some(OperationCategory::Reasoning),
            "tools" | "tool" => Some(OperationCategory::Tools),
            "control_flow" | "controlflow" | "control" => Some(OperationCategory::ControlFlow),
            "synchronization" | "sync" => Some(OperationCategory::Synchronization),
            "error_handling" | "error" | "errorhandling" => Some(OperationCategory::ErrorHandling),
            "communication" | "comm" => Some(OperationCategory::Communication),
            "internal" => Some(OperationCategory::Internal),
            _ => None,
        }
    }

    match action {
        OpsAction::List { category } => {
            let cat_filter = match &category {
                Some(c) => Some(parse_category(c).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Unknown category '{}'. Valid: metadata, memory, reasoning, tools, \
                         control_flow, synchronization, error_handling, communication, internal",
                        c
                    )
                })?),
                None => None,
            };

            let ops: Vec<_> = AIS_OPERATIONS
                .iter()
                .filter(|s| cat_filter.is_none_or(|c| s.category == c))
                .collect();

            if json_output {
                let json_ops: Vec<serde_json::Value> = ops
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "op": s.op_type.to_string(),
                            "name": s.name,
                            "category": category_str(s.category),
                            "description": s.description,
                            "latency": s.latency.as_str(),
                            "produces_output": s.produces_output,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&json_ops).unwrap());
            } else {
                let mut current_cat = None;
                for spec in &ops {
                    if current_cat != Some(spec.category) {
                        current_cat = Some(spec.category);
                        println!();
                        println!(
                            "  {}",
                            category_str(spec.category).to_uppercase().bold().cyan()
                        );
                        println!(
                            "  {}",
                            apxm_core::constants::ui::icons::HRULE.repeat(40).dimmed()
                        );
                    }
                    println!(
                        "  {:<18} {}  {}",
                        spec.op_type.to_string().bold(),
                        format!("[{}]", spec.latency.as_str()).dimmed(),
                        spec.description
                    );
                }
                println!();
                println!("  {} operations total", ops.len());
                println!("  Use {} for details", "apxm ops show <OP>".bold());
            }
        }
        OpsAction::Show { name } => {
            let name_upper = name.to_uppercase();
            let spec = AIS_OPERATIONS
                .iter()
                .find(|s| s.op_type.to_string() == name_upper || s.name.eq_ignore_ascii_case(&name))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Unknown operation '{}'. Run 'apxm ops list' to see all operations.",
                        name
                    )
                })?;

            if json_output {
                let required: Vec<serde_json::Value> = spec
                    .fields
                    .iter()
                    .filter(|f| f.required)
                    .map(|f| serde_json::json!({"name": f.name, "description": f.description}))
                    .collect();
                let optional: Vec<serde_json::Value> = spec
                    .fields
                    .iter()
                    .filter(|f| !f.required)
                    .map(|f| serde_json::json!({"name": f.name, "description": f.description}))
                    .collect();

                let mut result = serde_json::json!({
                    "op": spec.op_type.to_string(),
                    "name": spec.name,
                    "category": category_str(spec.category),
                    "description": spec.description,
                    "long_description": spec.long_description,
                    "latency": spec.latency.as_str(),
                    "required_fields": required,
                    "optional_fields": optional,
                    "produces_output": spec.produces_output,
                    "needs_submission": spec.needs_submission,
                    "min_inputs": spec.min_inputs,
                });

                if let Some(example) = spec.example_json {
                    result["example"] = serde_json::Value::String(example.to_string());
                }

                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else {
                println!();
                println!(
                    "  {} {}",
                    spec.op_type.to_string().bold().cyan(),
                    spec.name.dimmed()
                );
                println!(
                    "  {}",
                    apxm_core::constants::ui::icons::HRULE.repeat(50).dimmed()
                );
                println!("  {}", spec.description);
                println!();
                println!("  {}", spec.long_description);
                println!();
                println!(
                    "  {} {}  {} {}  {} {}",
                    "Category:".bold(),
                    category_str(spec.category),
                    "Latency:".bold(),
                    spec.latency.as_str(),
                    "Output:".bold(),
                    if spec.produces_output { "yes" } else { "no" }
                );

                let required_fields: Vec<_> = spec.fields.iter().filter(|f| f.required).collect();
                let optional_fields: Vec<_> = spec.fields.iter().filter(|f| !f.required).collect();

                if !required_fields.is_empty() {
                    println!();
                    println!("  {}", "Required Fields:".bold());
                    for f in &required_fields {
                        println!("    {} {}", f.name.green().bold(), f.description.dimmed());
                    }
                }

                if !optional_fields.is_empty() {
                    println!();
                    println!("  {}", "Optional Fields:".bold());
                    for f in &optional_fields {
                        println!("    {} {}", f.name.yellow(), f.description.dimmed());
                    }
                }

                if let Some(example) = spec.example_json {
                    println!();
                    println!("  {}", "Example Node JSON:".bold());
                    println!("  {}", example);
                }
                println!();
            }
        }
    }
    Ok(())
}

fn doctor_command(config: Option<PathBuf>, json_output: bool) -> Result<()> {
    let report = MlirEnvReport::detect();
    let mlir_available = report.is_ready();
    let mlir_prefix = report
        .resolved_prefix
        .as_ref()
        .map(|p| p.display().to_string());
    let mlir_version = report.llvm_version.clone();

    // --- Backends ---
    let (backend_count, backend_names): (usize, Vec<String>) =
        match apxm_credentials::BackendStore::open() {
            Ok(store) => match store.list() {
                Ok(backends) => {
                    let names: Vec<String> = backends.iter().map(|b| b.name.clone()).collect();
                    (names.len(), names)
                }
                Err(_) => (0, vec![]),
            },
            Err(_) => (0, vec![]),
        };

    // --- Environment Variables ---
    let env_apxm_backend = env::var("APXM_BACKEND").ok();
    let env_mlir_dir = env::var("MLIR_DIR").ok();
    let env_llvm_dir = env::var("LLVM_DIR").ok();

    // --- Config (driver feature only) ---
    #[cfg(feature = "driver")]
    let (config_found, config_path, config_backends) = {
        match load_config(config) {
            Ok(cfg) => {
                let path = ApXmConfig::default_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "~/.apxm/config.toml".to_string());
                let backends = cfg.backends.len();
                (true, Some(path), backends)
            }
            Err(_) => {
                let path = ApXmConfig::default_path()
                    .map(|p| p.display().to_string())
                    .ok();
                (false, path, 0usize)
            }
        }
    };
    #[cfg(not(feature = "driver"))]
    let (_config_found, _config_path, _config_backends) = {
        let _ = config;
        (false, None::<String>, 0usize)
    };

    // --- JSON output mode ---
    if json_output {
        #[allow(unused_mut)]
        let mut report_json = serde_json::json!({
            "mlir": {
                "available": mlir_available,
                "prefix": mlir_prefix,
                "version": mlir_version,
            },
            "backends": {
                "count": backend_count,
                "names": backend_names,
            },
            "environment": {
                "APXM_BACKEND": env_apxm_backend,
                "MLIR_DIR": env_mlir_dir,
                "LLVM_DIR": env_llvm_dir,
            },
        });
        // Only include config section when driver feature is available
        #[cfg(feature = "driver")]
        {
            report_json["config"] = serde_json::json!({
                "found": config_found,
                "path": config_path,
                "backends": config_backends,
            });
        }
        println!("{}", serde_json::to_string_pretty(&report_json)?);
        return Ok(());
    }

    // --- Human-readable output ---

    // 1. MLIR toolchain (existing checks)
    print_section_header("MLIR Toolchain");
    print_minimal_mlir_status();

    if mlir_available {
        let detail = match &mlir_version {
            Some(v) => format!("ready (LLVM {})", v),
            None => "ready".to_string(),
        };
        print_status_line("MLIR toolchain", Status::Ok, &detail);
    } else {
        print_status_line("MLIR toolchain", Status::Error, "missing");
    }

    // 2. Backends
    print_section_header("Backends");
    if backend_count > 0 {
        print_status_line(
            "Backends",
            Status::Ok,
            &format!(
                "{} backend{} registered",
                backend_count,
                if backend_count == 1 { "" } else { "s" }
            ),
        );
    } else {
        print_status_line("Backends", Status::Warning, "none registered");
        print_hint("Run `apxm backend add` to configure a backend.");
    }

    // 3. Environment Variables
    print_section_header("Environment");
    for (name, value) in [
        ("APXM_BACKEND", &env_apxm_backend),
        ("MLIR_DIR", &env_mlir_dir),
        ("LLVM_DIR", &env_llvm_dir),
    ] {
        match value {
            Some(v) => print_status_line(name, Status::Ok, v),
            None => print_status_line(name, Status::Warning, "not set"),
        }
    }
    if env_mlir_dir.is_none() || env_llvm_dir.is_none() {
        print_hint("Run `eval $(apxm activate)` to set MLIR/LLVM environment variables.");
    }

    // 4. Config (driver feature only)
    #[cfg(feature = "driver")]
    {
        print_section_header("Configuration");
        if config_found {
            let path_display = config_path.as_deref().unwrap_or("~/.apxm/config.toml");
            print_status_line(
                "Config file",
                Status::Ok,
                &format!("found at {}", path_display),
            );
            print_status_line(
                "LLM backends",
                if config_backends > 0 {
                    Status::Ok
                } else {
                    Status::Warning
                },
                &format!("{} configured", config_backends),
            );
        } else {
            let path_display = config_path.as_deref().unwrap_or("~/.apxm/config.toml");
            print_status_line(
                "Config file",
                Status::Warning,
                &format!("not found ({})", path_display),
            );
        }
    }

    // Return error if MLIR is missing (critical dependency)
    if !mlir_available {
        return Err(anyhow::anyhow!("MLIR toolchain not detected"));
    }

    Ok(())
}

/// Auto-detect the conda prefix for the `apxm` environment.
///
/// Resolution order:
/// 1. `CONDA_PREFIX` env var
/// 2. `conda info --envs --json` output (looks for an env named "apxm")
/// 3. Common paths: ~/miniforge3/envs/apxm, ~/mambaforge/envs/apxm, ~/miniconda3/envs/apxm
fn detect_conda_prefix() -> Option<PathBuf> {
    // 1. Check CONDA_PREFIX env var
    if let Ok(prefix) = env::var("CONDA_PREFIX") {
        let p = PathBuf::from(&prefix);
        if p.is_dir() {
            return Some(p);
        }
    }

    // 2. Try `conda info --envs --json`
    if let Ok(output) = std::process::Command::new("conda")
        .args(["info", "--envs", "--json"])
        .output()
        && output.status.success()
        && let Ok(text) = String::from_utf8(output.stdout)
    {
        // Minimal JSON parsing: look for paths ending in /apxm
        for line in text.lines() {
            let trimmed = line.trim().trim_matches('"').trim_end_matches(',');
            let candidate = PathBuf::from(trimmed);
            if candidate.file_name().map(|n| n == "apxm").unwrap_or(false) && candidate.is_dir() {
                return Some(candidate);
            }
        }
    }

    // 3. Check common paths
    if let Some(home) = dirs::home_dir() {
        let candidates = [
            home.join("miniforge3/envs/apxm"),
            home.join("mambaforge/envs/apxm"),
            home.join("miniconda3/envs/apxm"),
        ];
        for candidate in &candidates {
            if candidate.is_dir() {
                return Some(candidate.clone());
            }
        }
    }

    None
}

fn print_minimal_mlir_status() {
    let conda_prefix = detect_conda_prefix();
    let conda_bin = conda_prefix.as_ref().map(|p| p.join("bin"));
    let mlir_tblgen = conda_bin.as_ref().map(|p| p.join("mlir-tblgen"));
    let mlir_cmake = conda_prefix.as_ref().map(|p| p.join("lib/cmake/mlir"));
    let llvm_cmake = conda_prefix.as_ref().map(|p| p.join("lib/cmake/llvm"));

    match conda_prefix.as_ref() {
        Some(prefix) => {
            print_status_line("Conda prefix", Status::Ok, &prefix.display().to_string())
        }
        None => {
            print_status_line("Conda prefix", Status::Error, "<not set>");
            print_hint("Run `cargo run -p apxm-cli -- install`, then `conda activate apxm`.");
            return;
        }
    }

    let checks: &[(&str, bool)] = &[
        (
            "mlir-tblgen",
            mlir_tblgen.as_ref().map(|p| p.is_file()).unwrap_or(false),
        ),
        (
            "cmake/mlir",
            mlir_cmake.as_ref().map(|p| p.is_dir()).unwrap_or(false),
        ),
        (
            "cmake/llvm",
            llvm_cmake.as_ref().map(|p| p.is_dir()).unwrap_or(false),
        ),
    ];

    for &(label, found) in checks {
        let status = if found { Status::Ok } else { Status::Error };
        print_status_line(label, status, if found { "found" } else { "missing" });
    }

    if checks.iter().any(|(_, found)| !found) {
        print_subsection_header("Suggested Fix");
        println!("cargo run -p apxm-cli -- install");
        println!("conda activate apxm");
        println!("eval \"$(cargo run -p apxm-cli -- activate)\"");
        if let Some(ref prefix) = conda_prefix {
            println!("# Or export directly from: {}", prefix.display());
        }
    }
}

fn task_command(action: TaskAction, json_output: bool) -> Result<()> {
    use apxm_graph::ApxmGraph;

    match action {
        TaskAction::Merge {
            graphs,
            name,
            output,
        } => {
            let mut parsed_graphs: Vec<ApxmGraph> = Vec::with_capacity(graphs.len());
            for path in &graphs {
                let content = std::fs::read_to_string(path)
                    .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", path.display()))?;
                let graph = ApxmGraph::from_json(&content)
                    .map_err(|e| anyhow::anyhow!("Invalid graph {}: {e}", path.display()))?;
                parsed_graphs.push(graph);
            }

            let input_count = parsed_graphs.len();
            let merged = ApxmGraph::merge(&name, &parsed_graphs);

            let total_nodes = merged.nodes.len();
            let total_edges = merged.edges.len();
            let sync_node_id = merged.nodes.last().map(|n| n.id);

            if json_output {
                let merged_json = serde_json::to_value(&merged)
                    .map_err(|e| anyhow::anyhow!("Failed to serialize merged graph: {e}"))?;
                let result = serde_json::json!({
                    "merged_graph": merged_json,
                    "stats": {
                        "input_graphs": input_count,
                        "total_nodes": total_nodes,
                        "total_edges": total_edges,
                        "sync_node_id": sync_node_id,
                    }
                });
                let json_str = serde_json::to_string_pretty(&result).unwrap();
                if let Some(out_path) = output {
                    std::fs::write(&out_path, &json_str).map_err(|e| {
                        anyhow::anyhow!("Failed to write {}: {e}", out_path.display())
                    })?;
                    eprintln!("Wrote merged graph to {}", out_path.display());
                } else {
                    println!("{json_str}");
                }
            } else {
                let graph_json = merged
                    .to_json()
                    .map_err(|e| anyhow::anyhow!("Failed to serialize merged graph: {e}"))?;
                let written_path = if let Some(ref out_path) = output {
                    std::fs::write(out_path, &graph_json).map_err(|e| {
                        anyhow::anyhow!("Failed to write {}: {e}", out_path.display())
                    })?;
                    out_path.clone()
                } else {
                    let default_path = PathBuf::from(format!("{name}.json"));
                    std::fs::write(&default_path, &graph_json).map_err(|e| {
                        anyhow::anyhow!("Failed to write {}: {e}", default_path.display())
                    })?;
                    default_path
                };

                print_section_header("Task Merge");
                print_status_line("name", Status::Ok, &name);
                print_status_line("inputs", Status::Ok, &format!("{input_count} graph(s)"));
                print_status_line("nodes", Status::Ok, &format!("{total_nodes}"));
                print_status_line("edges", Status::Ok, &format!("{total_edges}"));
                if let Some(sid) = sync_node_id {
                    print_status_line("sync node", Status::Ok, &format!("id={sid}"));
                }
                println!();
                println!(
                    "  Wrote merged graph to {}",
                    written_path.display().to_string().bold()
                );
            }

            Ok(())
        }
    }
}

fn codegen_command(action: CodegenAction, json_output: bool) -> Result<()> {
    match action {
        CodegenAction::Frontend { output_dir } => {
            let output_dir = output_dir.unwrap_or_else(default_frontend_codegen_dir);
            apxm_frontend::write_generated_python(&output_dir)?;

            let mut files: Vec<String> = apxm_frontend::render_generated_python()
                .into_iter()
                .map(|(name, _)| name.to_string())
                .collect();
            files.sort();

            if json_output {
                let output = serde_json::json!({
                    "target": "frontend",
                    "output_dir": output_dir.display().to_string(),
                    "files": files,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Generated frontend bindings:");
                println!("  target: frontend");
                println!("  output: {}", output_dir.display());
                for file in files {
                    println!("  - {file}");
                }
            }

            Ok(())
        }
    }
}

fn default_frontend_codegen_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/apxm-frontend/python/apxm/_generated")
}

fn print_section_header(title: &str) {
    use apxm_core::constants::ui;
    println!();
    println!("  {}", title.bold().cyan());
    println!("  {}", ui::icons::HRULE.repeat(title.len()).dimmed());
}

fn print_subsection_header(title: &str) {
    println!();
    println!("  {}", title.bold());
}

fn print_hint(message: &str) {
    use apxm_core::constants::ui;
    println!("  {} {}", ui::icons::INFO.cyan(), message);
}

enum Status {
    Ok,
    Warning,
    Error,
}

fn print_status_line(label: &str, status: Status, value: &str) {
    use apxm_core::constants::ui;
    let (icon, status_str) = match status {
        Status::Ok => (ui::icons::SUCCESS.green(), ui::labels::OK.green().bold()),
        Status::Warning => (
            ui::icons::WARNING.yellow(),
            ui::labels::WARN.yellow().bold(),
        ),
        Status::Error => (ui::icons::FAILED.red(), ui::labels::MISSING.red().bold()),
    };
    println!("  {} {:<14} [{}] {}", icon, label.bold(), status_str, value);
}

fn replay_command(session: PathBuf) -> Result<()> {
    use apxm_core::constants;
    use apxm_core::types::SessionManifest;

    // Read manifest
    let manifest_path = session.join(constants::session::files::MANIFEST);
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let manifest: SessionManifest =
        serde_json::from_str(&manifest_text).context("Failed to parse manifest")?;

    let duration_secs = manifest.duration_ms as f64 / 1000.0;
    let status_str = if manifest.success {
        "success"
    } else {
        "failed"
    };

    println!(
        "Session: {} ({} nodes, {:.1}s, {})",
        manifest.execution_id, manifest.node_count, duration_secs, status_str
    );
    println!();

    // Read trace (open directly, handle missing file)
    let trace_path = session.join(constants::session::files::TRACE);
    let trace_file = match std::fs::File::open(&trace_path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("  (no trace file found)");
            return Ok(());
        }
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to open {}: {e}",
                trace_path.display()
            ));
        }
    };

    // Read node names from the input graph (best-effort)
    let input_path = session.join(constants::session::files::INPUT_GRAPH);
    let node_names: HashMap<u64, String> = std::fs::read_to_string(&input_path)
        .ok()
        .and_then(|text| apxm_graph::ApxmGraph::from_json(&text).ok())
        .map(|graph| graph.nodes.iter().map(|n| (n.id, n.name.clone())).collect())
        .unwrap_or_default();

    // Parse trace events and build timeline
    use apxm_events::ApxmEvent;
    use apxm_events::payload::EventPayload;

    enum EventKind {
        Start,
        End { duration_ms: u64, success: bool },
    }

    struct TimelineEntry {
        timestamp_ms: f64,
        node_name: String,
        op_type: String,
        kind: EventKind,
    }

    let mut entries: Vec<TimelineEntry> = Vec::new();
    let mut first_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;

    let reader = std::io::BufReader::new(trace_file);

    for line in std::io::BufRead::lines(reader) {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let event: ApxmEvent = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let ts = event.meta.timestamp;
        if first_timestamp.is_none() {
            first_timestamp = Some(ts);
        }
        let elapsed_ms = (ts - first_timestamp.unwrap()).num_milliseconds().max(0) as f64;

        let resolve_name = |node_id: u64| {
            node_names
                .get(&node_id)
                .cloned()
                .unwrap_or_else(|| format!("node_{}", node_id))
        };

        match event.payload {
            EventPayload::OperationStart(ref p) => {
                entries.push(TimelineEntry {
                    timestamp_ms: elapsed_ms,
                    node_name: resolve_name(p.node_id),
                    op_type: p.op_type.clone(),
                    kind: EventKind::Start,
                });
            }
            EventPayload::OperationEnd(ref p) => {
                entries.push(TimelineEntry {
                    timestamp_ms: elapsed_ms,
                    node_name: resolve_name(p.node_id),
                    op_type: p.op_type.clone(),
                    kind: EventKind::End {
                        duration_ms: p.duration_ms,
                        success: p.success,
                    },
                });
            }
            _ => {}
        }
    }

    if entries.is_empty() {
        println!("  (no operation events in trace)");
        return Ok(());
    }

    // Sort by timestamp
    entries.sort_by(|a, b| a.timestamp_ms.partial_cmp(&b.timestamp_ms).unwrap());

    // Print timeline
    for entry in &entries {
        let t = entry.timestamp_ms / 1000.0;
        let (icon, detail) = match &entry.kind {
            EventKind::Start => (
                constants::ui::icons::STARTED,
                constants::ui::labels::STARTED.to_string(),
            ),
            EventKind::End {
                duration_ms,
                success,
            } => {
                let i = if *success {
                    constants::ui::icons::SUCCESS
                } else {
                    constants::ui::icons::FAILED
                };
                (i, format!("{:.1}s", *duration_ms as f64 / 1000.0))
            }
        };
        println!(
            "  {:>5.1}s  {:<20} {:<15} {} {}",
            t, entry.node_name, entry.op_type, icon, detail
        );
    }

    Ok(())
}

fn session_command(action: SessionAction, json: bool) -> Result<()> {
    match action {
        SessionAction::List { status, limit } => session_list_command(status, limit, json),
        SessionAction::Inspect { session } => session_inspect_command(session, json),
        SessionAction::Diff { session1, session2 } => {
            session_diff_command(session1, session2, json)
        }
        SessionAction::Clean {
            older_than,
            all,
            dry_run,
        } => session_clean_command(older_than, all, dry_run),
    }
}

fn get_sessions_dir() -> Result<PathBuf> {
    let home = env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".apxm/sessions"))
}

fn session_list_command(status_filter: Option<String>, limit: usize, json: bool) -> Result<()> {
    use apxm_core::constants;
    use apxm_core::types::SessionManifest;

    let sessions_dir = get_sessions_dir()?;
    if !sessions_dir.exists() {
        if json {
            println!("{{\"sessions\":[]}}");
        } else {
            println!("No sessions found");
        }
        return Ok(());
    }

    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(&sessions_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join(constants::session::files::MANIFEST);
        if let Ok(text) = std::fs::read_to_string(&manifest_path) {
            if let Ok(manifest) = serde_json::from_str::<SessionManifest>(&text) {
                if let Some(ref filter) = status_filter {
                    if manifest.status != *filter {
                        continue;
                    }
                }
                let size = dir_size(&path)?;
                sessions.push((manifest, path, size));
            }
        }
    }

    sessions.sort_by(|a, b| b.0.timestamp.cmp(&a.0.timestamp));
    let sessions: Vec<_> = sessions.into_iter().take(limit).collect();

    if json {
        let output: Vec<_> = sessions
            .iter()
            .map(|(m, p, s)| {
                serde_json::json!({
                    "execution_id": m.execution_id,
                    "graph_name": m.graph_name,
                    "timestamp": m.timestamp,
                    "status": m.status,
                    "duration_ms": m.duration_ms,
                    "node_count": m.node_count,
                    "success": m.success,
                    "path": p.display().to_string(),
                    "size_bytes": s,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if sessions.is_empty() {
            println!("No sessions found");
            return Ok(());
        }
        println!("Sessions (most recent first):");
        println!();
        for (manifest, path, size) in &sessions {
            let duration_secs = manifest.duration_ms as f64 / 1000.0;
            let size_mb = *size as f64 / 1_000_000.0;
            let status_icon = match manifest.status.as_str() {
                "completed" if manifest.success => constants::ui::icons::SUCCESS,
                "failed" => constants::ui::icons::FAILED,
                _ => constants::ui::icons::INFO,
            };
            println!(
                "{} {} | {} | {:.1}s | {} nodes | {:.1} MB",
                status_icon,
                manifest.execution_id,
                manifest.timestamp,
                duration_secs,
                manifest.node_count,
                size_mb
            );
            if let Some(ref name) = manifest.graph_name {
                println!("   Graph: {}", name);
            }
            println!("   Path: {}", path.display());
            println!();
        }
    }
    Ok(())
}

fn dir_size(path: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in walkdir::WalkDir::new(path) {
        let entry = entry?;
        if entry.file_type().is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

fn session_inspect_command(session_id: String, json: bool) -> Result<()> {
    use apxm_core::constants;
    use apxm_core::types::SessionManifest;

    let session_path = resolve_session_path(&session_id)?;
    let manifest_path = session_path.join(constants::session::files::MANIFEST);
    let manifest_text = std::fs::read_to_string(&manifest_path)?;
    let manifest: SessionManifest = serde_json::from_str(&manifest_text)?;

    let nodes_dir = session_path.join(constants::session::files::NODES_DIR);
    let mut node_info = Vec::new();

    if nodes_dir.exists() {
        for entry in std::fs::read_dir(&nodes_dir)? {
            let entry = entry?;
            let path = entry.path();
            let node_json_path = path.join(constants::session::node::NODE_JSON);
            if let Ok(text) = std::fs::read_to_string(&node_json_path) {
                if let Ok(info) = serde_json::from_str::<serde_json::Value>(&text) {
                    let status_path = path.join(constants::session::node::STATUS_JSON);
                    let status = if let Ok(s) = std::fs::read_to_string(&status_path) {
                        serde_json::from_str::<serde_json::Value>(&s).ok()
                    } else {
                        None
                    };
                    node_info.push((info, status));
                }
            }
        }
    }

    if json {
        let output = serde_json::json!({
            "manifest": manifest,
            "nodes": node_info,
            "path": session_path.display().to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Session: {}", manifest.execution_id);
        if let Some(ref name) = manifest.graph_name {
            println!("Graph: {}", name);
        }
        println!("Status: {}", manifest.status);
        println!("Duration: {:.2}s", manifest.duration_ms as f64 / 1000.0);
        println!("Nodes: {}", manifest.node_count);
        println!("Path: {}", session_path.display());
        println!();

        if !node_info.is_empty() {
            println!("Node details:");
            for (info, status) in &node_info {
                let id = info.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                let name = info
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let op = info.get("op").and_then(|v| v.as_str()).unwrap_or("unknown");
                println!("  {} | {} | {}", id, name, op);
                if let Some(s) = &status {
                    if let Some(duration) = s.get("duration_ms").and_then(|v| v.as_u64()) {
                        println!("    Duration: {:.2}s", duration as f64 / 1000.0);
                    }
                    if let Some(status_str) = s.get("status").and_then(|v| v.as_str()) {
                        println!("    Status: {}", status_str);
                    }
                }
            }
        }
    }
    Ok(())
}

fn resolve_session_path(session_id: &str) -> Result<PathBuf> {
    let path = PathBuf::from(session_id);
    if path.exists() && path.is_dir() {
        return Ok(path);
    }

    let sessions_dir = get_sessions_dir()?;
    let session_path = sessions_dir.join(session_id);
    if session_path.exists() && session_path.is_dir() {
        return Ok(session_path);
    }

    Err(anyhow::anyhow!("Session not found: {}", session_id))
}

fn session_diff_command(session1_id: String, session2_id: String, json: bool) -> Result<()> {
    use apxm_core::constants;
    use apxm_core::types::SessionManifest;

    let path1 = resolve_session_path(&session1_id)?;
    let path2 = resolve_session_path(&session2_id)?;

    let manifest1: SessionManifest = {
        let text = std::fs::read_to_string(path1.join(constants::session::files::MANIFEST))?;
        serde_json::from_str(&text)?
    };
    let manifest2: SessionManifest = {
        let text = std::fs::read_to_string(path2.join(constants::session::files::MANIFEST))?;
        serde_json::from_str(&text)?
    };

    let results1 = load_session_results(&path1)?;
    let results2 = load_session_results(&path2)?;

    let mut changed_nodes = Vec::new();
    let mut timing_diffs = Vec::new();

    for (node_id, output1) in &results1 {
        if let Some(output2) = results2.get(node_id) {
            if output1 != output2 {
                changed_nodes.push(*node_id);
            }
        }
    }

    let nodes1 = load_node_timings(&path1)?;
    let nodes2 = load_node_timings(&path2)?;
    for (node_id, duration1) in &nodes1 {
        if let Some(duration2) = nodes2.get(node_id) {
            let diff = (*duration2 as i64) - (*duration1 as i64);
            if diff.abs() > 100 {
                timing_diffs.push((*node_id, *duration1, *duration2, diff));
            }
        }
    }

    if json {
        let output = serde_json::json!({
            "session1": {
                "id": manifest1.execution_id,
                "duration_ms": manifest1.duration_ms,
                "status": manifest1.status,
            },
            "session2": {
                "id": manifest2.execution_id,
                "duration_ms": manifest2.duration_ms,
                "status": manifest2.status,
            },
            "changed_nodes": changed_nodes,
            "timing_diffs": timing_diffs.iter().map(|(id, d1, d2, diff)| {
                serde_json::json!({
                    "node_id": id,
                    "duration1_ms": d1,
                    "duration2_ms": d2,
                    "diff_ms": diff,
                })
            }).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Comparing sessions:");
        println!(
            "  Session 1: {} ({:.2}s, {})",
            manifest1.execution_id,
            manifest1.duration_ms as f64 / 1000.0,
            manifest1.status
        );
        println!(
            "  Session 2: {} ({:.2}s, {})",
            manifest2.execution_id,
            manifest2.duration_ms as f64 / 1000.0,
            manifest2.status
        );
        println!();

        let duration_diff = (manifest2.duration_ms as i64) - (manifest1.duration_ms as i64);
        println!(
            "Total duration diff: {:+.2}s ({:+}ms)",
            duration_diff as f64 / 1000.0,
            duration_diff
        );
        println!();

        if !changed_nodes.is_empty() {
            println!("Nodes with changed output ({}):", changed_nodes.len());
            for node_id in &changed_nodes {
                println!("  - Node {}", node_id);
            }
            println!();
        }

        if !timing_diffs.is_empty() {
            println!("Nodes with significant timing changes:");
            for (node_id, d1, d2, diff) in &timing_diffs {
                println!(
                    "  Node {}: {:.2}s → {:.2}s ({:+.2}s)",
                    node_id,
                    *d1 as f64 / 1000.0,
                    *d2 as f64 / 1000.0,
                    *diff as f64 / 1000.0
                );
            }
        }
    }
    Ok(())
}

fn load_session_results(session_path: &Path) -> Result<HashMap<u64, serde_json::Value>> {
    use apxm_core::constants;
    let results_path = session_path.join(constants::session::files::RESULTS);
    if !results_path.exists() {
        return Ok(HashMap::new());
    }
    let text = std::fs::read_to_string(&results_path)?;
    let all: serde_json::Value = serde_json::from_str(&text)?;
    let mut results = HashMap::new();
    if let Some(token_values) = all.get("token_values").and_then(|v| v.as_object()) {
        for (k, v) in token_values {
            if let Ok(id) = k.parse::<u64>() {
                results.insert(id, v.clone());
            }
        }
    }
    Ok(results)
}

fn load_node_timings(session_path: &Path) -> Result<HashMap<u64, u64>> {
    use apxm_core::constants;
    let nodes_dir = session_path.join(constants::session::files::NODES_DIR);
    let mut timings = HashMap::new();
    if !nodes_dir.exists() {
        return Ok(timings);
    }
    for entry in std::fs::read_dir(&nodes_dir)? {
        let entry = entry?;
        let path = entry.path();
        let status_path = path.join(constants::session::node::STATUS_JSON);
        if let Ok(text) = std::fs::read_to_string(&status_path) {
            if let Ok(status) = serde_json::from_str::<serde_json::Value>(&text) {
                if let (Some(node_json), Some(duration)) = (
                    std::fs::read_to_string(path.join(constants::session::node::NODE_JSON)).ok(),
                    status.get("duration_ms").and_then(|v| v.as_u64()),
                ) {
                    if let Ok(node_info) = serde_json::from_str::<serde_json::Value>(&node_json) {
                        if let Some(id) = node_info.get("id").and_then(|v| v.as_u64()) {
                            timings.insert(id, duration);
                        }
                    }
                }
            }
        }
    }
    Ok(timings)
}

fn session_clean_command(older_than: Option<String>, all: bool, dry_run: bool) -> Result<()> {
    let sessions_dir = get_sessions_dir()?;
    if !sessions_dir.exists() {
        println!("No sessions directory found");
        return Ok(());
    }

    let cutoff = if all {
        None
    } else if let Some(ref duration_str) = older_than {
        Some(parse_duration(duration_str)?)
    } else {
        return Err(anyhow::anyhow!(
            "Must specify --older-than <duration> or --all"
        ));
    };

    let mut to_delete = Vec::new();
    let now = chrono::Utc::now();

    for entry in std::fs::read_dir(&sessions_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        use apxm_core::constants;
        use apxm_core::types::SessionManifest;
        let manifest_path = path.join(constants::session::files::MANIFEST);
        if let Ok(text) = std::fs::read_to_string(&manifest_path) {
            if let Ok(manifest) = serde_json::from_str::<SessionManifest>(&text) {
                if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(&manifest.timestamp) {
                    let age = now.signed_duration_since(timestamp.with_timezone(&chrono::Utc));
                    if all || (cutoff.is_some() && age > cutoff.unwrap()) {
                        to_delete.push((path, manifest.execution_id.clone(), age));
                    }
                }
            }
        }
    }

    if to_delete.is_empty() {
        println!("No sessions to delete");
        return Ok(());
    }

    to_delete.sort_by_key(|(_, _, age)| *age);

    if dry_run {
        println!("Would delete {} sessions:", to_delete.len());
        for (_path, id, age) in &to_delete {
            println!("  {} (age: {}d)", id, age.num_days());
        }
    } else {
        println!("Deleting {} sessions...", to_delete.len());
        for (path, id, age) in &to_delete {
            println!("  {} (age: {}d)", id, age.num_days());
            std::fs::remove_dir_all(path)?;
        }
        println!("Done");
    }
    Ok(())
}

fn parse_duration(s: &str) -> Result<chrono::Duration> {
    let s = s.trim();
    if let Some(days_str) = s.strip_suffix('d') {
        let days: i64 = days_str.parse()?;
        Ok(chrono::Duration::days(days))
    } else if let Some(hours_str) = s.strip_suffix('h') {
        let hours: i64 = hours_str.parse()?;
        Ok(chrono::Duration::hours(hours))
    } else {
        Err(anyhow::anyhow!(
            "Invalid duration format. Use '7d' or '24h'"
        ))
    }
}

// ========================================================================
// Cache command handlers
// ========================================================================

fn cache_command(action: CacheAction, json: bool) -> Result<()> {
    match action {
        CacheAction::Stats => cache_stats_command(json),
        CacheAction::Clear { yes } => cache_clear_command(yes, json),
        CacheAction::Export { output } => cache_export_command(output, json),
    }
}

fn get_cache_db_path() -> Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".apxm").join("cache.db"))
}

#[cfg(feature = "driver")]
fn cache_stats_command(json: bool) -> Result<()> {
    use rusqlite::Connection;

    let db_path = get_cache_db_path()?;

    if !db_path.exists() {
        if json {
            println!("{{\"exists\": false}}");
        } else {
            println!("No cache database found at {}", db_path.display());
        }
        return Ok(());
    }

    let conn = Connection::open(&db_path)?;

    // Get total count
    let total_entries: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memo_cache",
        [],
        |row| row.get(0),
    )?;

    // Get total size (approximate from content lengths)
    let total_size: i64 = conn.query_row(
        "SELECT SUM(LENGTH(content)) FROM memo_cache",
        [],
        |row| row.get(0),
    ).unwrap_or(0);

    // Get oldest and newest entries
    let oldest: Option<i64> = conn.query_row(
        "SELECT MIN(inserted_at) FROM memo_cache",
        [],
        |row| row.get(0),
    ).ok();

    let newest: Option<i64> = conn.query_row(
        "SELECT MAX(inserted_at) FROM memo_cache",
        [],
        |row| row.get(0),
    ).ok();

    // Get hit statistics (we don't track this in the schema, so we'll just show entry count)
    // In a real implementation, you'd add a hit_count column to track this

    if json {
        let output = serde_json::json!({
            "exists": true,
            "path": db_path.display().to_string(),
            "total_entries": total_entries,
            "total_size_bytes": total_size,
            "oldest_entry_timestamp": oldest,
            "newest_entry_timestamp": newest,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("MemoCache Statistics");
        println!("===================");
        println!("Database: {}", db_path.display());
        println!("Total entries: {}", total_entries);
        println!("Total size: {:.2} MB", total_size as f64 / 1_000_000.0);

        if let Some(oldest_ts) = oldest {
            use std::time::{SystemTime, UNIX_EPOCH, Duration};
            let oldest_time = UNIX_EPOCH + Duration::from_secs(oldest_ts as u64);
            if let Ok(duration) = SystemTime::now().duration_since(oldest_time) {
                println!("Oldest entry: {} days ago", duration.as_secs() / 86400);
            }
        }

        if let Some(newest_ts) = newest {
            use std::time::{SystemTime, UNIX_EPOCH, Duration};
            let newest_time = UNIX_EPOCH + Duration::from_secs(newest_ts as u64);
            if let Ok(duration) = SystemTime::now().duration_since(newest_time) {
                println!("Newest entry: {} seconds ago", duration.as_secs());
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "driver"))]
fn cache_stats_command(json: bool) -> Result<()> {
    if json {
        println!("{{\"error\": \"Cache commands require the driver feature\"}}");
    } else {
        println!("Cache commands require the driver feature");
        println!("Rebuild with: cargo build -p apxm-cli --features driver");
    }
    Err(anyhow::anyhow!("Driver feature required"))
}

#[cfg(feature = "driver")]
fn cache_clear_command(yes: bool, json: bool) -> Result<()> {
    use rusqlite::Connection;
    use std::io::{self, Write};

    let db_path = get_cache_db_path()?;

    if !db_path.exists() {
        if json {
            println!("{{\"exists\": false, \"cleared\": 0}}");
        } else {
            println!("No cache database found");
        }
        return Ok(());
    }

    let conn = Connection::open(&db_path)?;

    // Get count before clearing
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memo_cache",
        [],
        |row| row.get(0),
    )?;

    if count == 0 {
        if json {
            println!("{{\"exists\": true, \"cleared\": 0}}");
        } else {
            println!("Cache is already empty");
        }
        return Ok(());
    }

    if !yes && !json {
        print!("Delete {} cached entries? [y/N] ", count);
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled");
            return Ok(());
        }
    }

    conn.execute("DELETE FROM memo_cache", [])?;

    if json {
        println!("{{\"exists\": true, \"cleared\": {}}}", count);
    } else {
        println!("Cleared {} entries from cache", count);
    }

    Ok(())
}

#[cfg(not(feature = "driver"))]
fn cache_clear_command(_yes: bool, json: bool) -> Result<()> {
    if json {
        println!("{{\"error\": \"Cache commands require the driver feature\"}}");
    } else {
        println!("Cache commands require the driver feature");
    }
    Err(anyhow::anyhow!("Driver feature required"))
}

#[cfg(feature = "driver")]
fn cache_export_command(output: Option<PathBuf>, json_flag: bool) -> Result<()> {
    use rusqlite::Connection;

    let db_path = get_cache_db_path()?;

    if !db_path.exists() {
        if json_flag {
            println!("{{\"exists\": false, \"entries\": []}}");
        } else {
            println!("No cache database found");
        }
        return Ok(());
    }

    let conn = Connection::open(&db_path)?;
    let mut stmt = conn.prepare(
        "SELECT key, content, model, input_tokens, output_tokens, inserted_at, ttl_secs FROM memo_cache"
    )?;

    let entries: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "key": row.get::<_, i64>(0)?,
                "content": row.get::<_, String>(1)?,
                "model": row.get::<_, String>(2)?,
                "input_tokens": row.get::<_, i64>(3)?,
                "output_tokens": row.get::<_, i64>(4)?,
                "inserted_at": row.get::<_, i64>(5)?,
                "ttl_secs": row.get::<_, i64>(6)?,
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let output_json = serde_json::json!({
        "exists": true,
        "entries": entries,
        "total": entries.len(),
    });

    let json_str = serde_json::to_string_pretty(&output_json)?;

    if let Some(path) = output {
        std::fs::write(&path, json_str)?;
        if !json_flag {
            println!("Exported {} entries to {}", entries.len(), path.display());
        }
    } else {
        println!("{}", json_str);
    }

    Ok(())
}

#[cfg(not(feature = "driver"))]
fn cache_export_command(_output: Option<PathBuf>, json: bool) -> Result<()> {
    if json {
        println!("{{\"error\": \"Cache commands require the driver feature\"}}");
    } else {
        println!("Cache commands require the driver feature");
    }
    Err(anyhow::anyhow!("Driver feature required"))
}

// ========================================================================
// Workflow command handlers
// ========================================================================

#[cfg(feature = "driver")]
async fn workflow_command(action: WorkflowAction, json: bool) -> Result<()> {
    match action {
        WorkflowAction::Run { file, args } => workflow_run_command(file, args).await,
        WorkflowAction::Validate { file } => workflow_validate_command(file, json),
        WorkflowAction::Analyze { file } => workflow_analyze_command(file, json),
    }
}

#[cfg(not(feature = "driver"))]
fn workflow_command_no_driver(action: WorkflowAction, json: bool) -> Result<()> {
    match action {
        WorkflowAction::Validate { file } => workflow_validate_command(file, json),
        WorkflowAction::Analyze { file } => workflow_analyze_command(file, json),
        _ => Err(anyhow::anyhow!(
            "Workflow run requires the `driver` feature. Re-run with: cargo run -p apxm-cli --features driver -- workflow run"
        )),
    }
}

fn workflow_validate_command(file: PathBuf, json: bool) -> Result<()> {
    use apxm_runtime::workflow::WorkflowDef;

    let def = WorkflowDef::from_file(&file)
        .with_context(|| format!("Failed to load workflow {}", file.display()))?;

    let errors = def.validate();

    if json {
        let output = serde_json::json!({
            "valid": errors.is_empty(),
            "errors": errors,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if errors.is_empty() {
            println!(
                "{} Workflow is valid",
                apxm_core::constants::ui::icons::SUCCESS
            );
            println!("  Name: {}", def.name);
            println!("  Steps: {}", def.graphs.len());
            println!(
                "  Parameters: {}",
                def.parameters
                    .iter()
                    .map(|p| format!("{}: {}", p.name, p.type_name))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        } else {
            println!(
                "{} Validation failed:",
                apxm_core::constants::ui::icons::FAILED
            );
            for error in &errors {
                println!("  - {}", error);
            }
            return Err(anyhow::anyhow!("Validation failed"));
        }
    }

    Ok(())
}

fn workflow_analyze_command(file: PathBuf, json: bool) -> Result<()> {
    use apxm_runtime::workflow::{WorkflowDef, execution_phases};

    let def = WorkflowDef::from_file(&file)
        .with_context(|| format!("Failed to load workflow {}", file.display()))?;

    let errors = def.validate();
    if !errors.is_empty() {
        return Err(anyhow::anyhow!(
            "Workflow validation failed: {}",
            errors.join(", ")
        ));
    }

    let phases = execution_phases(&def.graphs)?;

    if json {
        let output = serde_json::json!({
            "name": def.name,
            "total_steps": def.graphs.len(),
            "phases": phases.len(),
            "max_parallelism": phases.iter().map(|p| p.len()).max().unwrap_or(0),
            "execution_plan": phases,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Workflow: {}", def.name.bold());
        println!();
        println!("  Total steps: {}", def.graphs.len());
        println!("  Execution phases: {}", phases.len());
        println!(
            "  Max parallelism: {}",
            phases.iter().map(|p| p.len()).max().unwrap_or(0)
        );
        println!();
        println!("Execution plan:");
        for (i, phase) in phases.iter().enumerate() {
            println!("  Phase {}: {} step(s) in parallel", i, phase.len());
            for step_id in phase {
                let step = def.graphs.iter().find(|s| &s.id == step_id).unwrap();
                println!("    - {} ({})", step_id, step.path);
            }
        }
    }

    Ok(())
}

#[cfg(feature = "driver")]
async fn workflow_run_command(file: PathBuf, args: Vec<String>) -> Result<()> {
    use apxm_runtime::workflow::{WorkflowDef, execution_phases};
    use std::time::Instant;

    // Parse workflow
    let def = WorkflowDef::from_file(&file)
        .with_context(|| format!("Failed to load workflow {}", file.display()))?;

    // Validate workflow
    let errors = def.validate();
    if !errors.is_empty() {
        return Err(anyhow::anyhow!(
            "Workflow validation failed: {}",
            errors.join(", ")
        ));
    }

    // Parse arguments (name=value format)
    let mut params = HashMap::new();
    for arg in &args {
        if let Some((key, value)) = arg.split_once('=') {
            params.insert(key.to_string(), value.to_string());
        } else {
            return Err(anyhow::anyhow!(
                "Invalid argument format '{}'. Expected name=value",
                arg
            ));
        }
    }

    // Check that all required parameters are provided
    for param in &def.parameters {
        if !params.contains_key(&param.name) {
            return Err(anyhow::anyhow!(
                "Missing required parameter: {} (type: {})",
                param.name,
                param.type_name
            ));
        }
    }

    println!("Executing workflow: {}", def.name.bold());
    println!();

    let base_dir = file
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Failed to get parent directory"))?
        .to_path_buf();

    let phases = execution_phases(&def.graphs)?;
    let start = Instant::now();

    // Create workflow session directory
    let paths = apxm_core::paths::ApxmPaths::discover()?;
    let workflow_session_dir = paths.sessions_dir()?.join(format!(
        "workflow-{}-{}",
        def.name,
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    ));

    println!("Session directory: {}", workflow_session_dir.display());
    println!();

    let mut step_outputs: HashMap<String, String> = HashMap::new();
    let mut step_results: HashMap<String, apxm_runtime::workflow::StepResult> = HashMap::new();

    // Load config and create linker
    let config = load_config(None)?;
    let linker_config = LinkerConfig::from_apxm_config(config);
    let linker = Linker::new(linker_config).await?;

    // Execute phases
    for (phase_idx, phase) in phases.iter().enumerate() {
        println!("Phase {}: {} step(s)", phase_idx, phase.len());

        for step_id in phase {
            let step = def.graphs.iter().find(|s| &s.id == step_id).unwrap();

            // Check if any dependency failed → skip this step
            let should_skip = step.depends_on.iter().any(|dep| {
                step_results.get(dep).map_or(false, |r| {
                    r.status != apxm_runtime::workflow::StepStatus::Success
                })
            });

            if should_skip {
                println!("  {} Skipping (failed dependency)", step_id);
                step_results.insert(
                    step_id.clone(),
                    apxm_runtime::workflow::StepResult {
                        id: step_id.clone(),
                        status: apxm_runtime::workflow::StepStatus::Skipped,
                        ..Default::default()
                    },
                );
                continue;
            }

            // Resolve parameters
            let resolved_params: HashMap<String, String> = step
                .params
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        apxm_runtime::workflow::resolve(v, &step_outputs, &params),
                    )
                })
                .collect();

            let step_session_dir = workflow_session_dir.join(&step.id);
            std::fs::create_dir_all(&step_session_dir)?;

            let graph_path = base_dir.join(&step.path);

            println!(
                "  {} Starting: {} ({})",
                apxm_core::constants::ui::icons::STARTED,
                step_id,
                graph_path.display()
            );

            let step_start = Instant::now();

            // Convert resolved params to Vec<String> for the graph's parameters
            let graph_args: Vec<String> = {
                // Load the graph to get its parameter order
                let graph_bytes = std::fs::read(&graph_path)?;
                let graph_text = std::str::from_utf8(&graph_bytes)?;
                let graph = apxm_graph::ApxmGraph::from_json(graph_text)?;

                graph
                    .parameters
                    .iter()
                    .map(|p| resolved_params.get(&p.name).cloned().unwrap_or_default())
                    .collect()
            };

            // Execute the graph
            let result = linker
                .run_graph(&graph_path, graph_args, None, Some(&step_session_dir))
                .await;

            let duration_ms = step_start.elapsed().as_millis() as u64;

            match result {
                Ok(link_result) => {
                    // Extract the final output value
                    let output =
                        link_result
                            .execution
                            .results
                            .values()
                            .last()
                            .and_then(|v| match v {
                                apxm_core::types::Value::String(s) => Some(s.clone()),
                                _ => Some(v.to_string()),
                            });

                    println!(
                        "  {} Completed: {} ({:.1}s)",
                        apxm_core::constants::ui::icons::SUCCESS,
                        step_id,
                        duration_ms as f64 / 1000.0
                    );

                    if let Some(ref out) = output {
                        step_outputs.insert(step_id.clone(), out.clone());
                    }

                    step_results.insert(
                        step_id.clone(),
                        apxm_runtime::workflow::StepResult {
                            id: step_id.clone(),
                            status: apxm_runtime::workflow::StepStatus::Success,
                            output,
                            duration_ms,
                            session_dir: Some(step_session_dir),
                            error: None,
                        },
                    );
                }
                Err(e) => {
                    println!(
                        "  {} Failed: {} - {}",
                        apxm_core::constants::ui::icons::FAILED,
                        step_id,
                        e
                    );

                    step_results.insert(
                        step_id.clone(),
                        apxm_runtime::workflow::StepResult {
                            id: step_id.clone(),
                            status: apxm_runtime::workflow::StepStatus::Failed,
                            output: None,
                            duration_ms,
                            session_dir: Some(step_session_dir),
                            error: Some(e.to_string()),
                        },
                    );
                }
            }
        }

        println!();
    }

    // Resolve final output
    let output = def
        .output
        .as_ref()
        .map(|tmpl| apxm_runtime::workflow::resolve(tmpl, &step_outputs, &params));

    let total_duration = start.elapsed();

    println!("Workflow completed in {:.1}s", total_duration.as_secs_f64());
    println!();
    println!("Results:");
    for (step_id, result) in &step_results {
        let status_icon = match result.status {
            apxm_runtime::workflow::StepStatus::Success => apxm_core::constants::ui::icons::SUCCESS,
            apxm_runtime::workflow::StepStatus::Failed => apxm_core::constants::ui::icons::FAILED,
            apxm_runtime::workflow::StepStatus::Skipped => apxm_core::constants::ui::icons::WARNING,
        };
        println!(
            "  {} {}: {:?} ({:.1}s)",
            status_icon,
            step_id,
            result.status,
            result.duration_ms as f64 / 1000.0
        );
    }

    if let Some(output) = output {
        println!();
        println!("Final output:");
        println!("{}", output);
    }

    Ok(())
}

#[cfg(feature = "driver")]
fn load_config(config: Option<PathBuf>) -> Result<ApXmConfig> {
    if let Some(path) = config {
        return ApXmConfig::from_file(&path)
            .map_err(|e| anyhow::anyhow!(e))
            .with_context(|| format!("Failed to load config {}", path.display()));
    }

    match ApXmConfig::load_scoped() {
        Ok(config) => Ok(config),
        Err(ConfigError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(ApXmConfig::default())
        }
        Err(ConfigError::HomeDirMissing) => Ok(ApXmConfig::default()),
        Err(err) => Err(anyhow::anyhow!(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── category_str tests ──────────────────────────────────────────────────

    #[test]
    fn category_str_reasoning() {
        use apxm_core::types::OperationCategory;
        assert_eq!(category_str(OperationCategory::Reasoning), "reasoning");
    }

    #[test]
    fn category_str_memory() {
        use apxm_core::types::OperationCategory;
        assert_eq!(category_str(OperationCategory::Memory), "memory");
    }

    #[test]
    fn category_str_tools() {
        use apxm_core::types::OperationCategory;
        assert_eq!(category_str(OperationCategory::Tools), "tools");
    }

    #[test]
    fn category_str_control_flow() {
        use apxm_core::types::OperationCategory;
        assert_eq!(category_str(OperationCategory::ControlFlow), "control_flow");
    }

    #[test]
    fn category_str_all_variants() {
        use apxm_core::types::OperationCategory;
        // Ensure every variant maps to a non-empty string
        let variants = [
            OperationCategory::Metadata,
            OperationCategory::Memory,
            OperationCategory::Reasoning,
            OperationCategory::Tools,
            OperationCategory::ControlFlow,
            OperationCategory::Synchronization,
            OperationCategory::ErrorHandling,
            OperationCategory::Communication,
            OperationCategory::Internal,
            OperationCategory::Coordination,
            OperationCategory::Identity,
        ];
        for cat in variants {
            let s = category_str(cat);
            assert!(!s.is_empty(), "category_str returned empty for {:?}", cat);
        }
    }

    // ── find_op_spec tests ──────────────────────────────────────────────────

    #[test]
    fn find_op_spec_ask() {
        let spec = find_op_spec("ASK");
        assert!(spec.is_some(), "ASK should be a valid operation");
        let spec = spec.unwrap();
        assert_eq!(spec.op_type.to_string(), "ASK");
    }

    #[test]
    fn find_op_spec_think() {
        let spec = find_op_spec("THINK");
        assert!(spec.is_some(), "THINK should be a valid operation");
    }

    #[test]
    fn find_op_spec_inv() {
        // The canonical name is INV_TOOL (to_string() on AISOperationType::InvTool).
        // "INV" is the legacy .fromStr alias but not the Display name.
        let spec = find_op_spec("INV_TOOL");
        assert!(spec.is_some(), "INV_TOOL should be a valid operation");
    }

    #[test]
    fn find_op_spec_nonexistent() {
        assert!(find_op_spec("DOES_NOT_EXIST").is_none());
        assert!(find_op_spec("").is_none());
        assert!(find_op_spec("ask").is_none()); // case-sensitive
    }

    #[test]
    fn find_op_spec_returns_correct_category() {
        use apxm_core::types::OperationCategory;
        let ask = find_op_spec("ASK").unwrap();
        assert_eq!(ask.category, OperationCategory::Reasoning);

        let inv = find_op_spec("INV_TOOL").unwrap();
        assert_eq!(inv.category, OperationCategory::Tools);
    }

    // ── GraphAnalysis tests ─────────────────────────────────────────────────

    fn pipeline_graph() -> apxm_graph::ApxmGraph {
        serde_json::from_value(serde_json::json!({
            "name": "test-pipeline",
            "nodes": [
                {"id": 1, "name": "step-a", "op": "ASK", "attributes": {"template_str": "a"}},
                {"id": 2, "name": "step-b", "op": "ASK", "attributes": {"template_str": "b"}},
                {"id": 3, "name": "step-c", "op": "ASK", "attributes": {"template_str": "c"}}
            ],
            "edges": [
                {"from": 1, "to": 2, "dependency": "Data"},
                {"from": 2, "to": 3, "dependency": "Data"}
            ],
            "parameters": [],
            "metadata": {}
        }))
        .unwrap()
    }

    fn fanout_graph() -> apxm_graph::ApxmGraph {
        serde_json::from_value(serde_json::json!({
            "name": "test-fanout",
            "nodes": [
                {"id": 1, "name": "a", "op": "ASK", "attributes": {"template_str": "a"}},
                {"id": 2, "name": "b", "op": "ASK", "attributes": {"template_str": "b"}},
                {"id": 3, "name": "c", "op": "ASK", "attributes": {"template_str": "c"}},
                {"id": 4, "name": "sync", "op": "WAIT_ALL", "attributes": {"tokens": []}}
            ],
            "edges": [
                {"from": 1, "to": 4, "dependency": "Data"},
                {"from": 2, "to": 4, "dependency": "Data"},
                {"from": 3, "to": 4, "dependency": "Data"}
            ],
            "parameters": [],
            "metadata": {}
        }))
        .unwrap()
    }

    fn single_node_graph() -> apxm_graph::ApxmGraph {
        serde_json::from_value(serde_json::json!({
            "name": "single",
            "nodes": [
                {"id": 1, "name": "only", "op": "ASK", "attributes": {"template_str": "hi"}}
            ],
            "edges": [],
            "parameters": [],
            "metadata": {}
        }))
        .unwrap()
    }

    #[test]
    fn graph_analysis_pipeline_basic_properties() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        assert_eq!(ga.graph.name, "test-pipeline");
        assert_eq!(ga.graph.nodes.len(), 3);
        assert_eq!(ga.edge_count, 2);
        assert_eq!(ga.node_ids.len(), 3);
    }

    #[test]
    fn graph_analysis_pipeline_entry_exit_nodes() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // In a pipeline A->B->C, entry is [1] and exit is [3]
        assert_eq!(ga.entry_nodes, vec![1]);
        assert_eq!(ga.exit_nodes, vec![3]);
    }

    #[test]
    fn graph_analysis_pipeline_phases() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // Sequential pipeline should have 3 phases, each with 1 node
        assert_eq!(ga.phases.len(), 3);
        assert_eq!(ga.phases[0], vec![1]);
        assert_eq!(ga.phases[1], vec![2]);
        assert_eq!(ga.phases[2], vec![3]);
    }

    #[test]
    fn graph_analysis_pipeline_max_parallelism() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // Sequential => max parallelism = 1
        assert_eq!(ga.max_parallelism(), 1);
    }

    #[test]
    fn graph_analysis_pipeline_speedup() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // Sequential => speedup ~1.0
        let speedup = ga.speedup();
        assert!(
            (speedup - 1.0).abs() < 0.01,
            "sequential pipeline speedup should be ~1.0, got {}",
            speedup,
        );
    }

    #[test]
    fn graph_analysis_pipeline_critical_path() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        let (path, ms) = ga.critical_path();
        // Critical path includes all 3 nodes in sequence
        assert_eq!(path.len(), 3);
        assert_eq!(path, vec![1, 2, 3]);
        assert!(ms > 0, "critical path ms should be positive");
    }

    #[test]
    fn graph_analysis_fanout_max_parallelism() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // Nodes 1,2,3 are all entry nodes with no predecessors => phase 1 has 3 nodes
        assert_eq!(ga.max_parallelism(), 3);
    }

    #[test]
    fn graph_analysis_fanout_phases() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // Phase 1: [1,2,3] parallel, Phase 2: [4] sync
        assert_eq!(ga.phases.len(), 2);
        assert_eq!(ga.phases[0].len(), 3);
        assert_eq!(ga.phases[1], vec![4]);
    }

    #[test]
    fn graph_analysis_fanout_speedup_greater_than_one() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        let speedup = ga.speedup();
        assert!(
            speedup > 1.0,
            "fan-out graph should have speedup > 1.0, got {}",
            speedup,
        );
    }

    #[test]
    fn graph_analysis_fanout_entry_exit() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        // Entry: 1,2,3 (sorted)
        let mut entries = ga.entry_nodes.clone();
        entries.sort();
        assert_eq!(entries, vec![1, 2, 3]);

        // Exit: 4
        assert_eq!(ga.exit_nodes, vec![4]);
    }

    #[test]
    fn graph_analysis_single_node() {
        let raw = single_node_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        assert_eq!(ga.graph.name, "single");
        assert_eq!(ga.graph.nodes.len(), 1);
        assert_eq!(ga.edge_count, 0);
        assert_eq!(ga.entry_nodes, vec![1]);
        assert_eq!(ga.exit_nodes, vec![1]);
        assert_eq!(ga.phases.len(), 1);
        assert_eq!(ga.max_parallelism(), 1);
        assert!((ga.speedup() - 1.0).abs() < 0.01);
    }

    #[test]
    fn graph_analysis_node_accessors() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        assert_eq!(ga.node_op(1), "ASK");
        assert_eq!(ga.node_name(1), "step-a");
        assert_eq!(ga.node_name(2), "step-b");
        assert_eq!(ga.node_name(3), "step-c");

        // Non-existent node returns "?"
        assert_eq!(ga.node_op(999), "?");
        assert_eq!(ga.node_name(999), "?");
    }

    #[test]
    fn graph_analysis_critical_path_fanout() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        let (path, ms) = ga.critical_path();
        // Critical path goes through one of the parallel nodes + sync
        assert_eq!(path.len(), 2);
        // The last node on the critical path should be the sync node
        assert_eq!(*path.last().unwrap(), 4);
        assert!(ms > 0);
    }

    #[test]
    fn graph_analysis_sequential_vs_parallel_ms() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_graph(&raw);

        let seq = ga.sequential_ms();
        let par = ga.parallel_ms();
        // Sequential sum should be >= parallel sum
        assert!(
            seq >= par,
            "sequential {}ms should be >= parallel {}ms",
            seq,
            par
        );
    }

    #[test]
    fn graph_analysis_no_nodes_errors() {
        let result =
            serde_json::from_value::<apxm_graph::ApxmGraph>(serde_json::json!({"name": "empty"}));
        assert!(result.is_err());
    }
}
