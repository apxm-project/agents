# ACPX Absorption: APXM Subsumes the Orchestration Layer

**Date**: March 31, 2026
**Status**: Revised architecture -- ACPX disappears into APXM

---

## The Argument

ACPX should not exist as a separate orchestration layer. APXM already has everything ACPX does, and does it better -- except for one thing: the ACP protocol client that knows how to spawn and talk to Claude Code, Codex, and Gemini.

That one missing piece is ~500-800 lines of Rust implementing the `CapabilityExecutor` trait.

### What APXM Already Has (That ACPX Duplicates Poorly)

| Capability | ACPX | APXM |
|---|---|---|
| **Agent dispatch** | `acp()` node, spawns 1 agent | `DELEGATE` op: full sub-DAG execution with child context, AAM tracking, scoped memory |
| **Inter-agent comms** | None (agents are isolated) | `COMMUNICATE` op: local sub-flow, HTTP remote, or broadcast fan-out in parallel |
| **Agent spawning** | Registry of npx commands | `SPAWN_AGENT` op: registers agent in FlowRegistry with capabilities and goals |
| **Parallel execution** | Sequential only | Work-stealing dataflow scheduler with 4-priority tiers |
| **Conditional routing** | `switch` edges with JSON path | `BRANCH_ON_VALUE`, `SWITCH` ops with full expression evaluation |
| **Tool invocation** | Shell commands only | `INV` op: CapabilitySystem with schema validation, timeouts, interceptors |
| **Sandbox execution** | None | `EXC` op: SandboxRegistry with security profiles |
| **Human-in-the-loop** | `checkpoint()` node | `PAUSE` + `RESUME` ops with checkpoint serialization |
| **Memory** | None | 3-tier: STM (microsecond), LTM (persistent), Episodic (audit trail) |
| **Model routing** | Passthrough string | OperationRoutes + BackendFallback + per-op defaults |
| **Optimization** | None | MLIR compiler: FuseAskOps, CSE, DCE, critical path analysis |
| **Observability** | Trace bundles (NDJSON) | Metrics, events, episodic memory, `--emit-metrics` |
| **Compilation** | Interpreted TypeScript | Compiled to .apxmobj binary artifacts |

### What ACPX Has That APXM Doesn't

**One thing**: An ACP protocol client -- the code that:

1. Spawns `npx @agentclientprotocol/claude-agent-acp@^0.24.2` as a subprocess
2. Sends JSON-RPC messages over stdio (`initialize`, `session/new`, `session/prompt`)
3. Collects the response text
4. Handles permission negotiation

That's it. Everything else is a worse version of what APXM already has.

---

## How APXM Absorbs ACPX

### The Missing Piece: AcpCapability

A new `CapabilityExecutor` implementation that speaks the Agent Client Protocol:

```rust
// New file: apxm-runtime/src/capability/acp.rs (~500-800 lines)

use crate::capability::executor::{CapabilityExecutor, CapabilityMetadata};
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::process::Command;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Registry of known ACP agent commands
pub struct AcpAgentRegistry {
    agents: HashMap<String, AcpAgentConfig>,
}

pub struct AcpAgentConfig {
    pub command: String,          // e.g., "npx"
    pub args: Vec<String>,        // e.g., ["-y", "@agentclientprotocol/claude-agent-acp@^0.24.2"]
    pub env: HashMap<String, String>,
    pub default_model: Option<String>,
    pub timeout_ms: u64,
}

impl Default for AcpAgentRegistry {
    fn default() -> Self {
        let mut agents = HashMap::new();
        agents.insert("claude".into(), AcpAgentConfig {
            command: "npx".into(),
            args: vec!["-y".into(), "@agentclientprotocol/claude-agent-acp@^0.24.2".into()],
            env: HashMap::new(),
            default_model: None,
            timeout_ms: 300_000,
        });
        agents.insert("codex".into(), AcpAgentConfig {
            command: "npx".into(),
            args: vec!["@zed-industries/codex-acp@^0.10.0".into()],
            env: HashMap::new(),
            default_model: None,
            timeout_ms: 300_000,
        });
        agents.insert("gemini".into(), AcpAgentConfig {
            command: "gemini".into(),
            args: vec!["--acp".into()],
            env: HashMap::new(),
            default_model: None,
            timeout_ms: 300_000,
        });
        // ... more agents
        Self { agents }
    }
}

/// ACP capability executor -- spawns an external coding agent via ACP
pub struct AcpCapability {
    registry: AcpAgentRegistry,
    metadata: CapabilityMetadata,
}

#[async_trait]
impl CapabilityExecutor for AcpCapability {
    async fn execute(
        &self,
        args: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError> {
        let profile = args.get("profile")
            .and_then(|v| v.as_string())
            .unwrap_or("claude");
        let prompt = args.get("prompt")
            .and_then(|v| v.as_string())
            .ok_or_else(|| RuntimeError::Capability {
                message: "ACP capability requires 'prompt' argument".into(),
            })?;
        let cwd = args.get("cwd")
            .and_then(|v| v.as_string());

        let config = self.registry.agents.get(profile)
            .ok_or_else(|| RuntimeError::Capability {
                message: format!("Unknown ACP agent profile: {}", profile),
            })?;

        // 1. Spawn agent process
        let mut child = Command::new(&config.command)
            .args(&config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(cwd.unwrap_or("."))
            .envs(&config.env)
            .spawn()?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        // 2. Send JSON-RPC: initialize
        send_jsonrpc(&stdin, "initialize", json!({
            "protocolVersion": "2025-11-05",
            "clientInfo": { "name": "apxm", "version": "0.1.0" }
        })).await?;
        let init_response = read_jsonrpc(&stdout).await?;

        // 3. Send JSON-RPC: session/new
        send_jsonrpc(&stdin, "session/new", json!({})).await?;
        let session = read_jsonrpc(&stdout).await?;
        let session_id = session["result"]["sessionId"].as_str().unwrap();

        // 4. Send JSON-RPC: session/prompt
        send_jsonrpc(&stdin, "session/prompt", json!({
            "sessionId": session_id,
            "prompt": { "type": "text", "text": prompt }
        })).await?;

        // 5. Collect response (streaming events until completion)
        let response = collect_acp_response(&stdout).await?;

        // 6. Cleanup
        child.kill().await.ok();

        Ok(Value::String(response))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}
```

### Registration

The ACP capability registers itself like any other capability:

```rust
// In Runtime::new() or via CLI config
let acp = Arc::new(AcpCapability::new(AcpAgentRegistry::default()));
capability_system.register(acp)?;
```

### Usage in Graphs

Two ways to invoke ACP agents:

**Option A: Via INV operation** (uses existing CapabilitySystem):

```json
{
  "id": 5,
  "name": "review_code",
  "op": "INV",
  "attributes": {
    "capability": "acp",
    "params_json": "{\"profile\": \"claude\", \"prompt\": \"Review this code...\", \"cwd\": \"/repo\"}"
  }
}
```

**Option B: Via DELEGATE operation** (uses FlowRegistry for pre-registered agent flows):

```json
{
  "id": 5,
  "name": "review_code",
  "op": "DELEGATE",
  "attributes": {
    "target_agent": "claude-reviewer",
    "task_spec": "Review the PR diff and report issues"
  }
}
```

**Option C: Direct LLM** (no agent needed -- just call the model):

```json
{
  "id": 5,
  "name": "review_code",
  "op": "ASK",
  "attributes": {
    "template_str": "Review this code: {0}",
    "model_policy": "best"
  }
}
```

---

## The Two-Tier Model

The key architectural insight: **not every LLM task needs a full coding agent**.

```
                    APXM Graph
                        |
           ┌────────────┼────────────┐
           |            |            |
      ┌────v────┐  ┌────v────┐  ┌────v────┐
      │ ASK     │  │ INV     │  │ DELEGATE│
      │ THINK   │  │ (acp)   │  │         │
      │ REASON  │  │         │  │         │
      └────┬────┘  └────┬────┘  └────┬────┘
           |            |            |
    Direct LLM     ACP Agent    Sub-DAG
    (fast, cheap)  (powerful)   (registered)
           |            |            |
      ┌────v────┐  ┌────v────┐  ┌────v────┐
      │ vLLM    │  │ Claude  │  │ Another │
      │ OpenAI  │  │ Code    │  │ APXM    │
      │ Ollama  │  │ Codex   │  │ graph   │
      │         │  │ Gemini  │  │         │
      └─────────┘  └─────────┘  └─────────┘
```

**When to use each tier:**

| Tier | Operation | Use When | Latency | Cost |
|------|-----------|----------|---------|------|
| **Direct LLM** | ASK, THINK, REASON | Analysis, reasoning, generation, classification | ~1-10s | Low |
| **ACP Agent** | INV(acp) | Multi-step coding: edit files, run tests, browse | ~30-300s | High |
| **Sub-DAG** | DELEGATE | Orchestrated multi-agent workflow | Varies | Varies |

**The compiler can decide**: A simple "review this diff" task can go to ASK (direct LLM, 3 seconds). A complex "fix this bug, run tests, iterate until green" task needs INV(acp) (full Claude Code, 5 minutes). The APXM compiler can analyze the task and route accordingly.

---

## What Happens to ACPX Flow Files

Existing `.flow.ts` files translate directly to APXM graphs:

### echo.flow.ts (Before)

```typescript
defineFlow({
  name: "echo-test",
  startAt: "load_input",
  nodes: {
    load_input: compute({ run: ({ input }) => ({ question: input.question }) }),
    ask_codex: acp({ profile: "codex", prompt: ... }),
    ask_claude: acp({ profile: "claude", prompt: ... }),
    compare: compute({ run: ({ outputs }) => ... }),
  },
  edges: [
    { from: "load_input", to: "ask_codex" },
    { from: "ask_codex", to: "ask_claude" },  // SEQUENTIAL!
    { from: "ask_claude", to: "compare" },
  ],
});
```

### Equivalent APXM Graph (After -- now parallel!)

```json
{
  "name": "echo-test",
  "parameters": [{"name": "question", "type_name": "str"}],
  "nodes": [
    {"id": 1, "name": "ask_codex", "op": "INV", "attributes": {
      "capability": "acp",
      "params_json": "{\"profile\": \"codex\", \"prompt\": \"Answer: {0}\"}"
    }},
    {"id": 2, "name": "ask_claude", "op": "INV", "attributes": {
      "capability": "acp",
      "params_json": "{\"profile\": \"claude\", \"prompt\": \"Answer: {0}\"}"
    }},
    {"id": 3, "name": "compare", "op": "ASK", "attributes": {
      "template_str": "Compare these answers:\nCodex: {0}\nClaude: {1}\nAre they the same?",
      "model_policy": "fast"
    }}
  ],
  "edges": [
    {"from": 1, "to": 3, "dependency": "Data"},
    {"from": 2, "to": 3, "dependency": "Data"}
  ]
}
```

**Key difference**: Nodes 1 and 2 have no dependency on each other. APXM's dataflow scheduler runs them **in parallel**. ACPX would run them sequentially.

### review.flow.ts (Before -- 6 nodes, sequential)

```
load_pr -> fetch_diff -> review_code(claude) -> judge_verdict -> post_approval
                                                              -> post_changes
```

### Equivalent APXM Graph (After -- optimized)

```json
{
  "name": "pr-review",
  "parameters": [
    {"name": "repo", "type_name": "str"},
    {"name": "prNumber", "type_name": "int"}
  ],
  "nodes": [
    {"id": 1, "name": "fetch_diff", "op": "EXC", "attributes": {
      "command": "gh pr diff {prNumber} --repo {repo}"
    }},
    {"id": 2, "name": "review_code", "op": "ASK", "attributes": {
      "template_str": "Review this PR diff for correctness, security, and style:\n\n{0}\n\nReturn JSON: {\"verdict\": \"approve\"|\"request_changes\", \"summary\": \"...\", \"comments\": [...]}",
      "model_policy": "best",
      "output_schema": {"type": "object", "properties": {"verdict": {"type": "string"}, "summary": {"type": "string"}}}
    }},
    {"id": 3, "name": "judge", "op": "BRANCH_ON_VALUE", "attributes": {
      "key": "verdict"
    }},
    {"id": 4, "name": "approve", "op": "EXC", "attributes": {
      "command": "gh pr review {prNumber} --repo {repo} --approve --body '{summary}'"
    }},
    {"id": 5, "name": "request_changes", "op": "EXC", "attributes": {
      "command": "gh pr review {prNumber} --repo {repo} --request-changes --body '{summary}'"
    }}
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"},
    {"from": 2, "to": 3, "dependency": "Data"},
    {"from": 3, "to": 4, "dependency": "Data"},
    {"from": 3, "to": 5, "dependency": "Data"}
  ]
}
```

**Key difference**: The review step uses `ASK` (direct LLM call) instead of spawning a full Claude Code agent. For a simple "read diff, give verdict" task, you don't need file editing, terminal access, or multi-turn conversation. A single LLM call is 10x faster and 10x cheaper.

**When you DO need the full agent**: If the review requires the agent to check out the code, run tests, inspect files, then iterate -- use `INV(acp)` to spawn Claude Code with full capabilities.

---

## Revised Architecture

```
                           User / OpenClaw
                                |
                                v
                    ┌───────────────────────┐
                    │   APXM Graph          │
                    │   (authored or        │
                    │    generated)          │
                    └──────────┬────────────┘
                               |
                    ┌──────────v────────────┐
                    │   APXM Compiler       │  MLIR passes:
                    │                       │  - Task complexity analysis
                    │   Decides per-node:   │  - Model affinity
                    │   Direct LLM vs       │  - Parallelism extraction
                    │   Agent dispatch      │  - Speculation insertion
                    └──────────┬────────────┘
                               |
                    ┌──────────v────────────┐
                    │   APXM Runtime        │
                    │   (DataflowScheduler) │
                    │                       │
                    │   ┌─────────────────┐ │
                    │   │ CapabilitySystem │ │
                    │   │                 │ │
                    │   │ ┌─────────────┐ │ │
                    │   │ │ LLM Backend │ │ │  ASK/THINK/REASON
                    │   │ │ (vLLM etc)  │ │ │  Direct inference
                    │   │ └─────────────┘ │ │
                    │   │                 │ │
                    │   │ ┌─────────────┐ │ │
                    │   │ │ ACP Client  │ │ │  INV(acp) / DELEGATE
                    │   │ │ (NEW)       │ │ │  Spawn Claude/Codex/Gemini
                    │   │ └─────────────┘ │ │
                    │   │                 │ │
                    │   │ ┌─────────────┐ │ │
                    │   │ │ Tools/MCP   │ │ │  INV(tool) / EXC
                    │   │ │ (existing)  │ │ │  Functions, sandboxed exec
                    │   │ └─────────────┘ │ │
                    │   └─────────────────┘ │
                    │                       │
                    │   ┌─────────────────┐ │
                    │   │ Memory System   │ │  Context, memoization
                    │   │ STM/LTM/Episodic│ │
                    │   └─────────────────┘ │
                    │                       │
                    │   ┌─────────────────┐ │
                    │   │ Model Router    │ │  Dynamic model selection
                    │   │ (NEW)           │ │
                    │   └─────────────────┘ │
                    └───────────────────────┘
```

**ACPX is gone.** Its role is a single `CapabilityExecutor` implementation inside APXM.

---

## What We Take From ACPX (As a Library)

We don't need ACPX as an orchestrator. But we can learn from or reuse its:

1. **Agent adapter registry** -- the mapping of profile names to spawn commands
2. **ACP protocol implementation** -- the JSON-RPC message format and session lifecycle
3. **Session persistence patterns** -- how to resume agent sessions across invocations

These become implementation details of `AcpCapability`, not a separate system.

---

## Implementation Plan (Replaces Phase 2 from master-plan.md)

### Phase 2 Revised: ACP Client in APXM (Weeks 4-6)

**Week 4: AcpCapability core**

| File | Action | Description |
|------|--------|-------------|
| `apxm-runtime/src/capability/acp.rs` | Create | AcpCapability implementing CapabilityExecutor |
| `apxm-runtime/src/capability/acp/registry.rs` | Create | Agent adapter registry (profile -> spawn command) |
| `apxm-runtime/src/capability/acp/protocol.rs` | Create | JSON-RPC message construction and parsing |
| `apxm-runtime/src/capability/acp/session.rs` | Create | Session lifecycle (create, prompt, cancel) |

**Week 5: Session persistence + model routing**

| File | Action | Description |
|------|--------|-------------|
| `apxm-runtime/src/capability/acp/persistence.rs` | Create | Session state in LTM for resume |
| `apxm-runtime/src/capability/acp/config.rs` | Create | `~/.apxm/agents.toml` config |
| Integration with ModelRouter | Modify | Model policy resolution for ACP agents |

**Week 6: Testing + CLI**

| File | Action | Description |
|------|--------|-------------|
| `apxm-cli/src/commands/agent.rs` | Create | `apxm agent list/test/add` |
| Tests | Create | End-to-end: graph with INV(acp) -> Claude Code |
| `apxm-runtime/src/capability/mod.rs` | Modify | Auto-register AcpCapability |

### Configuration: `~/.apxm/agents.toml`

```toml
[defaults]
permission_mode = "approve-all"
timeout_ms = 300000
session_ttl_s = 300

[[agents]]
profile = "claude"
command = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@^0.24.2"]
default_model = "claude-sonnet-4"

[[agents]]
profile = "codex"
command = "npx"
args = ["@zed-industries/codex-acp@^0.10.0"]

[[agents]]
profile = "gemini"
command = "gemini"
args = ["--acp"]

[[agents]]
profile = "openclaw"
command = "openclaw"
args = ["acp"]

[[agents]]
profile = "custom"
command = "./my-agent"
args = ["--acp", "--stdio"]
env = { MY_API_KEY = "$MY_API_KEY" }
timeout_ms = 600000
```

---

## Process Model Integration

With the APXM process model, ACP agents gain formal lifecycle management:

- **SPAWN_AGENT** with `profile` attribute replaces manual subprocess management
- **COMMUNICATE** with `protocol: "acp"` replaces direct session/prompt calls
- **ProcessTable** provides the unified registry for all live agent processes
- **AgentProcess** encapsulates the session, profile, and state

This completes the absorption: ACPX's orchestration is now expressed as
standard APXM operations (SPAWN_AGENT + COMMUNICATE) rather than as a
separate layer on top.

## Summary

| Before (ACPX separate) | After (APXM absorbs) |
|---|---|
| User writes `.flow.ts` in TypeScript | User writes APXM graph JSON (or generates it) |
| ACPX interprets flow sequentially | APXM compiles and executes in parallel |
| ACPX spawns agents one at a time | APXM's scheduler runs N agents concurrently |
| No optimization | MLIR passes optimize the graph |
| No memory across steps | 3-tier memory with memoization |
| Separate trace system | Unified metrics + episodic memory |
| Node.js 22+ required | Pure Rust runtime |
| Two systems to deploy/monitor | One system |

**Lines of new code needed**: ~800 (AcpCapability + protocol + registry)
**Lines of code eliminated**: The entire ACPX orchestration layer (~30K lines TypeScript)
**Net complexity**: Dramatically simpler
