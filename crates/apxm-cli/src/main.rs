//! APXM command-line interface.
//!
//! Provides the `apxm` binary with subcommands for building, compiling,
//! running agent workflows, environment setup (`install`, `doctor`), and
//! LLM credential management (`register`).

use std::path::PathBuf;

#[cfg(feature = "driver")]
use anyhow::Context;
use anyhow::Result;
use apxm_core::utils::build::MlirEnvReport;
use apxm_credentials::CredentialStore;
use apxm_credentials::credential::Credential;
#[cfg(feature = "driver")]
use apxm_driver::compiler::Compiler;
#[cfg(feature = "driver")]
use apxm_driver::{ApXmConfig, ConfigError, Linker, LinkerConfig};
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::collections::{BTreeMap, HashMap, HashSet};
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

    let filter =
        EnvFilter::try_new(&filter_str).unwrap_or_else(|_| EnvFilter::new("apxm=info"));

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
    /// Compile ApxmGraph JSON/binary to an artifact
    Compile {
        /// Input graph file or directory (.json or JSON-encoded binary)
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
        /// Input graph file (.json or JSON-encoded binary)
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
    /// Manage LLM provider credentials (deprecated, use 'backend')
    Llm {
        #[command(subcommand)]
        action: LlmAction,
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
    /// Manage and inspect model definitions from ~/.apxm/models.toml
    Models {
        #[command(subcommand)]
        action: ModelsAction,
    },
    /// Browse AIS operations (the agent instruction set)
    Ops {
        #[command(subcommand)]
        action: OpsAction,
    },
    /// Validate an ApxmGraph JSON file against the AIS contract
    Validate {
        /// Input graph file (.json)
        input: PathBuf,
    },
    /// Analyze an ApxmGraph for parallelism, critical path, and execution phases
    Analyze {
        /// Input graph file (.json)
        input: PathBuf,
    },
    /// Browse graph templates (starter patterns)
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },
    /// Explain what a graph does in human-readable terms
    Explain {
        /// Path to the ApxmGraph JSON file
        file: PathBuf,
    },
    /// Compose graph fragments (tasks)
    Task {
        #[command(subcommand)]
        action: TaskAction,
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
        /// Backend type (cloud, onprem, local)
        #[arg(long)]
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
}

#[derive(Subcommand)]
enum LlmAction {
    /// Register a new LLM provider credential
    Add {
        /// Credential name (e.g., "my-openai")
        name: String,
        /// Provider type (openai, anthropic, google, openrouter, ollama)
        #[arg(long)]
        provider: String,
        /// API key (omit to enter interactively)
        #[arg(long)]
        api_key: Option<String>,
        /// Base URL override
        #[arg(long)]
        base_url: Option<String>,
        /// Default model
        #[arg(long)]
        model: Option<String>,
        /// Extra headers as key=value pairs
        #[arg(long, value_parser = parse_header)]
        header: Vec<(String, String)>,
    },
    /// List registered credentials
    List,
    /// Remove a credential
    Remove {
        /// Name of the credential to remove
        name: String,
    },
    /// Validate a credential by making a test API call
    Test {
        /// Name of the credential to test (omit to test all)
        name: Option<String>,
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
    /// List available ACP agents (built-in + user-registered)
    List,
    /// Add or override an agent profile
    Add {
        /// Agent profile name (e.g., "my-agent")
        name: String,
        /// Agent command to spawn (e.g., "my-agent --acp")
        #[arg(long)]
        command: String,
        /// Permission mode (approve-all, approve-reads, deny-all)
        #[arg(long, default_value = "approve-reads")]
        permissions: String,
        /// Grace period in ms after closing stdin before SIGTERM
        #[arg(long)]
        close_grace_ms: Option<u64>,
    },
    /// Remove a user-registered agent profile
    Remove {
        /// Agent profile name to remove
        name: String,
    },
    /// Test spawning an agent (runs initialize + session/new + close)
    Test {
        /// Agent profile name to test
        name: String,
    },
}

/// Actions for `apxm models`
#[derive(Subcommand, Debug)]
enum ModelsAction {
    /// List all models defined in ~/.apxm/models.toml
    List,
    /// Show circuit-breaker health for all registered LLM backends
    Health,
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
    match action {
        AgentAction::List => {
            let reg = apxm_acp::AgentRegistry::load();
            let list = reg.list();
            if json_output {
                let entries: Vec<serde_json::Value> = list
                    .iter()
                    .map(|(name, profile, builtin)| {
                        serde_json::json!({
                            "name": name,
                            "command": profile.command,
                            "source": if *builtin { "builtin" } else { "user" },
                            "close_grace_ms": profile.close_grace_ms,
                            "session_create_timeout_ms": profile.session_create_timeout_ms,
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

            print_section_header("ACP Agent Profiles");
            let max_name = list.iter().map(|(n, _, _)| n.len()).max().unwrap_or(8);
            let max_source = 7; // "builtin"
            println!(
                "  {:<name_w$}  {:<src_w$}  {}",
                "PROFILE",
                "SOURCE",
                "COMMAND",
                name_w = max_name,
                src_w = max_source,
            );
            for (name, profile, builtin) in &list {
                let source = if *builtin { "builtin" } else { "user" };
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
                "{} agent profile{} registered",
                list.len(),
                if list.len() == 1 { "" } else { "s" }
            );
        }
        AgentAction::Add {
            name,
            command,
            permissions,
            close_grace_ms,
        } => {
            let permission_mode: apxm_acp::PermissionMode = permissions
                .parse()
                .map_err(|e: String| anyhow::anyhow!(e))?;
            use apxm_acp::constants::timeouts as acp_timeouts;
            let profile = apxm_acp::AgentProfile {
                command: command.clone(),
                close_grace_ms: close_grace_ms.unwrap_or(acp_timeouts::DEFAULT_CLOSE_GRACE_MS),
                session_create_timeout_ms: acp_timeouts::DEFAULT_SESSION_TIMEOUT_MS,
                permission_mode,
                env: Default::default(),
                default_mode: None,
                default_model: None,
                system_prompt: None,
                capabilities: Vec::new(),
            };
            let mut reg = apxm_acp::AgentRegistry::load();
            reg.add(name.clone(), profile)
                .map_err(|e| anyhow::anyhow!("Failed to save agent profile: {e}"))?;
            print_section_header("Agent Profile Added");
            print_status_line(&name, Status::Ok, &command);
        }
        AgentAction::Remove { name } => {
            let mut reg = apxm_acp::AgentRegistry::load();
            if reg.is_builtin(&name) {
                return Err(anyhow::anyhow!(
                    "Cannot remove built-in agent '{name}'. You can override it with: apxm agent add {name} --command ..."
                ));
            }
            match reg.remove(&name) {
                Ok(true) => {
                    print_section_header("Agent Profile Removed");
                    print_status_line(&name, Status::Ok, "removed");
                }
                Ok(false) => {
                    return Err(anyhow::anyhow!("Agent profile '{name}' not found"));
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to remove agent profile: {e}"));
                }
            }
        }
        AgentAction::Test { name } => {
            let reg = apxm_acp::AgentRegistry::load();
            let profile = reg.get(&name).ok_or_else(|| {
                anyhow::anyhow!("Agent profile '{name}' not found. Run: apxm agent list")
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
    }
    Ok(())
}

async fn models_command(action: ModelsAction, json_output: bool) -> Result<()> {
    use apxm_runtime::model_router::ModelRegistry;

    match action {
        ModelsAction::List => {
            let registry = ModelRegistry::load_from_default_path();
            let models = registry.list();

            if json_output {
                println!("{}", serde_json::to_string_pretty(&models.iter().map(|m| {
                    serde_json::json!({
                        "name": m.name,
                        "backend": m.backend,
                        "cost_per_1k_input": m.cost_per_1k_input,
                        "cost_per_1k_output": m.cost_per_1k_output,
                        "context_window": m.context_window,
                        "tags": m.tags,
                        "supports_thinking": m.supports_thinking,
                    })
                }).collect::<Vec<_>>())?);
                return Ok(());
            }

            if models.is_empty() {
                println!("No models defined. Create ~/.apxm/models.toml to add model definitions.");
                println!();
                println!("Example:");
                println!("  [[models]]");
                println!("  name = \"claude-sonnet-4-5\"");
                println!("  backend = \"anthropic\"");
                println!("  cost_per_1k_input = 0.003");
                println!("  cost_per_1k_output = 0.015");
                println!("  context_window = 200000");
                println!("  tags = [\"production\"]");
                return Ok(());
            }

            let default_model = registry.default_model();

            println!("Models ({} defined):", models.len());
            println!();
            for m in &models {
                let is_default = default_model.as_deref() == Some(&m.name);
                let default_marker = if is_default { " [default]" } else { "" };
                println!("  {}{}", m.name.bold(), default_marker.dimmed());
                println!("    Backend:        {}", m.backend);
                if m.context_window > 0 {
                    println!("    Context window: {} tokens", format_number(m.context_window));
                }
                if m.cost_per_1k_input > 0.0 || m.cost_per_1k_output > 0.0 {
                    println!("    Cost:           ${:.4}/1k in, ${:.4}/1k out",
                        m.cost_per_1k_input, m.cost_per_1k_output);
                }
                if !m.tags.is_empty() {
                    println!("    Tags:           {}", m.tags.join(", "));
                }
                if m.supports_thinking {
                    println!("    Thinking:       supported");
                }
                println!();
            }

            let routing = registry.routing();
            if !routing.prefer_tags.is_empty() {
                println!("Routing policy:");
                println!("  Prefer tags:   {}", routing.prefer_tags.join(", "));
                if !routing.fallback_tags.is_empty() {
                    println!("  Fallback tags: {}", routing.fallback_tags.join(", "));
                }
            }
        }

        ModelsAction::Health => {
            // Load backends from the credential store (same logic as llm_command)
            let mut backend_names: Vec<String> = Vec::new();
            if let Ok(store) = apxm_credentials::CredentialStore::open()
                && let Ok(credentials) = store.list_all()
            {
                for (name, _) in credentials {
                    backend_names.push(name);
                }
            }

            // Also include any model registry entries
            let registry = ModelRegistry::load_from_default_path();
            for m in registry.list() {
                if !backend_names.contains(&m.backend) {
                    backend_names.push(m.backend.clone());
                }
            }

            if json_output {
                let statuses: Vec<serde_json::Value> = backend_names.iter().map(|name| {
                    serde_json::json!({
                        "backend": name,
                        "state": "Unknown",
                        "note": "Circuit breaker state is per-process; use runtime metrics for live state"
                    })
                }).collect();
                println!("{}", serde_json::to_string_pretty(&statuses)?);
                return Ok(());
            }

            if backend_names.is_empty() {
                println!("No backends registered. Run: apxm llm register");
                return Ok(());
            }

            println!("LLM Backend Health:");
            println!();
            println!("  {:30} {}", "Backend".bold(), "Status".bold());
            println!("  {}", "-".repeat(60));

            // We can't read live circuit-breaker state from CLI (it's per-process).
            // Show registered backends + note that live state is in the running runtime.
            for name in &backend_names {
                let status = "Registered (circuit breaker active at runtime)";
                println!("  {:30} {}", name, status.dimmed());
            }

            println!();
            println!("Note: Circuit breaker state (Closed/Open/HalfOpen) is tracked");
            println!("      per-process within the running runtime. Use tracing logs");
            println!("      (RUST_LOG=apxm_runtime=info) to observe live state changes.");
            println!();

            // Show models.toml routing config
            let routing = registry.routing();
            if !routing.prefer_tags.is_empty() {
                println!("Routing policy (from ~/.apxm/models.toml):");
                println!("  Prefer tags:   {}", routing.prefer_tags.join(", "));
                if !routing.fallback_tags.is_empty() {
                    println!("  Fallback tags: {}", routing.fallback_tags.join(", "));
                }
            }
        }
    }
    Ok(())
}

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
            no_cse_llm,
        } => compile_command(input, output, emit_diagnostics, opt_level, no_cse_llm),
        Commands::Decompile { artifact, output } => decompile_command(artifact, output),
        Commands::Execute {
            input,
            args,
            opt_level,
            emit_metrics,
        } => execute_command(input, args, opt_level, cli.config, emit_metrics).await,
        Commands::Run {
            input,
            args,
            emit_metrics,
        } => run_command(input, args, cli.config, emit_metrics).await,
        Commands::Doctor => doctor_command(cli.config, cli.json),
        Commands::Activate { shell } => activate_command(&shell),
        Commands::Install => install_command(),
        Commands::Backend { action } => backend_command(action, cli.json).await,
        Commands::Llm { action } => llm_command(action).await,
        Commands::Tool { action } => tool_command(action, cli.json),
        Commands::Agent { action } => agent_command(action, cli.json).await,
        Commands::Models { action } => models_command(action, cli.json).await,
        Commands::Ops { action } => ops_command(action, cli.json),
        Commands::Validate { input } => validate_command(input, cli.json),
        Commands::Analyze { input } => analyze_command(input, cli.json),
        Commands::Template { action } => template_command(action, cli.json),
        Commands::Explain { file } => explain_command(file, cli.json),
        Commands::Task { action } => task_command(action, cli.json),
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
        Commands::Llm { action } => llm_command(action).await,
        Commands::Tool { action } => tool_command(action, cli.json),
        Commands::Agent { action } => agent_command(action, cli.json).await,
        Commands::Models { action } => models_command(action, cli.json).await,
        Commands::Ops { action } => ops_command(action, cli.json),
        Commands::Validate { input } => validate_command(input, cli.json),
        Commands::Analyze { input } => analyze_command(input, cli.json),
        Commands::Template { action } => template_command(action, cli.json),
        Commands::Explain { file } => explain_command(file, cli.json),
        Commands::Task { action } => task_command(action, cli.json),
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
    no_cse_llm: bool,
) -> Result<()> {
    use apxm_core::constants::diagnostics;
    use apxm_core::types::PipelineConfig;

    let opt = parse_opt_level(opt_level);

    let compile_start = std::time::Instant::now();
    let compiler = Compiler::with_opt_level(opt).context("Failed to initialize compiler")?;

    let graph = if input.is_dir() {
        load_graph_from_directory(&input)?
    } else {
        compiler
            .load_graph(&input)
            .map_err(|e| anyhow::anyhow!("Failed to parse graph: {e}"))?
    };

    // When diagnostics are requested, use the per-pass metrics path.
    // Otherwise use the fast bulk-run path.
    let (module, pass_diagnostics) = if emit_diagnostics.is_some() {
        if no_cse_llm {
            let config = PipelineConfig {
                opt_level: opt,
                verify: true,
                no_cse_llm: true,
                ..Default::default()
            };
            let (m, d) = compiler
                .compile_graph_with_config_and_diagnostics(&graph, config)
                .map_err(|e| anyhow::anyhow!("Failed to compile graph: {e}"))?;
            (m, Some(d))
        } else {
            let (m, d) = compiler
                .compile_graph_with_diagnostics(&graph)
                .map_err(|e| anyhow::anyhow!("Failed to compile graph: {e}"))?;
            (m, Some(d))
        }
    } else if no_cse_llm {
        let config = PipelineConfig {
            opt_level: opt,
            verify: true,
            no_cse_llm: true,
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
            input.join(format!("{}.apxmobj", graph.name))
        } else {
            input.with_extension("apxmobj")
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
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
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
            "No graph JSON files found in directory '{}'",
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

#[cfg(feature = "driver")]
async fn execute_command(
    input: PathBuf,
    args: Vec<String>,
    opt_level: u8,
    config: Option<PathBuf>,
    emit_metrics: Option<PathBuf>,
) -> Result<()> {
    let apxm_config = load_config(config).context("Failed to load configuration")?;
    let opt = parse_opt_level(opt_level);
    let linker_config = LinkerConfig::from_apxm_config(apxm_config).with_opt_level(opt);
    let linker = Linker::new(linker_config)
        .await
        .context("Failed to initialize runtime")?;

    let result = match linker.run_graph(&input, args).await {
        Ok(r) => r,
        Err(err) => {
            eprintln!("{}", err);
            return Err(anyhow::anyhow!("Execution failed"));
        }
    };

    // Emit metrics JSON if requested
    if let Some(metrics_path) = emit_metrics {
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

        std::fs::write(&metrics_path, serde_json::to_string_pretty(&metrics_json)?)
            .with_context(|| format!("Failed to write metrics to {}", metrics_path.display()))?;

        println!("Wrote metrics to {}", metrics_path.display());
    }

    // Print workflow outputs
    if result.execution.results.is_empty() {
        eprintln!("Warning: No output values");
    } else {
        for (key, value) in &result.execution.results {
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
) -> Result<()> {
    use apxm_artifact::Artifact;
    use apxm_driver::runtime::RuntimeExecutor;

    // Validate file extension
    if input.extension().and_then(|e| e.to_str()) != Some("apxmobj") {
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
    let linker_config = LinkerConfig::from_apxm_config(apxm_config);
    let runtime = RuntimeExecutor::new(&linker_config)
        .await
        .context("Failed to initialize runtime")?;

    // Execute artifact with args
    let result = runtime
        .execute_artifact_with_args(artifact, args)
        .await
        .map_err(|e| anyhow::anyhow!("Execution failed: {}", e))?;

    // Emit metrics JSON if requested
    if let Some(metrics_path) = emit_metrics {
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

        std::fs::write(&metrics_path, serde_json::to_string_pretty(&metrics_json)?)
            .with_context(|| format!("Failed to write metrics to {}", metrics_path.display()))?;

        println!("Wrote metrics to {}", metrics_path.display());
    }

    Ok(())
}

async fn backend_command(action: BackendAction, json_output: bool) -> Result<()> {
    use apxm_credentials::backend::{BackendError, BackendStore};
    use apxm_core::types::{BackendConfig, BackendType, ProviderProtocol};
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
            // Parse backend type
            let backend_type = BackendType::from_str(&r#type)
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            // Parse protocol
            let protocol = ProviderProtocol::from_str(&protocol)
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            // Get API key if needed
            let api_key = if api_key.is_some() || backend_type == BackendType::Local {
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
                endpoint,
                api_key,
                headers,
                models: vec![],
                docker: None,
            };

            store.add(backend).map_err(|e| anyhow::anyhow!("{e}"))?;

            if json_output {
                println!("{{\"status\":\"ok\",\"backend\":\"{name}\"}}");
            } else {
                print_section_header("Backend Registered");
                print_status_line("Name", Status::Ok, &name);
                print_status_line("Type", Status::Ok, &format!("{backend_type}"));
                print_status_line("Protocol", Status::Ok, &format!("{protocol}"));
                print_status_line("Store", Status::Ok, &store.path().display().to_string());
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
                // TODO: Implement actual backend validation
                // For now, just check that required fields are present
                let result = if backend.api_key.is_some() || backend.backend_type == BackendType::Local {
                    Ok("configuration valid".to_string())
                } else {
                    Err(BackendError::NotFound {
                        name: "api_key missing".to_string(),
                    })
                };

                match result {
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
                eprintln!("This will migrate credentials from ~/.apxm/credentials.toml to ~/.apxm/config.toml");
                eprint!("Continue? [y/N] ");
                use std::io::{self, BufRead};
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line)?;
                if !line.trim().eq_ignore_ascii_case("y") {
                    println!("Aborted.");
                    return Ok(());
                }
            }

            let count = store.migrate_from_credentials().map_err(|e| anyhow::anyhow!("{e}"))?;

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
    }

    Ok(())
}

async fn llm_command(action: LlmAction) -> Result<()> {
    let store = CredentialStore::open().map_err(|e| anyhow::anyhow!("{e}"))?;

    match action {
        LlmAction::Add {
            name,
            provider,
            api_key,
            base_url,
            model,
            header,
        } => {
            let api_key = if api_key.is_some() || provider == "ollama" {
                api_key
            } else {
                eprint!("Enter API key for {name}: ");
                let key = rpassword::read_password()
                    .map_err(|e| anyhow::anyhow!("Failed to read API key: {e}"))?;
                if key.is_empty() { None } else { Some(key) }
            };

            let headers: BTreeMap<String, String> = header.into_iter().collect();

            let cred = Credential {
                provider: provider.clone(),
                api_key,
                base_url,
                model,
                headers,
            };

            store.add(&name, cred).map_err(|e| anyhow::anyhow!("{e}"))?;

            print_section_header("Credential Registered");
            print_status_line("Name", Status::Ok, &name);
            print_status_line("Provider", Status::Ok, &provider);
            print_status_line("Store", Status::Ok, &store.path().display().to_string());
        }
        LlmAction::List => {
            let creds = store.list().map_err(|e| anyhow::anyhow!("{e}"))?;
            if creds.is_empty() {
                println!("No credentials registered.");
                println!("Add one with: apxm llm add <name> --provider <provider>");
                return Ok(());
            }

            print_section_header("Registered Credentials");
            for (name, summary) in &creds {
                let key_display = summary.masked_key.as_deref().unwrap_or("<none>");
                println!(
                    "  {:<16} {:<12} key={}{}{}",
                    name.bold(),
                    summary.provider,
                    key_display,
                    summary
                        .model
                        .as_ref()
                        .map(|m| format!("  model={m}"))
                        .unwrap_or_default(),
                    if summary.header_count > 0 {
                        format!("  +{} headers", summary.header_count)
                    } else {
                        String::new()
                    }
                );
            }
            println!();
            println!("Store: {}", store.path().display());
        }
        LlmAction::Remove { name } => {
            store.remove(&name).map_err(|e| anyhow::anyhow!("{e}"))?;
            print_section_header("Credential Removed");
            print_status_line(&name, Status::Ok, "removed");
        }
        LlmAction::Test { name } => {
            let creds_to_test: Vec<(String, Credential)> = match name {
                Some(ref n) => {
                    let cred = store
                        .get(n)
                        .map_err(|e| anyhow::anyhow!("{e}"))?
                        .ok_or_else(|| anyhow::anyhow!("Credential '{n}' not found"))?;
                    vec![(n.clone(), cred)]
                }
                None => store.list_all().map_err(|e| anyhow::anyhow!("{e}"))?,
            };

            if creds_to_test.is_empty() {
                println!("No credentials to test.");
                return Ok(());
            }

            print_section_header("Testing Credentials");
            let mut all_ok = true;
            for (cname, cred) in &creds_to_test {
                match apxm_credentials::validate::validate_credential(cname, cred).await {
                    Ok(msg) => print_status_line(cname, Status::Ok, &msg),
                    Err(e) => {
                        print_status_line(cname, Status::Error, &e.to_string());
                        all_ok = false;
                    }
                }
            }
            if !all_ok {
                return Err(anyhow::anyhow!("Some credentials failed validation"));
            }
        }
    }

    Ok(())
}

fn validate_command(input: PathBuf, json_output: bool) -> Result<()> {
    use apxm_core::types::AIS_OPERATIONS;
    use std::collections::HashSet;

    let content = std::fs::read_to_string(&input)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", input.display()))?;

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // Parse JSON
    let raw: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Invalid JSON in {}: {e}", input.display()))?;

    // Top-level checks
    if raw
        .get("name")
        .and_then(serde_json::Value::as_str)
        .is_none_or(|s| s.is_empty())
    {
        errors.push("graph name must not be empty".to_string());
    }

    let nodes = raw.get("nodes").and_then(serde_json::Value::as_array);
    if nodes.is_none() || nodes.is_some_and(|n| n.is_empty()) {
        errors.push("graph must contain at least one node".to_string());
    }

    // Node-level checks
    let mut node_ids: HashSet<u64> = HashSet::new();
    let valid_ops: HashSet<String> = AIS_OPERATIONS
        .iter()
        .map(|s| s.op_type.to_string())
        .collect();

    if let Some(nodes) = nodes {
        for node in nodes {
            let id = node
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let name = node
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let op = node
                .get("op")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");

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
            } else if !valid_ops.contains(op) {
                errors.push(format!(
                    "node '{name}' (id={id}) has unknown op '{op}'. Run 'apxm ops list' for valid ops."
                ));
            } else {
                // Check required attributes
                let spec = AIS_OPERATIONS.iter().find(|s| s.op_type.to_string() == op);
                if let Some(spec) = spec {
                    let attrs = node
                        .get("attributes")
                        .and_then(serde_json::Value::as_object);
                    for field in spec.fields.iter().filter(|f| f.required) {
                        let has_attr = attrs.is_some_and(|a| a.contains_key(field.name));
                        if !has_attr {
                            errors.push(format!(
                                "node '{name}' (id={id}, op={op}) missing required attribute '{}'",
                                field.name
                            ));
                        }
                    }
                }
            }
        }
    }

    // Edge checks
    if let Some(edges) = raw.get("edges").and_then(serde_json::Value::as_array) {
        for edge in edges {
            let from = edge
                .get("from")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let to = edge
                .get("to")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let dep = edge
                .get("dependency")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Data");

            if from == to {
                errors.push(format!("edge {from}->{to} is a self-loop"));
            }
            if !matches!(dep, "Data" | "Control" | "Effect") {
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

        // DAG cycle check (Kahn's algorithm)
        if !node_ids.is_empty() && !edges.is_empty() {
            let mut in_degree: HashMap<u64, usize> = node_ids.iter().map(|&id| (id, 0)).collect();
            let mut adjacency: HashMap<u64, Vec<u64>> = HashMap::new();

            for edge in edges {
                let from = edge
                    .get("from")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let to = edge
                    .get("to")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
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
    }

    // Parameter checks
    if let Some(params) = raw.get("parameters").and_then(serde_json::Value::as_array) {
        let valid_types: HashSet<&str> = ["str", "int", "float", "bool", "json"]
            .into_iter()
            .collect();
        let mut param_names: HashSet<String> = HashSet::new();
        for param in params {
            let pname = param
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let ptype = param
                .get("type_name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if pname.is_empty() {
                errors.push("parameter with empty name".to_string());
            }
            if !param_names.insert(pname.to_string()) {
                errors.push(format!("duplicate parameter name '{pname}'"));
            }
            if !valid_types.contains(ptype) {
                warnings.push(format!(
                    "parameter '{pname}' has non-standard type_name '{ptype}'"
                ));
            }
        }
    }

    // Also attempt full Rust-side parse+validate for deeper checks (when driver feature available)
    #[cfg(feature = "driver")]
    {
        match apxm_graph::ApxmGraph::from_json(&content) {
            Ok(_) => {}
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
                println!("  {} {}", "\u{26a0}".yellow(), w);
            }
        }
    } else {
        print_status_line(&input.display().to_string(), Status::Error, "invalid");
        for e in &errors {
            println!("  {} {}", "\u{2717}".red(), e);
        }
        for w in &warnings {
            println!("  {} {}", "\u{26a0}".yellow(), w);
        }
        return Err(anyhow::anyhow!("{} error(s) found", errors.len()));
    }

    Ok(())
}

/// Parsed graph topology used by analyze and explain commands.
struct GraphAnalysis<'a> {
    graph_name: &'a str,
    nodes: &'a Vec<serde_json::Value>,
    edge_count: usize,
    node_ids: HashSet<u64>,
    successors: HashMap<u64, Vec<u64>>,
    predecessors: HashMap<u64, Vec<u64>>,
    entry_nodes: Vec<u64>,
    exit_nodes: Vec<u64>,
    phases: Vec<Vec<u64>>,
}

impl<'a> GraphAnalysis<'a> {
    fn from_raw(raw: &'a serde_json::Value) -> Result<Self> {
        let graph_name = raw
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unnamed");
        let nodes = raw
            .get("nodes")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("graph has no nodes array"))?;
        let empty_edges = vec![];
        let edges = raw
            .get("edges")
            .and_then(serde_json::Value::as_array)
            .unwrap_or(&empty_edges);
        let edge_count = edges.len();

        let node_ids: HashSet<u64> = nodes
            .iter()
            .filter_map(|n| n.get("id").and_then(serde_json::Value::as_u64))
            .collect();

        let mut successors: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut predecessors: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut in_degree: HashMap<u64, usize> = node_ids.iter().map(|&id| (id, 0)).collect();

        for edge in edges.iter() {
            let from = edge
                .get("from")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let to = edge
                .get("to")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            successors.entry(from).or_default().push(to);
            predecessors.entry(to).or_default().push(from);
            *in_degree.entry(to).or_insert(0) += 1;
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

        // BFS phase layering
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

        Ok(Self {
            graph_name,
            nodes,
            edge_count,
            node_ids,
            successors,
            predecessors,
            entry_nodes,
            exit_nodes,
            phases,
        })
    }

    fn node_by_id(&self, id: u64) -> Option<&serde_json::Value> {
        self.nodes
            .iter()
            .find(|n| n.get("id").and_then(serde_json::Value::as_u64) == Some(id))
    }

    fn node_op(&self, id: u64) -> &str {
        self.node_by_id(id)
            .and_then(|n| n.get("op").and_then(serde_json::Value::as_str))
            .unwrap_or("?")
    }

    fn node_name(&self, id: u64) -> &str {
        self.node_by_id(id)
            .and_then(|n| n.get("name").and_then(serde_json::Value::as_str))
            .unwrap_or("?")
    }

    fn node_latency_ms(&self, id: u64) -> u64 {
        op_latency_ms(self.node_op(id))
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
    let raw: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Invalid JSON in {}: {e}", input.display()))?;

    let ga = GraphAnalysis::from_raw(&raw)?;
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
            "graph_name": ga.graph_name,
            "node_count": ga.nodes.len(),
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
        print_section_header(&format!("Analysis: {}", ga.graph_name));
        println!(
            "  {} nodes, {} edges, {} phases, max parallelism {}",
            ga.nodes.len(),
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
            println!("  {} Phase {} {}", "\u{25b6}".cyan(), i + 1, tag);
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
            "\u{26a1}".yellow(),
            critical_path.len(),
            critical_ms
        );
        println!(
            "  \u{1f680} Speedup: {:.2}x (sequential {}ms \u{2192} parallel {}ms)",
            speedup, sequential_ms, parallel_ms
        );

        if !suggestions.is_empty() {
            println!();
            println!("  {}", "Suggestions:".bold());
            for s in &suggestions {
                println!("    {} {s}", "\u{2022}".dimmed());
            }
        }
    }

    Ok(())
}

fn explain_command(file: PathBuf, json_output: bool) -> Result<()> {
    let content = std::fs::read_to_string(&file)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", file.display()))?;
    let raw: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Invalid JSON in {}: {e}", file.display()))?;

    let ga = GraphAnalysis::from_raw(&raw)?;
    let critical_ms = ga.parallel_ms();
    let max_parallelism = ga.max_parallelism();
    let depth = ga.phases.len();
    let parallelizable = max_parallelism > 1;

    // Helper: get notable attributes from a node for display
    let notable_attrs = |id: u64| -> Vec<(String, String)> {
        let mut attrs = Vec::new();
        if let Some(node) = ga.node_by_id(id)
            && let Some(a) = node
                .get("attributes")
                .and_then(serde_json::Value::as_object)
        {
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
                if let Some(val) = a.get(key) {
                    let display = match val {
                        serde_json::Value::String(s) => {
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
                        let spec = find_op_spec(op);
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
            "graph_name": ga.graph_name,
            "node_count": ga.nodes.len(),
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
        println!("  {} {}", "Graph:".bold().cyan(), ga.graph_name.bold(),);
        println!(
            "  Nodes: {} | Edges: {} | Depth: {}",
            ga.nodes.len(),
            ga.edge_count,
            depth,
        );
        println!();
        println!("  {}", "Execution Flow:".bold().cyan());
        println!("  {}", "\u{2550}".repeat(15).dimmed());

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
                let spec = find_op_spec(op);
                let cat = spec.map(|s| category_str(s.category)).unwrap_or("unknown");
                let lat_val = ga.node_latency_ms(id);
                let desc = spec.map(|s| s.description).unwrap_or("");

                println!();
                println!(
                    "    {} \"{}\" {} {} ({}, ~{}ms)",
                    format!("[{}]", id).dimmed(),
                    ga.node_name(id).bold(),
                    "\u{2014}".dimmed(),
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
                        "\u{2190}".dimmed(),
                        dep_strs.join(", "),
                    );
                }
                if !feeds.is_empty() {
                    let feed_strs: Vec<String> = feeds.iter().map(|f| f.to_string()).collect();
                    println!(
                        "        {} feeds: [{}]",
                        "\u{2192}".dimmed(),
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
                        "\u{2139}".cyan(),
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
                        println!("  {}", "\u{2500}".repeat(40).dimmed());
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
                println!("  {}", "\u{2500}".repeat(50).dimmed());
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

    // --- LLM Credentials ---
    let (cred_count, cred_providers): (usize, Vec<String>) = match CredentialStore::open() {
        Ok(store) => match store.list() {
            Ok(entries) => {
                let names: Vec<String> = entries.iter().map(|(name, _)| name.clone()).collect();
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
                let backends = cfg.llm_backends.len();
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
            "credentials": {
                "count": cred_count,
                "providers": cred_providers,
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

    // 2. LLM Credentials
    print_section_header("LLM Credentials");
    if cred_count > 0 {
        print_status_line(
            "Credentials",
            Status::Ok,
            &format!(
                "{} provider{} registered",
                cred_count,
                if cred_count == 1 { "" } else { "s" }
            ),
        );
    } else {
        print_status_line("Credentials", Status::Warning, "none registered");
        print_hint("Run `apxm llm add` to configure an LLM provider.");
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

fn print_section_header(title: &str) {
    println!();
    println!("  {}", title.bold().cyan());
    println!("  {}", "\u{2500}".repeat(title.len()).dimmed());
}

fn print_subsection_header(title: &str) {
    println!();
    println!("  {}", title.bold());
}

fn print_hint(message: &str) {
    println!("  {} {}", "\u{2139}".cyan(), message);
}

enum Status {
    Ok,
    Warning,
    Error,
}

fn print_status_line(label: &str, status: Status, value: &str) {
    let (icon, status_str) = match status {
        Status::Ok => ("\u{2713}".green(), "OK".green().bold()),
        Status::Warning => ("!".yellow(), "WARN".yellow().bold()),
        Status::Error => ("\u{2717}".red(), "MISSING".red().bold()),
    };
    println!("  {} {:<14} [{}] {}", icon, label.bold(), status_str, value);
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
        let spec = find_op_spec("INV");
        assert!(spec.is_some(), "INV should be a valid operation");
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

        let inv = find_op_spec("INV").unwrap();
        assert_eq!(inv.category, OperationCategory::Tools);
    }

    // ── GraphAnalysis tests ─────────────────────────────────────────────────

    /// Build a simple test graph JSON: A -> B -> C (sequential pipeline)
    fn pipeline_graph() -> serde_json::Value {
        serde_json::json!({
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
        })
    }

    /// Build a fan-out graph: 1,2,3 (parallel) -> 4 (sync)
    fn fanout_graph() -> serde_json::Value {
        serde_json::json!({
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
        })
    }

    /// Single-node graph (no edges)
    fn single_node_graph() -> serde_json::Value {
        serde_json::json!({
            "name": "single",
            "nodes": [
                {"id": 1, "name": "only", "op": "ASK", "attributes": {"template_str": "hi"}}
            ],
            "edges": [],
            "parameters": [],
            "metadata": {}
        })
    }

    #[test]
    fn graph_analysis_pipeline_basic_properties() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

        assert_eq!(ga.graph_name, "test-pipeline");
        assert_eq!(ga.nodes.len(), 3);
        assert_eq!(ga.edge_count, 2);
        assert_eq!(ga.node_ids.len(), 3);
    }

    #[test]
    fn graph_analysis_pipeline_entry_exit_nodes() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

        // In a pipeline A->B->C, entry is [1] and exit is [3]
        assert_eq!(ga.entry_nodes, vec![1]);
        assert_eq!(ga.exit_nodes, vec![3]);
    }

    #[test]
    fn graph_analysis_pipeline_phases() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

        // Sequential pipeline should have 3 phases, each with 1 node
        assert_eq!(ga.phases.len(), 3);
        assert_eq!(ga.phases[0], vec![1]);
        assert_eq!(ga.phases[1], vec![2]);
        assert_eq!(ga.phases[2], vec![3]);
    }

    #[test]
    fn graph_analysis_pipeline_max_parallelism() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

        // Sequential => max parallelism = 1
        assert_eq!(ga.max_parallelism(), 1);
    }

    #[test]
    fn graph_analysis_pipeline_speedup() {
        let raw = pipeline_graph();
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

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
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

        let (path, ms) = ga.critical_path();
        // Critical path includes all 3 nodes in sequence
        assert_eq!(path.len(), 3);
        assert_eq!(path, vec![1, 2, 3]);
        assert!(ms > 0, "critical path ms should be positive");
    }

    #[test]
    fn graph_analysis_fanout_max_parallelism() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

        // Nodes 1,2,3 are all entry nodes with no predecessors => phase 1 has 3 nodes
        assert_eq!(ga.max_parallelism(), 3);
    }

    #[test]
    fn graph_analysis_fanout_phases() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

        // Phase 1: [1,2,3] parallel, Phase 2: [4] sync
        assert_eq!(ga.phases.len(), 2);
        assert_eq!(ga.phases[0].len(), 3);
        assert_eq!(ga.phases[1], vec![4]);
    }

    #[test]
    fn graph_analysis_fanout_speedup_greater_than_one() {
        let raw = fanout_graph();
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

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
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

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
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

        assert_eq!(ga.graph_name, "single");
        assert_eq!(ga.nodes.len(), 1);
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
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

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
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

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
        let ga = GraphAnalysis::from_raw(&raw).unwrap();

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
        let raw = serde_json::json!({"name": "empty"});
        let result = GraphAnalysis::from_raw(&raw);
        assert!(result.is_err());
    }
}
