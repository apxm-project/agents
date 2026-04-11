# OpenClaw Integration: Using the Real OpenClaw with APXM Backends

> **Status**: Investigation complete, ready for implementation
> **Date**: 2026-04-11
> **Scope**: Integrate the existing OpenClaw project as APXM-GUI's chat agent, sharing APXM's backends

---

## 1. Core Principle

**APXM is infrastructure. OpenClaw is an existing agent. The only change is config bridging.**

OpenClaw (github.com/openclaw/openclaw) is a production Node.js/TypeScript AI agent with 247K GitHub stars. It already speaks ACP over stdio (`openclaw acp`), has its own tool system, and runs as a standalone process.

We do NOT build a new agent. We:
1. Install OpenClaw
2. Generate an OpenClaw config that points at APXM's backends
3. Have the GUI spawn it via ACP

```
~/.apxm/config.toml                    ~/.openclaw/openclaw.json
(APXM single source of truth)          (generated from config.toml)
         |                                       |
    apxm-gui reads                         openclaw reads
    config.toml to                         openclaw.json for
    generate openclaw.json                 LLM provider config
         |                                       |
         +----------> apxm-gui spawns ---------->+
                      `openclaw acp`
                      as subprocess
```

---

## 2. The Config Bridge Problem

### 2.0 Step 0: Enrich APXM's ModelConfig

Before bridging, APXM's `ModelConfig` was missing one field that OpenClaw requires: `max_output_tokens`.
This has been added to `apxm-core/src/types/backend.rs`:

```rust
pub struct ModelConfig {
    pub id: String,
    pub aliases: Vec<String>,
    pub context_window: usize,          // OpenClaw: contextWindow
    pub cost_per_1k_input: f64,         // OpenClaw: cost.input
    pub cost_per_1k_output: f64,        // OpenClaw: cost.output
    pub supports_vision: bool,          // OpenClaw: input (["text","image"] vs ["text"])
    pub supports_functions: bool,       // OpenClaw: supportsTools (via compat)
    pub supports_thinking: bool,        // OpenClaw: reasoning
    pub max_output_tokens: Option<usize>, // NEW — OpenClaw: maxTokens
    pub tags: Vec<String>,
}
```

With this addition, config.toml contains **every field** OpenClaw needs. The sync add-on is a pure translation, not a gap-filler.

### 2.1 APXM Config Format (`~/.apxm/config.toml`)

```toml
[[backends]]
name = "amd-gateway"
type = "onprem"
protocol = "openai"
endpoint = "https://llm-api.amd.com/api"
api_key = "dummy"

[backends.headers]
Ocp-Apim-Subscription-Key = "be05a9ef06cf46c2a7a76b558aa4c8d4"
user = "env:USER"

[[backends.models]]
id = "claude-sonnet-4-5@20250929"
aliases = ["claude", "sonnet"]
context_window = 200000
max_output_tokens = 8192
supports_vision = true
supports_functions = true

[[backends.models]]
id = "claude-haiku-4-5@20251001"
aliases = ["haiku", "fast"]
context_window = 200000
max_output_tokens = 8192
supports_functions = true

# ... 17 models total, all with max_output_tokens
```

### 2.2 OpenClaw Config Format (`~/.openclaw/openclaw.json`)

```json5
{
  "models": {
    "providers": {
      "amd-gateway": {
        "baseUrl": "https://llm-api.amd.com/api/chat/completions",
        "apiKey": "dummy",
        "api": "openai-completions",
        "headers": {
          "Ocp-Apim-Subscription-Key": "be05a9ef06cf46c2a7a76b558aa4c8d4"
        },
        "models": [
          {
            "id": "claude-sonnet-4-5@20250929",
            "name": "Claude Sonnet 4.5",
            "reasoning": false,
            "input": ["text", "image"],
            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
            "contextWindow": 200000,
            "maxTokens": 8192
          },
          {
            "id": "claude-haiku-4-5@20251001",
            "name": "Claude Haiku 4.5",
            "reasoning": false,
            "input": ["text"],
            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
            "contextWindow": 200000,
            "maxTokens": 8192
          }
        ]
      }
    }
  }
}
```

### 2.3 Field Mapping (APXM → OpenClaw)

| APXM config.toml | OpenClaw openclaw.json | Notes |
|---|---|---|
| `endpoint` | `baseUrl` | Append `/chat/completions` for OpenAI protocol |
| `api_key` | `apiKey` | Resolve `env:` prefixes |
| `protocol = "openai"` | `api: "openai-completions"` | Map protocol to OpenClaw API type |
| `protocol = "anthropic"` | `api: "anthropic-messages"` | Direct mapping |
| `backends.headers` | `headers` | Copy as-is, resolve `env:` values |
| `backends.models[].id` | `models[].id` | Direct copy |
| `backends.models[].context_window` | `models[].contextWindow` | Direct copy |
| `backends.models[].supports_vision` | `models[].input` | `true` → `["text","image"]`, else `["text"]` |
| (not in APXM) | `models[].reasoning` | Infer from model ID (o3, o4, DeepSeek-R1 → true) |
| (not in APXM) | `models[].cost` | All zeros (on-prem, no billing) |
| (not in APXM) | `models[].maxTokens` | Default 8192 |
| (not in APXM) | `models[].name` | Generated from ID |

### 2.4 Protocol Mapping

| APXM `protocol` | OpenClaw `api` |
|---|---|
| `"openai"` | `"openai-completions"` |
| `"anthropic"` | `"anthropic-messages"` |
| `"ollama"` | `"ollama"` |

OpenClaw supports 9 API types: `openai-completions`, `openai-responses`, `openai-codex-responses`, `anthropic-messages`, `google-generative-ai`, `github-copilot`, `bedrock-converse-stream`, `ollama`, `azure-openai-responses`.

---

## 3. The Config Generator

### 3.1 Implementation: `apxm openclaw sync`

A new subcommand that reads `~/.apxm/config.toml` and writes/updates `~/.openclaw/openclaw.json`:

```rust
// In apxm-cli or as standalone script
fn sync_openclaw_config(apxm_config: &ApxmConfig) -> OpenClawConfig {
    let mut providers = HashMap::new();

    for backend in &apxm_config.backends {
        let api = match backend.protocol.as_str() {
            "openai" => "openai-completions",
            "anthropic" => "anthropic-messages",
            "ollama" => "ollama",
            _ => "openai-completions",
        };

        let base_url = if api == "openai-completions" {
            format!("{}/chat/completions", backend.endpoint.trim_end_matches('/'))
        } else {
            backend.endpoint.clone()
        };

        let models: Vec<OpenClawModel> = backend.models.iter().map(|m| {
            OpenClawModel {
                id: m.id.clone(),
                name: humanize_model_id(&m.id),
                reasoning: is_reasoning_model(&m.id),
                input: if m.supports_vision.unwrap_or(false) {
                    vec!["text", "image"]
                } else {
                    vec!["text"]
                },
                cost: Cost::zero(), // on-prem
                context_window: m.context_window.unwrap_or(128000),
                max_tokens: 8192,
            }
        }).collect();

        let mut headers = HashMap::new();
        for (k, v) in &backend.headers {
            headers.insert(k.clone(), resolve_env_value(v));
        }

        providers.insert(backend.name.clone(), OpenClawProvider {
            base_url,
            api_key: resolve_env_value(&backend.api_key),
            api: api.to_string(),
            headers,
            models,
        });
    }

    OpenClawConfig {
        models: ModelsConfig {
            providers,
        },
    }
}
```

### 3.2 When to Run

- **Manual**: `apxm openclaw sync` (or `dekk apxm openclaw sync`)
- **Automatic**: GUI runs it on startup before spawning OpenClaw
- **Watch mode**: Optional file watcher on config.toml that re-syncs

### 3.3 Merge Behavior

OpenClaw's `models.mode` supports `"merge"` (default) and `"replace"`:
- With `"merge"`: our generated providers merge with any user-defined OpenClaw providers
- With `"replace"`: our config completely replaces providers

We use `"merge"` so users can add OpenClaw-specific providers alongside APXM backends.

---

## 4. GUI Integration

### 4.1 Lightweight ACP Client

The GUI spawns `openclaw acp` as a subprocess and communicates via NDJson on stdio. This is the same ACP protocol already implemented in `apxm-acp`, but we use a lightweight ~200-line client to avoid pulling in `apxm-runtime`.

**New file: `crates/tools/apxm-gui/src/acp_client.rs`**

```rust
pub struct AgentSession {
    child: tokio::process::Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    session_id: String,
    next_id: AtomicU64,
}

impl AgentSession {
    pub async fn spawn(command: &str, cwd: &Path) -> Result<Self, AgentError>;
    pub async fn prompt(&mut self, text: &str, tx: mpsc::Sender<AgentEvent>) -> Result<PromptResult, AgentError>;
    pub async fn close(self);
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

### 4.2 Startup Sequence

```
1. GUI starts
2. Read ~/.apxm/config.toml
3. Generate/update ~/.openclaw/openclaw.json (config sync)
4. On first chat message: spawn `openclaw acp`, complete ACP handshake
5. Store session in DashMap<String, Arc<Mutex<AgentSession>>>
6. Subsequent messages: reuse session
```

### 4.3 Agent Chat Endpoint

```
POST /api/agent/chat
  Request:  { session_id?: string, message: string }
  Response: SSE stream

SSE events:
  event: token       data: {"token": "..."}
  event: tool_call   data: {"id": "...", "name": "...", "arguments": {...}}
  event: tool_result  data: {"id": "...", "success": bool, "output": "..."}
  event: usage       data: {"inputTokens": N, "outputTokens": N}
  event: done        data: {"stopReason": "end_turn", "sessionId": "..."}
  event: error       data: {"error": "..."}

DELETE /api/agent/sessions/{id}
GET /api/agent/sessions
```

### 4.4 Reverse Request Handling

When OpenClaw makes ACP reverse requests, the GUI handles them:

| Reverse Request | GUI Implementation |
|---|---|
| `fs/readTextFile` | `validate_path()` + `tokio::fs::read_to_string()` |
| `fs/writeTextFile` | `validate_path()` + `tokio::fs::write()` |
| `terminal/create` | `tokio::process::Command::new()` |
| `terminal/output` | Read from spawned process stdout |
| `requestPermission` | Auto-approve reads, prompt for writes |

### 4.5 Frontend Changes

- `chat.ts`: new `streamAgentChat()` function
- `chat-view.tsx`: Agent mode toggle, tool call rendering, session management
- `global.css`: Tool card styles (~40 lines)

---

## 5. What Gets Built vs What Already Exists

### Already Exists (NO code changes)

| Component | Location | Status |
|---|---|---|
| OpenClaw agent | `github.com/openclaw/openclaw` | Production, 247K stars |
| ACP protocol | `openclaw acp` command | Built-in to OpenClaw |
| Agent registry entry | `apxm-acp/src/registry.rs:156` | `("openclaw", "openclaw acp", ...)` |
| Tool system | OpenClaw built-in | 14+ tools for file I/O, terminal, etc. |
| LLM client | OpenClaw built-in | Reads `~/.openclaw/openclaw.json` |

### New Code (to be built)

| Component | Location | Lines |
|---|---|---|
| Config sync (`apxm openclaw sync`) | `apxm-cli` or standalone | ~200 |
| GUI ACP client | `apxm-gui/src/acp_client.rs` | ~200 |
| GUI agent endpoint | `apxm-gui/src/api/agent.rs` | ~150 |
| GUI main.rs changes | Routes + AppState | ~15 |
| Frontend agent UI | `chat.ts`, `chat-view.tsx`, CSS | ~250 |

**Total new code: ~815 lines** (not ~2000 as in the previous incorrect plan)

---

## 6. Installation

```bash
# 1. Install OpenClaw (npm package)
npm install -g openclaw

# 2. Verify ACP mode works
openclaw acp --help

# 3. Sync APXM backends to OpenClaw config
apxm openclaw sync
# Reads ~/.apxm/config.toml → writes ~/.openclaw/openclaw.json

# 4. Start the GUI
apxm view
# Chat tab now has "Agent mode" toggle
```

After step 3, OpenClaw uses the exact same endpoints, API keys, and models as APXM. You register backends **once** in config.toml. The sync command translates.

---

## 7. The Answer: One Change, Zero Duplication

**Q: Do we need to register models in both APXM and OpenClaw?**

**A: No. You register in APXM (`config.toml`). A sync command generates OpenClaw's config from it.**

The "only change to OpenClaw" is its config file — and we don't even edit it by hand. The `apxm openclaw sync` command reads your existing `~/.apxm/config.toml` and writes the equivalent `~/.openclaw/openclaw.json` with the correct field names and format.

```
config.toml (you maintain)
    │
    │  apxm openclaw sync
    │  (automatic translation)
    ▼
openclaw.json (generated, never hand-edited)
    │
    │  openclaw reads on startup
    ▼
Same endpoint, same key, same headers, same models
```

When you add a backend to APXM (`apxm backend add ...`), run `apxm openclaw sync` (or let the GUI do it automatically on startup). OpenClaw sees the new backend immediately.

---

## 8. Implementation Order

1. **Config sync command** (~200 lines)
   - Parse config.toml backends
   - Map to OpenClaw's `ModelProviderConfig` schema
   - Write/merge into `~/.openclaw/openclaw.json`
   - Handle `env:` prefix resolution in API keys and headers

2. **GUI ACP client** (~200 lines)
   - Spawn `openclaw acp` subprocess
   - NDJson transport (stdin/stdout)
   - ACP handshake (initialize → session/new)
   - Prompt with streaming notifications
   - Reverse request handling (file I/O, terminal)

3. **GUI agent endpoint** (~150 lines)
   - `POST /api/agent/chat` → SSE stream
   - Session management in DashMap
   - Config sync on startup

4. **Frontend agent UI** (~250 lines)
   - Agent mode toggle
   - Tool call card rendering
   - Session ID state management

---

## 9. Risks and Mitigations

| Risk | Mitigation |
|---|---|
| OpenClaw not installed | GUI checks on startup, shows install instructions |
| Config sync stale | GUI runs sync on startup; manual `apxm openclaw sync` |
| OpenClaw format changes | Pin OpenClaw version; config schema is stable (typed) |
| `env:USER` resolution | Sync command resolves env refs at generation time |
| Cold start latency | ~500ms one-time; session persists across turns |
| Subprocess crashes | GUI detects broken pipe, returns error, allows retry |

---

## 10. Decision Record

**Why use existing OpenClaw (not build a new agent)?**
- OpenClaw is a production agent with 247K stars
- Already speaks ACP over stdio
- Has a mature tool system, session management, and LLM client
- Building from scratch would duplicate existing work
- APXM is infrastructure — it should leverage existing agents, not replace them

**Why config sync (not modifying OpenClaw source)?**
- Zero changes to OpenClaw's codebase
- Uses OpenClaw's standard config path (`~/.openclaw/openclaw.json`)
- OpenClaw's `ModelProviderConfig` type already supports custom providers with arbitrary baseUrl, apiKey, headers
- Our on-prem backends (amd-gateway, vllm-local) are just OpenAI-compatible endpoints — OpenClaw handles them natively via `openai-completions` API type
- Merge mode preserves any additional OpenClaw-specific config the user sets

**Why not have OpenClaw read config.toml directly?**
- Different schemas (TOML vs JSON5, different field names)
- Would require forking/modifying OpenClaw
- Config sync is a clean boundary — each tool reads its own config format
- OpenClaw's config system is complex (includes, env substitution, secret management) — better to use it as-is
