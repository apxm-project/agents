# Architecture

## System Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        TRIGGER LAYER (OpenClaw)                         │
│                                                                         │
│  GitHub Webhook ──┐                                                     │
│  Slack Message ───┤──> OpenClaw Gateway ──> flow-dispatcher skill       │
│  CLI / Cron ──────┘         │                     │                     │
│                             │              ┌──────┴──────┐              │
│                             │              │ Spawn N     │              │
│                             │              │ parallel    │              │
│                             │              │ flows       │              │
│                             │              └──────┬──────┘              │
│                             │                     │                     │
│                    acpx flow run -s pr-101 &       │                    │
│                    acpx flow run -s pr-102 &       │                    │
│                    acpx flow run -s pr-103 &       │                    │
└────────────────────────────────┬────────────────────────────────────────┘
                                 │
                                 v
┌─────────────────────────────────────────────────────────────────────────┐
│                     FLOW EXECUTION LAYER (acpx)                         │
│                                                                         │
│  review.flow.ts / triage.flow.ts / custom.flow.ts                       │
│                                                                         │
│  ┌──────────┐   ┌──────────┐   ┌──────────────┐   ┌──────────┐        │
│  │ COMPUTE  │──>│  ACTION   │──>│   ACP        │──>│ COMPUTE  │        │
│  │ load_pr  │   │ fetch_diff│   │ review_code  │   │ route    │        │
│  └──────────┘   └──────────┘   └──────┬───────┘   └────┬─────┘        │
│                                       │                 │              │
│                          Spawns agent │        ┌────────┴────────┐     │
│                          via ACP      │        v                 v     │
│                                       │  ┌──────────┐     ┌──────────┐│
│                                       │  │  ACTION   │     │  ACTION  ││
│                                       │  │ approve   │     │ request  ││
│                                       │  │ PR        │     │ changes  ││
│                                       │  └──────────┘     └──────────┘│
│                                       │                                │
│  Trace bundles: ~/.acpx/flows/runs/   │   Replay viewer: :4173        │
└───────────────────────────────────────┼────────────────────────────────┘
                                        │
                                        │ ACP (JSON-RPC over stdio)
                                        │
┌───────────────────────────────────────┼────────────────────────────────┐
│                          AGENT LAYER                                    │
│                                                                         │
│  Each ACP node spawns ONE agent instance.                               │
│  Each instance gets TWO layers of context:                              │
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ LAYER 1: Folder Discovery (automatic, from cwd)                │    │
│  │                                                                 │    │
│  │  Agent reads from project root:                                 │    │
│  │    Claude Code  ->  CLAUDE.md + .claude/skills/ + .claude/rules/│    │
│  │    Codex        ->  AGENTS.md + .agents/skills/ (walk-up)       │    │
│  │    Cline        ->  .cline/ + .agents/skills/                   │    │
│  │                                                                 │    │
│  │  = Project knowledge: build system, conventions, architecture   │    │
│  │  = Same for ALL instances in the same project                   │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ LAYER 2: Prompt Embedding (flow-controlled, per ACP node)      │    │
│  │                                                                 │    │
│  │  The flow reads .agents/skills/*/SKILL.md and injects           │    │
│  │  selected skills directly into each ACP node's prompt:          │    │
│  │                                                                 │    │
│  │  extract_intent  ->  embedSkills(["review"])                    │    │
│  │  test_changes    ->  embedSkills(["build", "test"])             │    │
│  │  final_review    ->  embedSkills(["review"])                    │    │
│  │                                                                 │    │
│  │  = Role-specific expertise: what THIS step needs to know        │    │
│  │  = Different per node, same agent binary                        │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                         │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐   │
│  │ Claude Code │  │   Codex     │  │   Cline     │  │  Gemini     │   │
│  │             │  │             │  │             │  │             │   │
│  │ profile:    │  │ profile:    │  │ profile:    │  │ profile:    │   │
│  │  "claude"   │  │  "codex"    │  │  "cline"    │  │  "gemini"   │   │
│  │             │  │             │  │             │  │             │   │
│  │ Adapter:    │  │ Adapter:    │  │ Adapter:    │  │ Adapter:    │   │
│  │ claude-     │  │ codex-acp   │  │ cline --acp │  │ built-in    │   │
│  │ agent-acp   │  │ (Rust)      │  │ (native)    │  │             │   │
│  │ (TS, npx)   │  │             │  │             │  │             │   │
│  │             │  │             │  │             │  │             │   │
│  │ Tools:      │  │ Tools:      │  │ Tools:      │  │ Tools:      │   │
│  │ Read, Edit, │  │ Read, Write,│  │ Read, Edit, │  │ Read, Write │   │
│  │ Bash, Glob, │  │ Bash, Glob  │  │ Bash, MCP,  │  │             │   │
│  │ Grep, MCP   │  │             │  │ Browser     │  │             │   │
│  └─────────────┘  └─────────────┘  └─────────────┘  └─────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────┐
│                  CONFIG GENERATION LAYER (dekk agents)                   │
│                                                                         │
│  Source of Truth:  .agents/                                             │
│  ├── project.md              <- Project knowledge                      │
│  ├── skills/                                                            │
│  │   ├── build/SKILL.md      <- How to build the project               │
│  │   ├── test/SKILL.md       <- How to run tests                       │
│  │   ├── review/SKILL.md     <- Code review guidelines                 │
│  │   └── debug/SKILL.md      <- Debugging procedures                   │
│  └── rules/                                                             │
│      ├── tests.md            <- Rules for tests/**                     │
│      └── api.md              <- Rules for src/api/**                   │
│                                                                         │
│  dekk agents generate  ─────────────────────────────────────────┐      │
│                                                                  │      │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐   │      │
│  │ CLAUDE.md  │ │ AGENTS.md  │ │.cursorrules│ │ .github/   │   │      │
│  │ .claude/   │ │            │ │            │ │ copilot-   │   │      │
│  │  skills/   │ │            │ │            │ │ instruct.. │   │      │
│  │  rules/    │ │            │ │            │ │ instruct./ │   │      │
│  └─────┬──────┘ └─────┬──────┘ └─────┬──────┘ └─────┬──────┘   │      │
│        │              │              │              │           │      │
│   Claude Code     Codex          Cursor         Copilot        │      │
│   auto-reads      auto-reads     auto-reads     auto-reads     │      │
│                                                                  │      │
│  dekk agents flow review  ───>  flows/review.flow.ts            │      │
│  dekk agents flow triage  ───>  flows/triage.flow.ts            │      │
│  dekk agents install      ───>  ~/.codex/skills/                │      │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Layer Details

### 1. Trigger Layer — OpenClaw

[OpenClaw](https://github.com/openclaw/openclaw) is a messaging-centric AI
gateway connecting to 23+ channels (Slack, GitHub, WhatsApp, Telegram, etc.).
It is NOT a coding agent — it is the **trigger and dispatch** layer.

**Role**: Convert external events into `acpx flow run` commands.

```
GitHub PR #42 opened
  -> OpenClaw receives webhook
  -> Activates "flow-dispatcher" skill
  -> Runs: acpx flow run triage.flow.ts -s pr-42 --input-json '{...}' --approve-all &
  -> Flow runs in background with named session "pr-42"
```

**Parallel execution**: Each named session (`-s pr-42`, `-s pr-43`) gets its
own agent process. OpenClaw can spawn hundreds of concurrent flows.

**Queue owner model**: First process targeting a session becomes the queue
owner, spawns the agent subprocess. Subsequent processes submit prompts via
Unix domain socket IPC. Idle TTL: 300s (override with `--ttl 0` for indefinite).

### 2. Flow Execution Layer — acpx

[acpx](https://github.com/openclaw/acpx) is a headless CLI for the Agent
Client Protocol (ACP). Its flow runtime executes TypeScript workflow modules
(`defineFlow()`) with four node types:

| Node Type | Purpose | Example |
|-----------|---------|---------|
| `compute` | Synchronous local logic | Parse input, route decisions |
| `action`/`shell` | Shell commands, async operations | `gh pr diff`, `gh pr review` |
| `acp` | Delegate to a coding agent | Code review, test verification |
| `checkpoint` | Pause for human input | Approval gates |

**Edge types**:
- Linear: `{ from: "a", to: "b" }`
- Conditional: `{ from: "a", switch: { on: "$output.route", cases: { x: "b", y: "c" } } }`

**Constraints**:
- Sequential execution (one node at a time)
- Max one outgoing edge per node
- Flow files must be TypeScript (`.flow.ts`)
- Requires Node.js >= 22

**Session persistence**: A single ACP session persists across all `acp` nodes
that share a `session.handle`. If the ACP connection dies, the runtime
reconnects and loads the same session via `session/load`.

**Observability**: Every step emits structured JSON. Trace bundles are saved
at `~/.acpx/flows/runs/<runId>/` for replay and debugging.

### 3. Agent Layer — Claude Code, Codex, Cline, etc.

Each `acp` node in a flow spawns a coding agent via the Agent Client Protocol
(JSON-RPC 2.0 over stdio). The agent type is selected by the `profile` field:

```typescript
review_code: acp({
  profile: "claude",                    // Which agent to spawn
  session: { handle: "review" },        // Persistent session
  cwd: ({ outputs }) => outputs.root,   // Working directory
  prompt: ({ outputs }) => "...",       // What to do
  parse: (text) => JSON.parse(text),    // Extract structured output
})
```

**Agent adapters** (how acpx spawns each agent):

| Agent | Adapter | Command |
|-------|---------|---------|
| Codex | codex-acp (Rust) | `npx @zed-industries/codex-acp@^0.10.0` |
| Claude Code | claude-agent-acp (TS) | `npx @agentclientprotocol/claude-agent-acp@^0.24.2` |
| Cline | Native | `cline --acp` |
| OpenClaw | Native | `openclaw acp` |
| Gemini | Built-in | Auto-download via npx |

**Each agent instance has its own**:
- **Tools**: Determined by the agent binary (Claude has Read/Edit/Bash/Grep/Glob/MCP; Codex has Read/Write/Bash/Glob; Cline adds Browser/MCP)
- **Session**: Isolated or shared via `session.handle`
- **Working directory**: Set by the flow's `cwd` field
- **Permission mode**: `--approve-all` for headless execution

**Each agent does NOT have**:
- Direct knowledge of the flow graph or other nodes
- Access to other agents' sessions
- Ability to spawn sub-flows (that's the flow runtime's job)

### 4. Config Generation Layer — dekk agents

[dekk](https://github.com/randreshg/dekk) generates agent configs from a
single source of truth (`.agents/` directory):

```bash
dekk agents init        # Auto-scaffold .agents/ from project detection
dekk agents generate    # Generate all agent configs
dekk agents install     # Install skills to ~/.codex/skills/
dekk agents status      # Show sync state
dekk agents list        # List available skills
dekk agents flow review # Generate starter flow template
```

**Write once, read everywhere**: One `.agents/project.md` becomes
`CLAUDE.md` + `AGENTS.md` + `.cursorrules` + `.github/copilot-instructions.md`.
Skills in `.agents/skills/` are synced to `.claude/skills/` and `~/.codex/skills/`.

---

## Why dekk — Commands Become Skills

dekk is a Python CLI framework (built on [Typer](https://typer.tiangolo.com/))
that any project can adopt as its terminal interface. The key insight is:
**every CLI command a developer types is knowledge an agent needs too.**

```
┌──────────────────────────────────────────────────────────────────────────┐
│  COMMANDS → SKILLS PIPELINE                                             │
│                                                                          │
│  Developer defines commands          dekk agents init reads them         │
│  ─────────────────────────          ─────────────────────────────        │
│                                                                          │
│  Mode A: dekk-based CLI (Python)     Introspects @app.command()         │
│  ┌──────────────────────────┐        with agent_skill=True              │
│  │ @app.command(             │                                           │
│  │   agent_skill=True        │ ──────> .agents/skills/build/SKILL.md    │
│  │ )                         │         ---                               │
│  │ def build():              │         name: build                       │
│  │   """Build the project""" │         description: Build the project    │
│  │   subprocess.run(["make"])│         ---                               │
│  └──────────────────────────┘         Run: `carts build`                │
│                                                                          │
│  Mode B: .dekk.toml (any project)    Reads [commands] section           │
│  ┌──────────────────────────┐                                           │
│  │ [commands]                │                                           │
│  │ build = {                 │ ──────> .agents/skills/build/SKILL.md    │
│  │   run = "cmake -B build", │         ---                               │
│  │   description = "Build"   │         name: build                       │
│  │ }                         │         description: Build                │
│  └──────────────────────────┘         ---                               │
│                                        Run: `cmake -B build`            │
│                                                                          │
│  Both produce the SAME output: .agents/skills/<cmd>/SKILL.md            │
│  Then: dekk agents generate → CLAUDE.md + AGENTS.md + .cursorrules + …  │
└──────────────────────────────────────────────────────────────────────────┘
```

### Mode A: dekk-Based CLIs

Projects with a Python CLI built on dekk mark commands with `agent_skill=True`:

```python
from dekk import Typer
app = Typer(name="carts")

@app.command(agent_skill=True)
def build():
    """Build the CARTS compiler from source."""
    subprocess.run(["cmake", "-B", "build", "-G", "Ninja"])
    subprocess.run(["cmake", "--build", "build"])

@app.command(agent_skill=True)
def test():
    """Run the CARTS test suite."""
    subprocess.run(["ctest", "--test-dir", "build"])

@app.command()          # no agent_skill → NOT a skill
def version():
    """Print version."""
    print("1.0.0")
```

When `carts agents init` runs, it introspects the parent app, finds commands
tagged with `agent_skill=True`, and generates SKILL.md templates. The
developer's CLI vocabulary (`carts build`, `carts test`) becomes the agent's
vocabulary — automatically.

The `create_agents_app()` factory makes this a one-liner:

```python
from dekk.agents import create_agents_app

# Any dekk-based CLI gets agents commands for free
agents_app = create_agents_app(source_dir=".carts", parent_app=app)
app.add_typer(agents_app, name="agents")
# Now available: carts agents init / generate / install / status / list / flow
```

### Mode B: Plain Projects

Projects without a Python CLI declare commands in `.dekk.toml`:

```toml
[project]
name = "my-compiler"

[commands]
build = { run = "cmake -B build -G Ninja && cmake --build build", description = "Build from source" }
test  = { run = "ctest --test-dir build", description = "Run test suite" }
lint  = { run = "clang-format --dry-run src/**/*.cpp", description = "Check code style" }
```

`dekk agents init` reads `[commands]` and generates the same SKILL.md templates.
No Python code required — any C++, Rust, JavaScript, or Go project can adopt
this pattern by adding a `.dekk.toml` file.

### Why This Matters

Without the commands→skills pipeline, making a project AI-friendly requires
writing duplicate instructions: once for the human (README, Makefile, scripts)
and once for the agent (SKILL.md, AGENTS.md, CLAUDE.md). With dekk:

1. **Define once** — commands exist for the developer (CLI or TOML)
2. **Auto-generate** — `dekk agents init` turns them into SKILL.md templates
3. **Customize** — developer adds troubleshooting tips, examples, gotchas
4. **Propagate** — `dekk agents generate` syncs to all agent config formats
5. **Embed** — acpx flows select which skills each agent step receives

The same `build` command that a developer types in their terminal becomes the
skill that an AI agent reads before building the project in an automated
workflow. Zero duplication, one source of truth.

---

## Two-Layer Skill Injection

This is the core design insight. Every agent needs two kinds of knowledge:

### Layer 1: Folder Discovery (project knowledge)

Agents automatically read instruction files from their working directory:

| Agent | Auto-reads | Walk-up? |
|-------|-----------|----------|
| Claude Code | `CLAUDE.md`, `.claude/skills/`, `.claude/rules/` | No — flat |
| Codex | `AGENTS.md`, `.agents/skills/` | Yes — cumulative |
| Cline | `.cline/`, `.agents/skills/` | No — flat |

This provides **project-level context**: build system, coding conventions,
directory structure, test framework. It's the same for every agent instance
working in the same project.

`dekk agents generate` ensures these files exist and stay in sync.

### Layer 2: Prompt Embedding (role-specific skills)

The flow reads `.agents/skills/*/SKILL.md` files and injects their content
directly into each ACP node's prompt text:

```typescript
// The flow controls which skills each step gets
test_changes: acp({
  prompt: ({ outputs }) => {
    const skills = embedSkills(outputs.projectRoot, ["build", "test"]);
    return [skills, "", "## Task: Verify Changes", "..."].join("\n");
  },
})
```

This provides **role-specific expertise**: the review step gets review
guidelines, the build step gets build instructions, the test step gets test
procedures. Different skills per node, same agent binary.

### Why two layers?

| Concern | Layer 1 (folder) | Layer 2 (prompt) |
|---------|-----------------|-----------------|
| What | Project knowledge | Role expertise |
| Who controls | Agent binary (auto) | Flow definition |
| Same across nodes? | Yes | No |
| Cost | ~0 tokens (lazy load) | Full SKILL.md in context |
| Works for | All agents equally | All agents equally |

---

## Flow Catalog

### echo.flow.ts — Agent Test (4 nodes)

```
load_input -> ask_codex -> ask_claude -> compare
```

Spawns both Codex and Claude Code to answer the same question. Compares
answers. Used for verifying agent connectivity.

### review.flow.ts — PR Code Review (6 nodes)

```
load_pr -> fetch_diff -> review_code -> judge_verdict -+-> post_approval
                                                       +-> post_changes
```

Fetches PR diff via `gh`, spawns an agent to review it, routes to either
approve or request changes based on the verdict.

### triage.flow.ts — PR Triage Pipeline (9 nodes)

```
load_pr -> fetch_context -> fetch_diff -> extract_intent -> classify -+-> test_changes -> final_review -> post_result
                                                                      +-> comment_and_close
```

Multi-step pipeline with 3 ACP calls sharing a persistent session:
1. **extract_intent** — Classify the PR (bug fix, feature, docs, low quality)
2. **test_changes** — Build and test the project, verify changes match intent
3. **final_review** — Final verdict based on verification results

Low-quality PRs are routed to `comment_and_close` instead of verification.

---

## Data Flow

```
Input JSON                 Flow nodes transform data              Output
{"repo":"o/r",     ->  load_pr -> fetch_diff -> review_code ->  {"verdict":"approve",
 "prNumber":42}                                                   "summary":"LGTM",
                                                                  "comments":["..."]}
```

Each node's output is stored in `outputs.<nodeId>` and accessible by
downstream nodes. The flow runtime manages the execution order based on
edges.

### Trace Bundle

Every run produces a trace bundle at `~/.acpx/flows/runs/<runId>/`:

```
<runId>/
├── manifest.json       # Run metadata, status, timing
├── flow.json           # Complete flow definition
├── trace.ndjson        # Raw trace events
├── projections/
│   ├── run.json        # Run-level summary
│   ├── live.json       # Live status (for dashboards)
│   └── steps.json      # Step-by-step results with outputs
├── sessions/
│   └── <sessionId>/
│       ├── binding.json    # ACP session binding
│       ├── record.json     # Session state
│       └── events.ndjson   # Raw ACP events
└── artifacts/
    └── sha256-*.txt    # Prompt/response content by hash
```

The replay viewer (`acpx/examples/flows/replay-viewer/`) reads these
bundles and renders them as interactive DAG visualizations.

---

## End-to-End Pipeline

### Manual (today)

```bash
cd my-project
dekk agents init                    # 1. Scaffold .agents/
dekk agents generate                # 2. Generate CLAUDE.md, AGENTS.md, etc.
dekk agents flow review             # 3. Generate flows/review.flow.ts
cd flows && npm install             # 4. Install deps
acpx --approve-all flow run \       # 5. Run the flow
  ./review.flow.ts \
  --input-json '{"repo":"org/repo","prNumber":42}'
```

### Automated (with OpenClaw — Phase 5)

```bash
# GitHub webhook -> OpenClaw -> flow-dispatcher skill -> acpx

# OpenClaw skill spawns flows as background processes:
acpx flow run triage.flow.ts -s pr-101 --input-json '{"prNumber":101}' --approve-all &
acpx flow run triage.flow.ts -s pr-102 --input-json '{"prNumber":102}' --approve-all &
acpx flow run triage.flow.ts -s pr-103 --input-json '{"prNumber":103}' --approve-all &

# Each flow has its own named session — true parallelism
# Results posted back to GitHub automatically
```

### At Scale (Phase 6)

```
Morning summary:
  - 47 PRs processed overnight
  - 31 approved automatically
  - 12 received review comments
  - 4 escalated for human judgment

Dashboard reads ~/.acpx/flows/runs/ projections:
  pr-101: approve  (3m 22s) — no issues
  pr-102: changes  (8m 14s) — security issue found
  pr-103: approve  (2m 45s) — docs-only change
  pr-104: escalate (12m 01s) — architectural concern
```
