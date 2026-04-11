# OpenClaw Architecture: Standalone Agent on APXM Infrastructure

> **Status**: Design document
> **Date**: 2026-04-11
> **Scope**: OpenClaw as a standalone ACP agent binary + GUI integration

---

## 1. Core Principle

**APXM is the infrastructure. OpenClaw is an agent built on it.**

APXM provides the program execution model — compiler, runtime, scheduler, backends, session tracking, and the Agent Client Protocol (ACP). OpenClaw is an APXM-aware coding agent that uses these facilities, the same way Claude Code uses git and npm as external tools.

OpenClaw is NOT embedded in the GUI. It is a separate binary that speaks ACP. The GUI spawns it, bridges its stdio to browser SSE, and provides the frontend. When APXM workflows need an agent, they spawn the same binary via `SPAWN_AGENT`.

```
                    ~/.apxm/config.toml
                   (single source of truth)
                            |
              +-------------+-------------+
              |             |             |
         apxm CLI      apxm-gui       openclaw
        (compiler,    (HTTP server,   (ACP agent,
         runtime,      browser UI,    LLM + tools,
         scheduler)    ACP bridge)    APXM-aware)
              |             |             |
         Same backends, same models, same API keys
```

---

## 2. What OpenClaw Is

A Rust binary with three responsibilities:

### 2.1 ACP Protocol Server

Implements JSON-RPC 2.0 over NDJson on stdin/stdout. Handles the standard ACP handshake and prompt loop:

```
Client → Agent: initialize (protocolVersion, clientCapabilities, clientInfo)
Agent → Client: initialize response (agentCapabilities, authMethods)

Client → Agent: authenticate (methodId, credential)     [if authMethods declared]
Agent → Client: authenticate response

Client → Agent: session/new (cwd, mcpServers)
Agent → Client: session/new response (sessionId)

Client → Agent: session/prompt (sessionId, prompt)
Agent → Client: session/update notifications (streaming text chunks)
Agent → Client: session/prompt response (stopReason, model)
```

The agent also makes **reverse requests** to the client for file I/O and terminal access:

```
Agent → Client: fs/readTextFile (path)
Client → Agent: response (content)

Agent → Client: terminal/create (command, args, cwd)
Client → Agent: response (terminalId)

Agent → Client: terminal/output (terminalId)
Client → Agent: response (output, exitStatus)
```

### 2.2 LLM Client (Zero Duplicate Registration)

OpenClaw has **no separate configuration**. It reads `~/.apxm/config.toml` — the exact same file used by `apxm compile`, `apxm execute`, `apxm-gui`, and `apxm-server`. You register backends and models **once**, and every component picks them up.

```
~/.apxm/config.toml (SINGLE SOURCE OF TRUTH)
│
├── [[backends]]  name="amd-gateway"  endpoint="https://llm-api.amd.com/api"
│   ├── models: claude-sonnet-4-5, claude-haiku-4-5, gemini-2.5-pro, gpt-4.1, ...
│   └── headers: { Ocp-Apim-Subscription-Key: "...", user: "env:USER" }
│
├── [chat]  default_model="claude-sonnet-4-5@20250929"
│   └── routing: ASK→haiku, THINK→sonnet, VERIFY→o4-mini, ...
│
└── Read by:
    ├── apxm compile/execute  → LLMRegistry → graph node execution
    ├── apxm-server           → /v1/generate endpoints
    ├── apxm-gui chat proxy   → load_backend_config() → reqwest
    └── openclaw              → config.rs → reqwest (same endpoint, same key)
```

**What OpenClaw reads from config.toml:**
- `endpoint` — where to send LLM requests
- `api_key` — authentication (resolves `env:` prefixes)
- `protocol` — `"openai"` or `"anthropic"` (drives request format)
- `headers` — extra HTTP headers (e.g., subscription keys)
- `[chat].default_model` — which model to use for its own reasoning

**What OpenClaw does NOT need:**
- Its own backend registration
- Its own model list
- Its own API keys
- Its own config file
- Any `openclaw.toml` or `openclaw.yaml`

The user runs `apxm backend add` once. OpenClaw works immediately.

### 2.3 APXM-Aware Tools

14 tools exposed to the LLM, implemented via ACP reverse requests:

| # | Tool | Implementation | ACP Mechanism |
|---|------|---------------|---------------|
| 1 | `list_workflows` | Scan project dir for .air/.py files | `fs/readTextFile` on directory |
| 2 | `read_workflow` | Read and return .air file contents | `fs/readTextFile` |
| 3 | `analyze_workflow` | Parse graph, compute critical path | In-process (JSON parsing) |
| 4 | `compile_workflow` | Compile at given opt level | `terminal/create` → `apxm compile` |
| 5 | `execute_workflow` | Run with params, emit session | `terminal/create` → `apxm execute` |
| 6 | `validate_workflow` | Check against AIS contract | `terminal/create` → `apxm validate` |
| 7 | `explain_workflow` | Human-readable walkthrough | `terminal/create` → `apxm explain` |
| 8 | `save_workflow` | Write .air file from graph spec | `fs/writeTextFile` |
| 9 | `list_sessions` | List `~/.apxm/sessions/` dirs | `fs/readTextFile` |
| 10 | `read_session` | Read manifest, results, metrics | `fs/readTextFile` |
| 11 | `read_session_node` | Read per-node output | `fs/readTextFile` |
| 12 | `list_operations` | AIS operation catalog | In-process (hardcoded or `apxm ops list`) |
| 13 | `read_file` | Read arbitrary project file | `fs/readTextFile` |
| 14 | `health_check` | Backend status from config | `fs/readTextFile` on config + HTTP probe |

Key design: OpenClaw doesn't link APXM crates for heavy operations. It uses the `apxm` CLI as infrastructure through `terminal/create` reverse requests, the same way Claude Code uses `cargo`, `git`, and `npm`.

---

## 3. Agent Loop

```
receive session/prompt from client
    |
    v
build request:
  system_prompt: APXM-aware preamble (~500 tokens)
  messages: conversation history + new prompt
  tools: 14 tool definitions (formatted for backend protocol)
  tool_choice: auto (or none on final round)
    |
    v
call LLM backend via reqwest:
  read config.toml → endpoint, api_key, protocol, headers
  POST to endpoint with tool-use body
  stream response
    |
    v
for each token:
  emit session/update notification (agent_message_chunk)
    |
    v
response has tool_calls? ─── yes ──> for each tool:
    |                                  1. emit status notification
    |                                  2. execute via reverse request
    |                                  3. collect result
    |                                  4. append to messages
    |                                  5. call LLM again (max 10 rounds)
    v no
return session/prompt response:
  { stopReason: "end_turn", model: "claude-sonnet-4-5" }
```

### 3.1 System Prompt

```
You are OpenClaw, an APXM-aware agent for designing, compiling, executing,
debugging, and understanding agent workflows.

APXM (Agent Program Execution Model) is a compiler and runtime for AI agent
workflows. Workflows are directed acyclic graphs of AIS operations.

Key concepts:
- AIR (.air files): Agent Intermediate Representation — MLIR-based graph format
- AIS: 40+ operations (ASK, THINK, DECIDE, SPAWN_AGENT, COMMUNICATE, FUSE, etc.)
- Compiler: O0-O3 optimization with passes (fuse-ask-ops, CSE, dead-context-elimination)
- Sessions: Execution traces stored in ~/.apxm/sessions/ with per-node output
- Backends: LLM providers configured in ~/.apxm/config.toml

Use tools proactively:
- When asked about a workflow, read it first
- When debugging, check session traces
- When building, use list_operations then save_workflow
- When optimizing, compile at O2 and analyze the diagnostics
```

Plus dynamic context from AAM preamble (beliefs, goals, capabilities) when spawned via APXM workflow.

### 3.2 Protocol Routing

OpenClaw reads `protocol` from config.toml to format tool-use requests correctly:

**Anthropic format** (`protocol = "anthropic"`):
```json
{
  "model": "claude-sonnet-4-5@20250929",
  "messages": [...],
  "system": "You are OpenClaw...",
  "tools": [
    {"name": "compile_workflow", "description": "...", "input_schema": {...}}
  ],
  "tool_choice": {"type": "auto"}
}
```

**OpenAI format** (`protocol = "openai"` or default):
```json
{
  "model": "gpt-4o",
  "messages": [...],
  "tools": [
    {"type": "function", "function": {"name": "compile_workflow", "description": "...", "parameters": {...}}}
  ],
  "tool_choice": "auto"
}
```

Tool result messages also differ:
- Anthropic: `tool_result` content block in user message
- OpenAI: `role: "tool"` message with `tool_call_id`

### 3.3 Streaming

During LLM response processing, OpenClaw emits ACP notifications for each text chunk:

```json
{"jsonrpc": "2.0", "method": "session/update", "params": {
  "update": {
    "sessionUpdate": "agent_message_chunk",
    "content": {"text": "I'll compile that workflow..."}
  }
}}
```

And token usage at the end of each LLM call:

```json
{"jsonrpc": "2.0", "method": "session/update", "params": {
  "update": {
    "sessionUpdate": "usage_update",
    "inputTokens": 1500,
    "outputTokens": 340
  }
}}
```

The ACP client (in APXM runtime or GUI bridge) accumulates these into the response text. The GUI bridge additionally forwards them as SSE events.

---

## 4. Authentication (No Separate Registration)

OpenClaw gets its LLM credentials from the same place as everything else: `~/.apxm/config.toml`.

### 4.1 Primary: config.toml (always available)

OpenClaw reads the `api_key` field from the first `[[backends]]` entry, resolving `env:` prefixes:

```toml
# Your existing config — no changes needed for OpenClaw
[[backends]]
name = "amd-gateway"
endpoint = "https://llm-api.amd.com/api"
api_key = "dummy"                              # literal key
# or: api_key = "env:ANTHROPIC_API_KEY"        # resolved from environment

[backends.headers]
Ocp-Apim-Subscription-Key = "be05a..."        # extra headers, also shared
```

OpenClaw parses this the same way the GUI's `load_backend_config()` does. Same endpoint, same key, same headers. If your APXM workflows can call Claude, OpenClaw can too — no additional setup.

### 4.2 Secondary: ACP credential injection (when spawned by APXM runtime)

When OpenClaw is spawned as part of an APXM workflow (via `SPAWN_AGENT`), the ACP client can also inject credentials through the handshake. OpenClaw can optionally declare `authMethods` in its `initialize` response:

```json
{"authMethods": [{"methodId": "anthropic-api-key"}]}
```

The ACP client resolves this from environment variables (`ANTHROPIC_API_KEY`). This is a secondary path — config.toml is always checked first.

### 4.3 What You Never Need To Do

- No `openclaw config` command
- No `openclaw backend add` command
- No `~/.openclaw/` directory
- No separate API key registration
- No model list duplication

**If APXM works, OpenClaw works.** Same config, same credentials, same models.

---

## 5. GUI Integration

### 5.1 Lightweight ACP Client in GUI

The GUI does NOT depend on the `apxm-acp` crate (which pulls `apxm-runtime`, adding ~650KB and 8 transitive deps). Instead, it implements a minimal ACP client in ~200 lines using only `tokio` + `serde_json` (already in the GUI's deps).

**New file: `crates/tools/apxm-gui/src/acp_client.rs`**

```rust
// Minimal ACP client for spawning and communicating with agents.
// Implements NDJson transport, message classification, and reverse request handling.

pub struct AgentSession {
    child: tokio::process::Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    session_id: String,
    agent_session_id: Option<String>,
    next_id: AtomicU64,
}

impl AgentSession {
    /// Spawn agent subprocess and complete ACP handshake.
    pub async fn spawn(command: &str, cwd: &Path) -> Result<Self, AgentError> { ... }

    /// Send prompt and stream token notifications via channel.
    pub async fn prompt(
        &mut self,
        text: &str,
        token_tx: mpsc::Sender<AgentEvent>,
    ) -> Result<PromptResult, AgentError> { ... }

    /// Gracefully close the session.
    pub async fn close(self) { ... }
}

pub enum AgentEvent {
    Token(String),
    ToolCall { id: String, name: String, args: serde_json::Value },
    ToolResult { id: String, success: bool, output: String },
    Usage { input_tokens: u64, output_tokens: u64 },
    Done { stop_reason: String },
    Error(String),
}
```

Key difference from `apxm-acp`: the `prompt()` method takes an `mpsc::Sender` and emits `AgentEvent::Token` for each `agent_message_chunk` notification **as it arrives**, enabling real-time SSE streaming.

### 5.2 Reverse Request Handling in GUI

When OpenClaw makes reverse requests, the GUI handles them using its existing infrastructure:

| Reverse Request | GUI Implementation |
|---|---|
| `fs/readTextFile` | `validate_path()` + `tokio::fs::read_to_string()` |
| `fs/writeTextFile` | `validate_path()` + `tokio::fs::write()` |
| `terminal/create` | `tokio::process::Command::new()` |
| `terminal/output` | Read from spawned process stdout |
| `terminal/waitForTerminalExit` | `child.wait()` |
| `terminal/kill` | `child.kill()` |
| `terminal/release` | Drop process handle |
| `requestPermission` | Auto-approve reads, deny writes (configurable) |

The `validate_path()` function (already in `main.rs`) provides path sandboxing.

### 5.3 Agent Chat Endpoint

**New file: `crates/tools/apxm-gui/src/api/agent.rs`**

```
POST /api/agent/chat
  Request: { session_id?: string, message: string }
  Response: SSE stream

  If no session_id: spawn openclaw, handshake, store session
  If session_id: reuse existing session

  SSE events:
    event: token      data: {"token": "..."}
    event: tool_call  data: {"id": "...", "name": "...", "arguments": {...}}
    event: tool_result data: {"id": "...", "name": "...", "success": bool, "output": "..."}
    event: usage      data: {"inputTokens": N, "outputTokens": N}
    event: done       data: {"stopReason": "end_turn", "sessionId": "..."}
    event: error      data: {"error": "..."}

DELETE /api/agent/sessions/{id}
  Gracefully close session (SIGTERM → SIGKILL)

GET /api/agent/sessions
  List active sessions with metadata
```

### 5.4 Session Persistence

ACP sessions are stateful — same subprocess, same conversation context across turns. The GUI manages this:

```rust
// In AppState (axum shared state)
struct AppState {
    // ... existing fields ...
    agent_sessions: DashMap<String, Arc<Mutex<AgentSession>>>,
}
```

**Lifecycle:**
1. First message: spawn `openclaw acp`, complete handshake, generate session_id, store in DashMap
2. Subsequent messages: look up session by ID, send `session/prompt` on existing subprocess
3. Explicit close: `DELETE /api/agent/sessions/{id}` → graceful shutdown (stdin close → SIGTERM → SIGKILL)
4. Idle timeout: background task reaps sessions inactive for 5 minutes
5. Server shutdown: close all sessions gracefully

### 5.5 Frontend Changes

Minimal changes to the chat UI:

**`frontend/src/api/chat.ts`** — new `streamAgentChat()` function:
```typescript
export async function streamAgentChat(
  message: string,
  sessionId: string | null,
  onToken: (token: string) => void,
  onToolCall: (call: ToolCallEvent) => void,
  onToolResult: (result: ToolResultEvent) => void,
  onDone: (sessionId: string) => void,
  onError: (error: string) => void,
  signal?: AbortSignal,
): Promise<void>
```

**`frontend/src/views/chat-view.tsx`** — additions:
- Agent mode toggle (switch between direct LLM proxy and OpenClaw agent)
- `ToolCallCard` component for rendering tool calls inline in chat
- Session ID state management
- Context passing (active workflow, active session from app-store)

**`frontend/src/styles/global.css`** — tool card styles (~40 lines)

---

## 6. Data Flow: End-to-End

### 6.1 GUI Chat → OpenClaw

```
Browser: user types "compile my workflow at O2"
    |
    v
Frontend: POST /api/agent/chat { session_id: "abc", message: "compile my workflow at O2" }
    |
    v
GUI (agent.rs): look up session "abc" in DashMap
    lock Arc<Mutex<AgentSession>>
    call session.prompt("compile my workflow at O2", token_tx)
    |
    v
GUI (acp_client.rs): send JSON-RPC to openclaw stdin:
    {"jsonrpc":"2.0","id":5,"method":"session/prompt",
     "params":{"sessionId":"...","prompt":[{"type":"text","text":"compile my workflow at O2"}]}}
    |
    v
OpenClaw (acp.rs): receive prompt, pass to agent loop
    |
    v
OpenClaw (agent.rs): build LLM request with tools
    call Claude API via reqwest
    |
    v
Claude responds: tool_call → compile_workflow({path: "workflow.air", opt_level: 2})
    |
    v
OpenClaw (tools.rs): execute tool via reverse request:
    send to stdout: {"jsonrpc":"2.0","id":100,"method":"terminal/create",
                     "params":{"command":"apxm","args":["compile","workflow.air","--opt-level","2"]}}
    |
    v
GUI (acp_client.rs): receive reverse request, spawn subprocess:
    tokio::process::Command::new("apxm").args(["compile","workflow.air","--opt-level","2"])
    wait for exit, capture output
    send response: {"jsonrpc":"2.0","id":100,"result":{"terminalId":"term_1"}}
    |
    v
OpenClaw: read terminal output, build tool result, send back to Claude
    Claude responds with text: "Compiled successfully. 12 passes applied..."
    |
    v
OpenClaw: emit streaming notifications:
    {"jsonrpc":"2.0","method":"session/update","params":{"update":
      {"sessionUpdate":"agent_message_chunk","content":{"text":"Compiled "}}}}
    {"jsonrpc":"2.0","method":"session/update","params":{"update":
      {"sessionUpdate":"agent_message_chunk","content":{"text":"successfully..."}}}}
    |
    v
GUI (acp_client.rs): receive notifications, send to token_tx channel
    |
    v
GUI (agent.rs): read from channel, emit SSE events:
    event: token  data: {"token": "Compiled "}
    event: token  data: {"token": "successfully..."}
    event: done   data: {"stopReason": "end_turn", "sessionId": "abc"}
    |
    v
Frontend: accumulate tokens into message, render in chat
```

### 6.2 APXM Workflow → OpenClaw

Same binary, same protocol, different client:

```python
from apxm import compile, GraphRecorder
from apxm._generated.agents import openclaw

@compile()
def meta_workflow(g: GraphRecorder):
    # Spawn OpenClaw as a node in a larger workflow
    agent = g.spawn("builder", profile=openclaw)
    agent.ask("Create a 3-node summarization workflow")
    result = g.done(agent.get_last_node())
```

APXM runtime spawns `openclaw acp` via `AcpSession::spawn()`, performs the same handshake, sends `session/prompt`, handles reverse requests via `CapabilityReverseHandler`. OpenClaw doesn't know or care whether it's talking to the GUI or the runtime.

---

## 7. Crate Structure

```
crates/
├── agents/
│   └── openclaw/                          ← NEW CRATE
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs                    CLI entrypoint: `openclaw acp`
│           ├── acp.rs                     ACP protocol server (stdin/stdout)
│           ├── agent.rs                   Agent loop (LLM + tool dispatch)
│           ├── tools.rs                   14 tool definitions + execution
│           ├── llm.rs                     LLM client (reqwest, protocol routing)
│           └── config.rs                  Config.toml parser
│
├── tools/
│   └── apxm-gui/                          ← MODIFIED
│       ├── Cargo.toml                     No new dependencies
│       └── src/
│           ├── main.rs                    Add route, add AppState field
│           ├── acp_client.rs              NEW: lightweight ACP client (~200 lines)
│           └── api/
│               ├── mod.rs                 Add `pub mod agent;`
│               ├── agent.rs              NEW: agent chat endpoint (~150 lines)
│               ├── chat.rs               Unchanged (legacy LLM proxy)
│               └── live.rs               Unchanged
│
├── orchestration/
│   └── apxm-acp/                          ← UNCHANGED
│       └── src/
│           └── registry.rs               ("openclaw", "openclaw acp", dg, dt) already registered
```

### 7.1 OpenClaw Dependencies

```toml
[package]
name = "openclaw"
version = "0.1.0"
edition = "2024"

[dependencies]
# Async runtime + process management
tokio = { version = "1", features = ["rt-multi-thread", "macros", "io-util", "process", "time", "sync"] }

# HTTP client for LLM API calls
reqwest = { version = "0.12", features = ["json", "rustls-tls", "stream"], default-features = false }

# JSON handling
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Config parsing
toml = "0.9"

# Command parsing (for ACP command strings)
shell-words = "1.1"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Error handling
anyhow = "1"
thiserror = "2"

# UUID for session IDs
uuid = { version = "1", features = ["v4"] }

# futures for stream processing
futures = "0.3"
```

**No APXM workspace crate dependencies.** OpenClaw is fully independent. It reads config.toml as text, shells out to `apxm` CLI for heavy operations, and speaks ACP on stdio. This means:

- Independent release cycle
- No recompilation when APXM internals change
- Can be installed separately
- Can be used with any ACP-compatible orchestrator, not just APXM

### 7.2 GUI Changes (No New Dependencies)

The GUI's `acp_client.rs` uses only crates already in Cargo.toml:
- `tokio` (process, io-util, sync) — already present
- `serde_json` — already present
- `futures` — already present

Zero new Cargo dependencies for the GUI.

---

## 8. Interaction Scenarios

### 8.1 Build a Workflow

```
User: Build me a workflow that researches a topic and writes a blog post

OpenClaw thinks: I need to know what operations are available.
  → calls list_operations (in-process or `apxm ops list --json`)

OpenClaw thinks: I'll create a 3-node chain.
  → calls save_workflow:
    fs/writeTextFile → writes blog_writer.air with research→outline→write nodes

OpenClaw thinks: Let me compile it to verify.
  → calls compile_workflow:
    terminal/create → `apxm compile blog_writer.air --opt-level 2`
    terminal/output → reads compilation result

OpenClaw thinks: Let me analyze parallelism.
  → calls analyze_workflow (in-process JSON parsing of the .air)

OpenClaw responds:
  "I've created blog_writer.air — a 3-node sequential chain:
   research (ASK) → outline (ASK) → write (ASK)
   Compiled at O2: 12 passes applied. Critical path: 3 nodes.
   Parameter: topic (str). Want me to run it?"
```

### 8.2 Debug a Failure

```
User: My chained_llm workflow failed, what happened?

OpenClaw → list_sessions (fs/readTextFile on ~/.apxm/sessions/)
OpenClaw → read_session (fs/readTextFile on latest session manifest)
OpenClaw → read_session_node (fs/readTextFile on failed node output)

OpenClaw responds:
  "Node 2 (analyze) failed with 429 Too Many Requests after 3 retries.
   Node 1 (research) succeeded in 2.1s.
   Options: route to a different backend, add delay, or increase backoff."
```

### 8.3 Explain and Optimize

```
User: What does shared_prefix_fanout do and can it be optimized?

OpenClaw → read_workflow (fs/readTextFile on the .py file)
OpenClaw → compile_workflow (terminal → `apxm compile --opt-level 2 --emit-diagnostics`)
OpenClaw → analyze_workflow (parse the compiled graph)

OpenClaw responds:
  "shared_prefix_fanout has 6 nodes: 5 ASK + 1 DONE.
   Max parallelism: 4 (fanout after shared prefix). Critical path: 3.
   At O2: fuse-ask-ops fused 2 nodes, CSE found 1 common subexpression.
   The vLLM backend would serve the shared prefix from KV cache."
```

### 8.4 Infrastructure Check

```
User: Are my backends healthy?

OpenClaw → health_check:
  fs/readTextFile on ~/.apxm/config.toml (parse backends)
  optional: HTTP GET to each endpoint for liveness

OpenClaw responds:
  "3 backends configured:
   - anthropic: healthy (7 models, https://api.anthropic.com)
   - openai: healthy (13 models, https://api.openai.com)
   - local-vllm: unreachable (http://localhost:8000) — is the server running?"
```

---

## 9. ACP Wire Protocol Reference

### 9.1 Methods OpenClaw Must Handle

| Method | Phase | Required |
|--------|-------|----------|
| `initialize` | Handshake | Yes |
| `authenticate` | Handshake | If authMethods declared |
| `session/new` | Handshake | Yes |
| `session/prompt` | Prompt loop | Yes |
| `session/set_mode` | Control | Optional |
| `session/set_config_option` | Control | Optional |
| `session/cancel` | Control | Optional |

### 9.2 Notifications OpenClaw Must Emit

| Notification | When | Format |
|---|---|---|
| `session/update` (agent_message_chunk) | Each text token | `{update: {sessionUpdate: "agent_message_chunk", content: {text: "..."}}}` |
| `session/update` (usage_update) | After each LLM call | `{update: {sessionUpdate: "usage_update", inputTokens: N, outputTokens: N}}` |

### 9.3 Reverse Requests OpenClaw May Send

| Method | Purpose | Params |
|---|---|---|
| `fs/readTextFile` | Read files | `{path, line?}` |
| `fs/writeTextFile` | Write files | `{path, content}` |
| `terminal/create` | Run commands | `{command, args, cwd?, env?}` |
| `terminal/output` | Get command output | `{terminalId}` |
| `terminal/waitForTerminalExit` | Wait for completion | `{terminalId}` |
| `terminal/kill` | Kill process | `{terminalId}` |
| `terminal/release` | Release resources | `{terminalId}` |
| `requestPermission` | Ask for approval | `{toolCall, options}` |

### 9.4 Constants

```
Protocol version: 1
Client name: "apxm" / version: "0.1.0"
Stop reason: "end_turn"
NDJson framing: one JSON object per line, terminated by \n
Timeouts:
  initialize: 30s (default)
  authenticate: 10s
  session control: 10s
  system preamble: 120s
  close grace: 100ms → SIGTERM → 1.5s → SIGKILL → 1s
```

---

## 10. Build and Installation

### 10.1 Building

```bash
# Build OpenClaw
cargo build -p openclaw --release
# → target/release/openclaw

# Build GUI (unchanged)
cargo build -p apxm-gui --release
# → target/release/apxm-gui

# Or build everything
dekk apxm build     # add openclaw to workspace members
```

### 10.2 Installation

```bash
# OpenClaw on PATH (required for ACP spawning)
cp target/release/openclaw ~/.local/bin/
# or: ln -s $(pwd)/target/release/openclaw ~/.local/bin/openclaw

# Verify
openclaw acp --help
```

### 10.3 Configuration

**There is no OpenClaw configuration step.** OpenClaw reads `~/.apxm/config.toml`, which you already have if you use APXM:

```bash
# This is your EXISTING config (already done):
apxm backend add amd-gateway --protocol openai --endpoint https://llm-api.amd.com/api

# OpenClaw reads the same file. Nothing else to configure.
# No `openclaw init`, no `openclaw config`, no second config file.
```

OpenClaw picks up:
- Backend endpoint and API key → for its own LLM reasoning
- Model list → for the GUI model selector dropdown
- `[chat].default_model` → which model OpenClaw uses by default
- Custom headers → passed through to LLM API calls

If you change config.toml (add a backend, add a model, change routing), OpenClaw sees it on next session.

### 10.4 Usage

**From GUI:**
```
apxm view
# Open chat → toggle "Agent mode" → talk to OpenClaw
```

**From APXM workflow:**
```python
agent = g.spawn("assistant", profile=openclaw)
agent.ask("Build me a summarization workflow")
```

**Standalone testing:**
```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{...}}' | openclaw acp
```

---

## 11. Files Modified/Created

| File | Change | Lines |
|------|--------|-------|
| `crates/agents/openclaw/Cargo.toml` | **NEW** | ~30 |
| `crates/agents/openclaw/src/main.rs` | **NEW**: CLI + ACP server | ~80 |
| `crates/agents/openclaw/src/acp.rs` | **NEW**: Protocol handler | ~250 |
| `crates/agents/openclaw/src/agent.rs` | **NEW**: Agent loop | ~300 |
| `crates/agents/openclaw/src/tools.rs` | **NEW**: 14 tools + dispatch | ~400 |
| `crates/agents/openclaw/src/llm.rs` | **NEW**: LLM client | ~250 |
| `crates/agents/openclaw/src/config.rs` | **NEW**: Config parser | ~80 |
| `crates/tools/apxm-gui/src/acp_client.rs` | **NEW**: ACP client | ~200 |
| `crates/tools/apxm-gui/src/api/agent.rs` | **NEW**: Agent endpoint | ~150 |
| `crates/tools/apxm-gui/src/api/mod.rs` | Add `pub mod agent` | ~1 |
| `crates/tools/apxm-gui/src/main.rs` | Add routes, AppState field | ~15 |
| `crates/tools/apxm-gui/Cargo.toml` | **No changes** | — |
| `frontend/src/api/chat.ts` | Add `streamAgentChat()` | ~60 |
| `frontend/src/views/chat-view.tsx` | Agent toggle + tool cards | ~150 |
| `frontend/src/styles/global.css` | Tool card styles | ~40 |
| `Cargo.toml` (workspace) | Add openclaw to members | ~1 |

**Total new code**: ~2000 lines (OpenClaw ~1400, GUI bridge ~400, frontend ~250)

---

## 12. Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Cold start latency (spawn + handshake) | ~500ms one-time cost; session persists across turns |
| OpenClaw subprocess crashes | GUI detects broken pipe, returns error, allows retry |
| Tool execution blocks response | 15s SSE keepalive; tool_call notification shows spinner |
| Infinite tool loops | MAX_TOOL_ROUNDS=10; last round: tool_choice=none |
| Large file reads | Cap at 500 lines in read_file tool |
| Conversation grows large | Frontend manages history; truncation strategy TBD |
| Reverse request denied | Permission mode configurable; graceful error in tool result |
| `openclaw` not on PATH | GUI checks on startup, shows error banner with install instructions |
| Anthropic vs OpenAI format | Protocol field from config.toml drives formatting |

---

## 13. Future Directions

### 13.1 OpenClaw in APXM Workflows

Once the binary exists, it works immediately in APXM workflows via the already-registered ACP template. No additional integration needed.

### 13.2 MCP Server Mode

`openclaw mcp` — expose APXM operations as MCP tools. This lets any MCP-compatible client (Claude Desktop, VS Code, etc.) use APXM through OpenClaw.

### 13.3 Chat as Primary View

After OpenClaw works, promote chat to the primary GUI tab:
- Move chat to position 1 in activities array
- Default activeTab to "chat"
- Cross-view navigation: tool results with paths become clickable links
- Persist chat history in localStorage

### 13.4 Separate Repository

If OpenClaw grows beyond APXM awareness (general coding agent), extract to its own repo. The ACP interface is the clean boundary — nothing needs to change on the APXM side.

---

## 14. Verification

1. `cargo build -p openclaw` — compiles cleanly
2. `echo '...' | openclaw acp` — handshake completes
3. GUI spawns OpenClaw — session established, tokens stream
4. "list my workflows" — calls list_workflows tool via fs/readTextFile
5. "compile chained_llm at O2" — calls compile_workflow via terminal/create → `apxm compile`
6. "create a summarization workflow" — calls save_workflow via fs/writeTextFile
7. "why did my last run fail?" — calls list_sessions + read_session
8. Session persistence — multiple turns in same conversation
9. Error handling — ask about non-existent file, get graceful error
10. Protocol compat — test with both Anthropic and OpenAI backends

---

## 15. Decision Record

**Why standalone binary (not embedded in GUI)?**
- Clean separation: APXM = infrastructure, OpenClaw = agent
- Works in both GUI chat AND APXM workflows (same binary, same protocol)
- Independent release cycle and dependency tree
- Testable in isolation (`echo | openclaw acp`)
- No new Cargo deps for the GUI
- Matches the pattern of all 16 ACP agents (Claude, Codex, Gemini, etc.)

**Why lightweight ACP client in GUI (not full apxm-acp crate)?**
- apxm-acp depends on apxm-runtime (~650KB, 8 transitive workspace crates)
- GUI only needs: spawn, handshake, prompt, notification streaming
- ~200 lines vs pulling in ProcessTable, CapabilitySystem, ExecutorEngine
- Clean upgrade path: swap to full crate later if needed

**Why reverse requests for tools (not linking APXM crates)?**
- OpenClaw stays independent — no compile-time coupling to APXM internals
- Same pattern as Claude Code using git/npm as CLI tools
- ACP reverse requests provide sandboxed file I/O and terminal access
- Permission modes (approve-all, approve-reads, deny-all) give the client control
