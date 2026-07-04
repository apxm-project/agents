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
    /// Compile an agent package's Python entry (with `@hook`/`@tool`
    /// registrations) to host-loop AIR on stdout. This is the cross-repo
    /// process contract Server invokes as a subprocess instead of reaching
    /// into this repo's Python frontend directly — see
    /// `commands::compile::compile_service_command` for the exact I/O
    /// contract (stdout carries ONLY the emitted AIR; errors + logs go to
    /// stderr; nonzero exit on failure).
    CompileService {
        /// Package directory (contains pack.toml, agent.toml, python/<entry>)
        package: PathBuf,
        /// Override the python/-relative entry (default: agent.toml's `entry`)
        #[arg(long)]
        entry: Option<String>,
        /// Pass the "host" loop override (the same override Studio's
        /// host-controlled chat surface used to pass) instead of the
        /// manifest's declared [runtime].loop.
        #[arg(long)]
        host_loop: bool,
        /// Enable optional web-tools registration for entries that gate on it.
        #[arg(long)]
        web_tools: bool,
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
    /// Scaffold, lint, build, and install agent packages (AGT-1 folder format)
    Package {
        #[command(subcommand)]
        action: PackageAction,
    },
    /// Scaffold, lint, and install organization packages (ORG-1 folder format)
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
        /// Runtime-minted capability grant id for write-tool turns (repeatable).
        #[arg(long = "capability-grant-id", value_name = "GRANT_ID")]
        capability_grant_ids: Vec<String>,
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
        /// Bound the conversation to at most N substantive turns. A stdin turn at
        /// the cap is soft-blocked (warns; `/continue` extends by N); an
        /// event-driven turn at the cap is dropped. Clamped to the operator
        /// ceiling $APXM_CHAT_MAX_TURNS_CEILING when set.
        #[arg(long = "max-turns", value_name = "N")]
        max_turns: Option<usize>,
        /// Bound event-driven (monitor cue) turns to at most N; events past the
        /// cap are dropped while stdin stays interactive. Clamped to
        /// $APXM_CHAT_MAX_EVENTS_CEILING when set.
        #[arg(long = "max-events", value_name = "N")]
        max_events: Option<usize>,
        /// Per-tool call budget for one turn's whole execution tree, as `CAP=N`
        /// (repeatable). Enforced by the runtime's trusted invoke seam; a
        /// spawned-agent fan-out cannot exceed it. Each N is clamped to the
        /// operator ceiling $APXM_TOOL_CALL_BUDGET_CEILING when set.
        #[arg(long = "tool-budget", value_name = "CAP=N")]
        tool_budget: Vec<String>,
        /// Per-tool call cap for the whole conversation, as `CAP=N` (repeatable).
        /// Tracked across turns by the host; the per-turn budget sent to the
        /// runtime is the lower of this remaining cap and any --tool-budget.
        #[arg(long = "tool-cap", value_name = "CAP=N")]
        tool_cap: Vec<String>,
        /// Bind a tool to an apxm-auth connection for its auth token, as
        /// `CAP=CONNECTION_ID` (repeatable). The server resolves the token
        /// (scoped to --owner) and the runtime injects it at the tool's invoke
        /// seam; the secret never enters the AIR or the prompt.
        #[arg(long = "tool-auth", value_name = "CAP=CONNECTION_ID")]
        tool_auth: Vec<String>,
        /// Tenant/owner scope for --tool-auth credential resolution.
        #[arg(long = "owner", value_name = "OWNER")]
        owner: Option<String>,
        /// Let the agent create and run workflows by exposing authoring tools.
        /// Ignored when `--air`/`--agent` set.
        #[arg(long = "author")]
        author: bool,
    },
}

#[derive(Args, Debug, Clone)]
pub struct GoalArgs {
    /// Goal/task for APXM to decompose and supervise.
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

    /// Runtime-minted capability grant id forwarded to APXM (repeatable).
    #[arg(long = "capability-grant-id", value_name = "GRANT_ID")]
    pub capability_grant_ids: Vec<String>,

    /// Mint a runtime grant for the SPAWN_AGENT tool binding (via --capability-grant-id or auto when profiles are used).
    #[arg(long = "delegate-spawn", hide = true)]
    pub delegate_spawn: bool,

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
    /// Run the OBS-2 retention/compaction pass: archive expired rollout
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
    /// Generate TypeScript event kind constants into the Studio frontend
    EventKinds {
        /// Output file path for generated TypeScript event kinds
        #[arg(long)]
        output: Option<PathBuf>,
        /// Check that the output is up to date without writing it
        #[arg(long)]
        check: bool,
    },
    /// Generate the op-spec.v1 AIS operation catalog + vectors fixture (TSF-1)
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
    /// Show accumulated op-usage counts from real executions (RT-9)
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
        /// Model supports extended thinking
        #[arg(long)]
        supports_thinking: bool,
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
pub enum PackageAction {
    /// Scaffold a new agent-package folder tree (apxm.agent-package.v1).
    New {
        /// Package id (also used as the pack_id / agent id).
        id: String,
        /// Destination directory (default: ./packages/<id>).
        #[arg(long)]
        path: Option<PathBuf>,
        /// Display name for the agent (default: derived from the id).
        #[arg(long)]
        display_name: Option<String>,
    },
    /// Validate a package folder against the agent-package.v1 contract and
    /// check capability-set agreement across agent.toml/capabilities.toml/skills.
    Lint {
        /// Package directory to validate (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// An org package directory whose capabilities/{capabilities,
        /// permissions}.toml are this package's org-global capability set
        /// (AGT-5): a skill invoking one of these is not flagged as
        /// undeclared even though the package itself never joins it.
        #[arg(long)]
        org: Option<PathBuf>,
    },
    /// Compile the package's skills and (re)compute the pack integrity hash
    /// chain, writing the result into pack.toml's [integrity] table.
    Build {
        /// Package directory to build (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Install a built package to `APXM_HOME/packages/<id>/`.
    Install {
        /// Package directory to install (default: current directory).
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
    /// best-effort mirroring to `$APXM_WORKSPACE_ROOT/integrations/<id>/`
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
