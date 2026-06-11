//! CLI type definitions for APXM.

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

use apxm_core::types::{MetricsLevel, OptimizationTarget};

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
    /// Compile canonical AIR source to an artifact
    Compile {
        /// Input workflow source (.py frontend, .air, or directory containing one .air)
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
        #[arg(short = 'O', long = "opt-level", default_value = "2")]
        opt_level: u8,
        /// Optimization target: latency, cost, tokens, parallelism, balanced
        #[arg(long, default_value_t = OptimizationTarget::Balanced)]
        target: OptimizationTarget,
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
        /// When set, --opt-level / --target / --warn no longer determine pass
        /// selection. --no-cse-llm and --disable-pass still filter the list.
        #[arg(long = "pass-list", value_name = "A,B,C", value_delimiter = ',')]
        pass_list_override: Option<Vec<String>>,
        /// Embed the given `skill.toml` as an `apxm.skill_manifest.v1` section
        /// in the output artifact. The embedded copy has `artifact_hash`
        /// stripped (it cannot be inside the artifact it hashes); all other
        /// fields are preserved verbatim so the server's
        /// `validate_embedded_manifest_field` round-trip succeeds.
        #[arg(long = "embed-manifest", value_name = "skill.toml")]
        embed_manifest: Option<PathBuf>,
    },
    /// Decompile an artifact back to AIR
    Decompile {
        /// Input artifact file (.apxmobj)
        artifact: PathBuf,
        /// Output AIR file (defaults to stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Compile and execute AIR source through the runtime
    #[command(trailing_var_arg = true)]
    Execute {
        /// Input workflow source (.py frontend, .air, or directory containing one .air)
        input: PathBuf,
        /// Arguments to pass to the entry flow
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
        /// Optimization level (0 = no optimizations, 1-3 = increasing optimization)
        #[arg(short = 'O', long = "opt-level", default_value = "2")]
        opt_level: u8,
        /// Optimization target: latency, cost, tokens, parallelism, balanced
        #[arg(long, default_value_t = OptimizationTarget::Balanced)]
        target: OptimizationTarget,
        /// Emit metrics JSON file with runtime execution statistics
        #[arg(long)]
        emit_metrics: Option<PathBuf>,
        /// Metrics emission tier (basic = aggregates only;
        /// detailed = adds in-flight observers like per-workflow pin-peak polling)
        #[arg(long, default_value_t = MetricsLevel::default())]
        emit_metrics_level: MetricsLevel,
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
        /// Runtime optimization target for artifact-level execution hints:
        /// latency, cost, tokens, parallelism, balanced
        #[arg(long, default_value_t = OptimizationTarget::Balanced)]
        target: OptimizationTarget,
        /// Emit metrics JSON file with runtime execution statistics
        #[arg(long)]
        emit_metrics: Option<PathBuf>,
        /// Metrics emission tier (basic = aggregates only;
        /// detailed = adds in-flight observers like per-workflow pin-peak polling)
        #[arg(long, default_value_t = MetricsLevel::default())]
        emit_metrics_level: MetricsLevel,
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
    /// Inspect or stop APXM job processes
    Process {
        #[command(subcommand)]
        action: ProcessAction,
    },
    /// Manage multi-step workflow files
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },
    /// Manage the MemoCache (response memoization cache)
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Count text with APXM's compiler-side tokenizer
    Tokenize {
        /// Text to tokenize. Use --file for larger inputs.
        text: Option<String>,
        /// Read text from a file instead of the positional argument.
        #[arg(long, conflicts_with = "text")]
        file: Option<PathBuf>,
        /// Model name used to select the tokenizer family.
        #[arg(long)]
        model: Option<String>,
    },
    /// Phase 14.8.F — stream a run's dispatch tree from
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
    /// Phase 14.8.F — inspect, replay, and archive on-disk rollouts.
    Rollout {
        #[command(subcommand)]
        action: RolloutAction,
    },
    /// Start, follow, inspect, or cancel a bounded APXM goal run.
    ///
    /// Sends the task to APXM server, which owns worker admission,
    /// workflow execution, events, cancellation, and sessions.
    Goal(GoalArgs),
    /// Interactive conversational REPL over a running apxm-server.
    ///
    /// Each user message runs one execution of the agent graph, threading a
    /// stable session id (so server-side memory accrues across turns) and a
    /// client-side transcript. Defaults to a built-in single-ASK chat graph;
    /// pass `--air` to drive a custom conversational agent authored in the
    /// Python frontend.
    Chat {
        /// AIR graph driven each turn (path to a `.air` file). Defaults to the
        /// built-in chat graph (a single ASK over a `{conversation}` param).
        #[arg(long)]
        air: Option<std::path::PathBuf>,
        /// apxm-server base URL (default $APXM_SERVER_BASE or
        /// http://127.0.0.1:18800).
        #[arg(long)]
        server: Option<String>,
        /// Reuse/resume a prior conversation by session id instead of minting
        /// a fresh one.
        #[arg(long)]
        session_id: Option<String>,
        /// Capability to admit for write-tool turns (repeatable); forwarded as
        /// ExecuteRequest.admit_capabilities.
        #[arg(long = "admit", value_name = "CAP")]
        admit: Vec<String>,
        /// Skill library / id to import into the agent's visible set (repeatable:
        /// `lib`, `lib::skill`, or `skill`). Activates server-side CALL_SKILL
        /// scoping; shared-tier skills are always visible. Empty = unrestricted.
        #[arg(long = "import", value_name = "LIB")]
        import: Vec<String>,
        /// Enable the agent's web tool group each turn (the runtime runs
        /// independent tool calls in parallel). Ignored when `--air` is set.
        #[arg(long)]
        tools: bool,
        /// Pin each direct-ASK turn to a registered backend by name (as listed by
        /// `GET /v1/models`). Ignored when `--air` or `--agent` is set.
        #[arg(long, value_name = "NAME")]
        backend: Option<String>,
        /// Pin each direct-ASK turn to a specific model id. Ignored when `--air`
        /// or `--agent` is set; use `--agent-model` for ACP agents.
        #[arg(long, value_name = "ID")]
        model: Option<String>,
        /// Run each chat turn through a spawned ACP agent profile, such as `claude`.
        #[arg(long = "agent", value_name = "PROFILE", conflicts_with = "air")]
        agent: Option<String>,
        /// ACP mode for `--agent`, such as `architect`.
        #[arg(long = "agent-mode", value_name = "MODE", requires = "agent")]
        agent_mode: Option<String>,
        /// ACP model request for `--agent`, used when the profile supports model control.
        #[arg(long = "agent-model", value_name = "MODEL", requires = "agent")]
        agent_model: Option<String>,
        /// Render the full per-agent event tree each turn instead of just the
        /// assistant's text.
        #[arg(long)]
        tree: bool,
        /// Subscribe to an apxm-os control-plane event stream at this base URL
        /// (e.g. http://127.0.0.1:9090). Each cue event (process output, file
        /// change, cron, webhook) becomes a turn, so the agent reacts to external
        /// events without a human typing — the REPL stays interactive too.
        #[arg(long = "monitor-url", value_name = "URL")]
        monitor_url: Option<String>,
    },
}

#[derive(Args, Debug, Clone)]
pub struct GoalArgs {
    /// Goal/task for APXM to decompose, plan, and supervise.
    #[arg(value_name = "TASK")]
    pub task: Option<String>,

    /// Print status for an existing goal id.
    #[arg(long, value_name = "GOAL_ID")]
    pub status: Option<String>,

    /// Print retained events for an existing goal id.
    #[arg(long, value_name = "GOAL_ID")]
    pub events: Option<String>,

    /// Cancel an in-flight goal id.
    #[arg(long, value_name = "GOAL_ID")]
    pub cancel: Option<String>,

    /// apxm-server base URL (default $APXM_SERVER_BASE or http://127.0.0.1:18800).
    #[arg(long)]
    pub server: Option<String>,

    /// Reuse a caller-provided goal session id.
    #[arg(long = "session-id", hide = true)]
    pub session_id: Option<String>,

    /// Optional repository, product, or run context passed to workers.
    #[arg(long)]
    pub context: Option<String>,

    /// Optional event payload/reason that triggered this goal pass.
    #[arg(long)]
    pub event: Option<String>,

    /// Optional trigger rule or source for this goal pass.
    #[arg(long)]
    pub trigger: Option<String>,

    /// Pin an explicit worker as ID[:ROLE[:PROFILE]]. Repeat to override auto-planning.
    #[arg(long = "worker", value_name = "ID[:ROLE[:PROFILE]]")]
    pub workers: Vec<String>,

    /// Dependencies for explicit workers as WORKER=DEP1,DEP2. Repeat to shape phases.
    #[arg(long = "depends", value_name = "WORKER=DEP1,DEP2")]
    pub depends: Vec<String>,

    /// Bind unprofiled explicit workers to resolvable ACP profiles selected by APXM.
    #[arg(long = "use-agents")]
    pub use_agents: bool,

    /// ACP profile for the default planner worker.
    #[arg(long = "planner", value_name = "PROFILE")]
    pub planner_profile: Option<String>,

    /// ACP profile for the default executor worker.
    #[arg(long = "executor", value_name = "PROFILE")]
    pub executor_profile: Option<String>,

    /// Optional reviewer/critic profile or ID[:ROLE[:PROFILE]]. Repeat for more reviewers.
    #[arg(long = "critic", value_name = "PROFILE|ID[:ROLE[:PROFILE]]")]
    pub critics: Vec<String>,

    /// Optional reviewer profile or ID[:ROLE[:PROFILE]]. Repeat for more reviewers.
    #[arg(long = "reviewer", value_name = "PROFILE|ID[:ROLE[:PROFILE]]")]
    pub reviewers: Vec<String>,

    /// ACP profile for the default verifier worker.
    #[arg(long = "verifier", value_name = "PROFILE")]
    pub verifier_profile: Option<String>,

    /// ACP profile for the final gate/eval supervisor.
    #[arg(long = "supervisor", value_name = "PROFILE")]
    pub supervisor_profile: Option<String>,

    /// Workspace allocation mode: session, shared, or git_worktree.
    #[arg(long = "workspace", default_value = "session")]
    pub workspace: String,

    /// Repository root for shared or git_worktree workspace modes.
    #[arg(long = "repo-root")]
    pub repo_root: Option<PathBuf>,

    /// Git ref used when --workspace git_worktree is selected.
    #[arg(long = "base-ref", default_value = "HEAD")]
    pub base_ref: String,

    /// Extra capability grant forwarded to APXM admission (repeatable).
    #[arg(long = "admit", value_name = "CAP")]
    pub admit: Vec<String>,

    /// Explicitly grant SPAWN_AGENT. Also auto-granted when profiles are used.
    #[arg(long = "admit-spawn", hide = true)]
    pub admit_spawn: bool,

    /// Skill library / id to import into the goal run's visible set.
    #[arg(long = "import", value_name = "LIB")]
    pub import: Vec<String>,

    /// Materialize and validate the generated workflow bundle without starting it.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Maximum server-owned passes to run until the gate verdict converges.
    /// Defaults to 1 (a single pass).
    #[arg(long = "max-iterations", value_name = "N")]
    pub max_iterations: Option<usize>,

    /// Start the goal but do not follow the goal event stream.
    #[arg(long = "no-follow")]
    pub no_follow: bool,

    /// Event page size while following.
    #[arg(long = "limit", default_value_t = 100, hide = true)]
    pub limit: usize,

    /// Stop following after this many seconds without cancelling the run.
    #[arg(long = "timeout-secs")]
    pub timeout_secs: Option<u64>,
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
        /// Optional directory holding the source skill files
        /// (skill.toml, SKILL.md, skill.air, skill.apxmobj).
        #[arg(long = "skill-dir")]
        skill_dir: Option<PathBuf>,
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
    /// Generate TypeScript types into the GUI frontend
    Typescript {
        /// Output file path for generated TypeScript
        #[arg(long)]
        output: Option<PathBuf>,
        /// Check that the output is up to date without writing it
        #[arg(long)]
        check: bool,
    },
    /// Generate TypeScript event kind constants into the GUI frontend
    EventKinds {
        /// Output file path for generated TypeScript event kinds
        #[arg(long)]
        output: Option<PathBuf>,
        /// Check that the output is up to date without writing it
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
pub enum ProcessAction {
    /// List APXM job processes visible from this worktree
    List {
        /// Include long-running APXM services such as apxm-server and apxm-gui
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
        /// Include long-running APXM services such as apxm-server and apxm-gui
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn workflow_run_parses() {
        let cli = Cli::try_parse_from(["apxm", "workflow", "run", "workflow.apxmw"])
            .expect("workflow run should parse");
        match cli.command {
            Commands::Workflow {
                action:
                    WorkflowAction::Run {
                        file, background, ..
                    },
            } => {
                assert_eq!(file, PathBuf::from("workflow.apxmw"));
                assert!(!background);
            }
            _ => panic!("expected workflow run action"),
        }
    }

    #[test]
    fn goal_authoring_flags_are_visible_in_help() {
        let mut command = Cli::command();
        let goal = command
            .find_subcommand_mut("goal")
            .expect("goal subcommand should exist");
        let mut help = Vec::new();
        goal.write_long_help(&mut help).expect("goal help");
        let help = String::from_utf8(help).expect("utf8 help");
        for flag in [
            "--event",
            "--trigger",
            "--depends",
            "--base-ref",
            "--admit",
            "--import",
            "--dry-run",
            "--timeout-secs",
        ] {
            assert!(help.contains(flag), "missing {flag} in help:\n{help}");
        }
    }

    #[test]
    fn chat_agent_flags_are_visible_in_help() {
        let mut command = Cli::command();
        let chat = command
            .find_subcommand_mut("chat")
            .expect("chat subcommand should exist");
        let mut help = Vec::new();
        chat.write_long_help(&mut help).expect("chat help");
        let help = String::from_utf8(help).expect("utf8 help");
        for flag in ["--agent", "--agent-mode", "--agent-model"] {
            assert!(help.contains(flag), "missing {flag} in help:\n{help}");
        }
    }

    #[test]
    fn chat_parses_direct_ask_routing() {
        let cli = Cli::try_parse_from([
            "apxm",
            "chat",
            "--backend",
            "local",
            "--model",
            "cheap-model",
        ])
        .expect("direct chat routing should parse");

        match cli.command {
            Commands::Chat {
                backend,
                model,
                agent,
                agent_model,
                ..
            } => {
                assert_eq!(backend.as_deref(), Some("local"));
                assert_eq!(model.as_deref(), Some("cheap-model"));
                assert!(agent.is_none());
                assert!(agent_model.is_none());
            }
            _ => panic!("expected chat command"),
        }
    }

    #[test]
    fn chat_parses_acp_agent_routing() {
        let cli = Cli::try_parse_from([
            "apxm",
            "chat",
            "--agent",
            "claude",
            "--agent-mode",
            "architect",
            "--agent-model",
            "claude-3-5-haiku-latest",
        ])
        .expect("ACP agent chat routing should parse");

        match cli.command {
            Commands::Chat {
                backend,
                model,
                agent,
                agent_mode,
                agent_model,
                ..
            } => {
                assert!(backend.is_none());
                assert!(model.is_none());
                assert_eq!(agent.as_deref(), Some("claude"));
                assert_eq!(agent_mode.as_deref(), Some("architect"));
                assert_eq!(agent_model.as_deref(), Some("claude-3-5-haiku-latest"));
            }
            _ => panic!("expected chat command"),
        }
    }

    #[test]
    fn chat_rejects_incoherent_agent_routing() {
        assert!(
            Cli::try_parse_from(["apxm", "chat", "--air", "chat.air", "--agent", "claude"])
                .is_err()
        );
        assert!(
            Cli::try_parse_from(["apxm", "chat", "--agent-model", "claude-3-5-haiku-latest"])
                .is_err()
        );
    }
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
        /// backend. Requires a capable backend (bubblewrap); spawning fails
        /// closed if none is available.
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
