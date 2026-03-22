# Phase 1: Codex Changes -- LLM Backend

**v7 -- Revised with gap analysis**

**Timeline:** Weeks 1-6 (6.5 days estimated)
**Depends on:** [APXM Phase 1 (A0-A4)](apxm.md) -- `apxm-events` crate with 33 event payloads, `apxm-backends` with `StreamAssembler` and 5 LLM backends
**Does NOT depend on:** [Gemini-CLI Phase 1](gemini-cli.md) -- these are independent
**Global plan:** [Plan 2: Codex Changes (all phases)](../plan-2-codex-changes.md)

---

## Current Architecture (Key Files)

| File | Role | Lines |
|------|------|-------|
| `core/src/client.rs` | `ModelClient` -- session LLM client, WebSocket/SSE transport, retry | 1823 |
| `core/src/client_common.rs` | `Prompt`, tool specs, `ResponseEvent` re-export | ~328 |
| `core/src/codex.rs` | `submission_loop()` -- main dispatch loop (line ~4138), `RegularTask::run()`, ResponseEvent to ServerNotification mapping | 7321 |
| `core/src/event_mapping.rs` | `ResponseItem` to `TurnItem` conversion | ~173 |
| `core/src/apxm_provider_config.rs` | **ALREADY EXISTS** -- loads APXM backends, converts to `ModelProviderInfo` | 336 |
| `codex-api/src/common.rs` | `ResponsesApiRequest`, `ResponseEvent` (12 variants), `ResponseCreateWsRequest` | 281 |
| `codex-api/src/sse/responses.rs` | SSE parsing for Responses API (tightly coupled) | 1059 |
| `codex-api/src/provider.rs` | `Provider` struct (base_url, headers, retry, idle_timeout) | 170 |
| `app-server-protocol/src/protocol/common.rs` | `server_notification_definitions!` (49 variants) | 1720 |
| `app-server-protocol/src/protocol/v2.rs` | `ThreadItem` (16 variants) | 7978 |

### Pre-existing APXM Integration

Codex already depends on `apxm-core` (in `core/Cargo.toml`) and has `apxm_provider_config.rs` which:
- Loads APXM backend configs from `~/.apxm/config.toml` and `~/.apxm/credentials.toml`
- Converts them to `ModelProviderInfo` with `WireApi::Responses`
- Resolves provider protocols (OpenAI, Anthropic, Google, Ollama)
- Is already integrated into model selection

This means Phase C1 (dependencies) is partially complete -- `apxm-core` is already wired in, though currently via relative path dependency.

### 26 ToolHandler Implementations (21 standard + 5 multi_agents)

Codex routes tool calls through a `ToolRouter` to 26 total `ToolHandler` implementations. The 21 standard handlers are listed below; the 5 `multi_agents/` handlers (spawn, wait, send_input, resume_agent, close_agent) are deferred to Phase 4 (`FLOW_CALL`/`WAIT_ALL`):

| # | Handler | Module | Category |
|---|---------|--------|----------|
| 1 | `BatchJobHandler` | `agent_jobs.rs` | Multi-agent |
| 2 | `ApplyPatchHandler` | `apply_patch.rs` | File mutation |
| 3 | `ArtifactsHandler` | `artifacts.rs` | File I/O |
| 4 | `DynamicToolHandler` | `dynamic.rs` | Dynamic dispatch |
| 5 | `GrepFilesHandler` | `grep_files.rs` | Search |
| 6 | `JsReplHandler` | `js_repl.rs` | Code execution |
| 7 | `JsReplResetHandler` | `js_repl.rs` | Code execution |
| 8 | `ListDirHandler` | `list_dir.rs` | File I/O |
| 9 | `McpHandler` | `mcp.rs` | MCP integration |
| 10 | `McpResourceHandler` | `mcp_resource.rs` | MCP integration |
| 11 | `PlanHandler` | `plan.rs` | Planning |
| 12 | `ReadFileHandler` | `read_file.rs` | File I/O |
| 13 | `RequestPermissionsHandler` | `request_permissions.rs` | Approval |
| 14 | `RequestUserInputHandler` | `request_user_input.rs` | Human-in-loop |
| 15 | `ShellHandler` | `shell.rs` | Shell execution |
| 16 | `ShellCommandHandler` | `shell.rs` | Shell execution |
| 17 | `TestSyncHandler` | `test_sync.rs` | Testing |
| 18 | `ToolSearchHandler` | `tool_search.rs` | Discovery |
| 19 | `ToolSuggestHandler` | `tool_suggest.rs` | Discovery |
| 20 | `UnifiedExecHandler` | `unified_exec.rs` | Unified dispatch |
| 21 | `ViewImageHandler` | `view_image.rs` | Media |

Additionally, the `multi_agents/` submodule contains 5 internal handlers (`spawn`, `wait`, `send_input`, `resume_agent`, `close_agent`) managed by the `BatchJobHandler` coordinator.

### Key Types

```
codex-api::ResponseEvent        -- SSE events from OpenAI (item/started, item/delta, item/completed, etc.)
codex-api::ResponsesApiRequest  -- request payload (model, instructions, input, tools, reasoning)
codex_protocol::ServerNotification -- 49-variant enum, delivered to TUI/JSON-RPC clients
codex_protocol::ThreadItem      -- 16-variant enum (what the user sees)
core::client::ModelClient       -- session-scoped LLM client
core::client::ModelClientSession -- per-turn WebSocket/SSE connection
core::client_common::Prompt     -- per-turn request payload
core::guardian::GuardianReviewSessionManager -- risk-scoring sub-agent for tool approval
```

### Approval Flow

Tool approval flows through `GuardianReviewSessionManager` -- a risk-scoring sub-agent that evaluates tool calls and provides a verdict, with fallback to user approval. This maps directly to APXM's VERIFY operation and capability interceptor pipeline.

---

## Codex Operations to AIS Mapping

Every operation Codex performs today has a direct AIS equivalent. This mapping is the blueprint for all five phases:

| Codex Operation | Current Implementation | AIS Equivalent | Phase |
|----------------|----------------------|---------------|-------|
| LLM call (prompt to response) | `RegularTask::run()` via OpenAI API | **ASK** / **THINK** / **REASON** | 1 |
| Extended thinking | `ReasoningEffortConfig` | **THINK** (with `thinking_budget` attribute) | 1 |
| Shell execution | `ShellHandler`, `ShellCommandHandler` | **INV** (sandboxed capability) | 2 |
| File read/write | `ReadFileHandler`, `ApplyPatchHandler` | **INV** (capability) | 2 |
| Tool search/suggest | `ToolSearchHandler`, `ToolSuggestHandler` | **QMEM** (capability lookup) | 2 |
| MCP tool calls | `McpHandler`, `McpResourceHandler` | **INV** (MCP capability) | 2 |
| Guardian approval | `GuardianReviewSessionManager` | **VERIFY** (claim to verdict) | 2 |
| Provider selection | `apxm_provider_config.rs` | **AAM Capabilities** `(C: Map<Name, Sig>)` | 2 |
| Response events | `ResponseEvent` to `ServerNotification` | **apxm-events** `(EventPayload)` | 1 |
| Session/Turn state | `SessionState`, `TurnState` | **AAM Beliefs** `(B: Map<Key, TypedValue>)` | 3 |
| Context compaction (2-phase) | Phase 1: `gpt-5.1-codex-mini`, Phase 2: `gpt-5.3-codex` | **QMEM** + **UMEM** (tiered memory) | 3 |
| Active instructions | Config + system prompt | **AAM Goals** `(G: PriorityQueue<Goal>)` | 3 |
| Available tools | `ToolRouter` registry | **AAM Capabilities** `(C: Map<Name, Sig>)` | 3 |
| Multi-agent spawn/wait | `multi_agents/spawn.rs`, `wait.rs` | **FLOW_CALL** / **WAIT_ALL** | 4 |
| Shadow comparison | (planned -- C3.3) | **REFLECT** (analyze execution trace) | 4 |
| `submission_loop()` dispatch | `codex.rs:4138` | **AIS Graph** (compiled DAG) | 4 |

---

## C1: Add APXM Dependencies (0.5 days)

### C1.1 Workspace root

**File:** `openai/codex/codex-rs/Cargo.toml`

APXM crates consumed as **external git dependencies**, not relative paths:

```toml
[workspace.dependencies]
apxm-core     = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
apxm-backends = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
apxm-events   = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
```

The existing `apxm-core` dependency in `core/Cargo.toml` (currently `path = "../../../../apxm/crates/apxm-core"` when Codex lives at `openai/codex/`) is migrated to the workspace git dependency.

### C1.2 Core crate

**File:** `openai/codex/codex-rs/core/Cargo.toml`

New crates added as optional, feature-gated:

```toml
[dependencies]
apxm-core     = { workspace = true }                      # migrate from path dep
apxm-backends = { workspace = true, optional = true }     # NEW
apxm-events   = { workspace = true, optional = true }     # NEW

[features]
apxm-llm = ["dep:apxm-backends", "dep:apxm-events"]
```

Feature-gated so existing builds are unaffected. The existing `apxm_provider_config.rs` continues to work with just `apxm-core`.

---

## C2: Create APXM Adapter (3 days)

### C2.1 Event translator (pure function)

**New file:** `openai/codex/codex-rs/core/src/apxm_adapter/event_translator.rs`

Pure function -- no state, no buffering. Translates `ApxmEvent` to Codex-compatible outputs:

```rust
use apxm_events::{ApxmEvent, EventPayload};

/// Translate ApxmEvent -> Codex internal representation.
/// Pure function. No state. No buffering.
pub fn to_codex_event(event: &ApxmEvent) -> Option<CodexApxmEvent> {
    match &event.payload {
        EventPayload::Token(tok)       => Some(CodexApxmEvent::TextDelta { text: tok.text.clone() }),
        EventPayload::Thought(th)      => Some(CodexApxmEvent::ReasoningDelta { text: th.text.clone() }),
        EventPayload::ToolCall(tc)     => Some(CodexApxmEvent::ToolCallCompleted { /* ... */ }),
        EventPayload::LlmDone(done)    => Some(CodexApxmEvent::ResponseCompleted { /* ... */ }),
        EventPayload::Retry(retry)     => Some(CodexApxmEvent::Retry { /* ... */ }),
        EventPayload::Usage(usage)     => Some(CodexApxmEvent::UsageUpdate { /* ... */ }),
        EventPayload::Warning(warn)    => Some(CodexApxmEvent::Warning { /* ... */ }),
        EventPayload::ContextCompacted(c) => Some(CodexApxmEvent::ContextCompacted { /* ... */ }),
        EventPayload::ModelRerouted(r) => Some(CodexApxmEvent::ModelRerouted { /* ... */ }),
        _ => None,  // Events Codex doesn't consume (runtime, session internals)
    }
}

/// Intermediate event type for Codex consumption.
pub enum CodexApxmEvent {
    TextDelta { text: String },
    ReasoningDelta { text: String },
    ToolCallCompleted { id: String, name: String, arguments: serde_json::Value },
    ResponseCompleted { content: String, model: String, finish_reason: String,
                        usage: CodexTokenUsage, tool_calls: Vec<CodexToolCall>,
                        response_id: Option<String> },
    Retry { attempt: u32, reason: String, retry_after_secs: f64 },
    UsageUpdate { input_tokens: usize, output_tokens: usize },
    Warning { code: String, message: String },
    ContextCompacted { original_tokens: usize, new_tokens: usize },
    ModelRerouted { from: String, to: String, reason: String },
}
```

~120 lines. Pure, unit-testable.

### C2.2 Request translator

**New file:** `openai/codex/codex-rs/core/src/apxm_adapter/request_translator.rs`

Converts Codex `Prompt` to APXM `LLMRequest`. Propagates the Codex turn ID as `trace_id` for cross-process event correlation:

```rust
use apxm_backends::llm::LLMRequest;
use crate::client_common::Prompt;

pub fn prompt_to_llm_request(prompt: &Prompt, config: &ApxmTurnConfig, turn_id: &str) -> LLMRequest {
    LLMRequest {
        messages: items_to_messages(&prompt.input),
        model: config.model.clone(),
        backend: config.backend.clone(),
        temperature: config.temperature,
        max_tokens: config.max_tokens,
        tools: tools_to_definitions(&prompt.tools),
        trace_id: Some(turn_id.to_string()),
        // ... map remaining fields
    }
}
```

~100 lines.

### C2.3 APXM LLM client

**New file:** `openai/codex/codex-rs/core/src/apxm_adapter/client.rs`

Wraps `LLMRegistry` for Codex's session model:

```rust
pub struct ApxmModelClient {
    registry: Arc<LLMRegistry>,
}

impl ApxmModelClient {
    pub async fn from_codex_config(config: &CodexConfig) -> Result<Self> { /* ... */ }

    pub fn stream_turn(
        &self, prompt: &Prompt, config: &ApxmTurnConfig,
    ) -> Pin<Box<dyn Stream<Item = Result<CodexApxmEvent>> + Send + '_>> {
        let request = prompt_to_llm_request(prompt, config);
        let event_stream = self.registry.generate_events(request);
        Box::pin(event_stream.filter_map(|result| async {
            match result {
                Ok(event) => to_codex_event(&event).map(Ok),
                Err(e) => Some(Err(e)),
            }
        }))
    }
}
```

~80 lines.

### C2.4 Module structure

**New file:** `openai/codex/codex-rs/core/src/apxm_adapter/mod.rs`

```rust
mod client;
mod event_translator;
mod request_translator;

pub use client::ApxmModelClient;
pub use event_translator::{to_codex_event, CodexApxmEvent};
pub use request_translator::prompt_to_llm_request;
```

### C2.5 Files created

| File | Lines (est.) |
|------|-------------|
| `core/src/apxm_adapter/mod.rs` | ~10 |
| `core/src/apxm_adapter/event_translator.rs` | ~120 |
| `core/src/apxm_adapter/request_translator.rs` | ~100 |
| `core/src/apxm_adapter/client.rs` | ~80 |
| Tests | ~200 |
| **Total** | **~510** |

---

## C3: Feature-Gated Integration (2 days)

### C3.1 Wire into agent loop

**File:** `openai/codex/codex-rs/core/src/codex.rs` (7321 lines; `submission_loop()` at line ~4138)

Add feature-gated branch:

```rust
#[cfg(feature = "apxm-llm")]
{
    if self.use_apxm_backend {
        let apxm_stream = self.apxm_client.stream_turn(&prompt, &apxm_config);
        // Process CodexApxmEvent stream
        // Map to existing ServerNotification pipeline
        // Handle tool calls through existing GuardianReviewSessionManager + ToolHandler
        return self.process_apxm_stream(apxm_stream).await;
    }
}
// Existing codex-api path (unchanged)
```

### C3.2 Configuration

**File:** `openai/codex/codex-rs/core/src/config/types.rs`

```rust
#[cfg(feature = "apxm-llm")]
#[derive(Deserialize, Default)]
pub struct ApxmBackendConfig {
    pub enabled: bool,
    pub backend: Option<String>,
    pub model: Option<String>,
    pub config_path: Option<String>,  // path to ~/.apxm/config.toml
}
```

### C3.3 Shadow mode validation

During transition, run both paths and compare:

```rust
// Stage 1: Shadow mode -- run both, use legacy
let legacy_result = self.model_client.stream_turn(&prompt).await;
let apxm_result = self.apxm_client.stream_turn(&prompt, &config);
compare_outputs(&legacy_result, &apxm_result);  // log discrepancies
return legacy_result;

// Stage 2: Primary with fallback -- use APXM, fall back to legacy on error
match self.apxm_client.stream_turn(&prompt, &config).await {
    Ok(result) => Ok(result),
    Err(e) => {
        tracing::warn!("APXM path failed, falling back: {e}");
        self.model_client.stream_turn(&prompt).await
    }
}

// Stage 3: Remove legacy
self.apxm_client.stream_turn(&prompt, &config).await
```

#### Divergence metric definition

`compare_outputs()` measures divergence on three axes:

1. **Token-level diff:** `|len(apxm_tokens) - len(legacy_tokens)| / len(legacy_tokens)` -- must be < 0.01 (1%)
2. **Semantic equivalence:** Tool call names match AND argument keys match (values may differ due to LLM non-determinism)
3. **Structural match:** Same number of tool calls in same order

When divergence exceeds threshold, both outputs are logged at `WARN` level with the divergence ratio and a diff of tool call sequences for manual review. Shadow mode runs for 3-4 weeks before progressing to Stage 2.

### C3.4 Files modified

| File | Change | Lines (est.) |
|------|--------|-------------|
| `core/Cargo.toml` | Migrate path dep to workspace git dep, add features | +8 |
| `core/src/lib.rs` | Add `#[cfg(feature = "apxm-llm")] mod apxm_adapter;` | +2 |
| `core/src/codex.rs` | Feature-gated branch in `submission_loop()` | +40 |
| `core/src/config/types.rs` | `ApxmBackendConfig` struct | +15 |

---

## C4: ServerNotification Bridging (1 day)

Map `CodexApxmEvent` to the 49-variant `ServerNotification` enum that TUI/JSON-RPC clients consume:

```rust
fn apxm_event_to_notification(event: CodexApxmEvent) -> Option<ServerNotification> {
    match event {
        CodexApxmEvent::TextDelta { text }
            => Some(ServerNotification::AgentMessageDelta { delta: text }),
        CodexApxmEvent::ReasoningDelta { text }
            => Some(ServerNotification::ReasoningSummaryTextDelta { delta: text }),
        CodexApxmEvent::ToolCallCompleted { id, name, arguments }
            => Some(ServerNotification::ItemStarted {
                   item: build_tool_thread_item(id, name, arguments) }),
        CodexApxmEvent::ResponseCompleted { content, .. }
            => Some(ServerNotification::ItemCompleted {
                   item: build_agent_message_item(content) }),
        CodexApxmEvent::Retry { attempt, reason, .. }
            => Some(ServerNotification::ThreadStatusChanged {
                   status: format!("Retrying (attempt {attempt}, {reason})") }),
        CodexApxmEvent::UsageUpdate { input_tokens, output_tokens }
            => Some(ServerNotification::ThreadTokenUsageUpdated {
                   input_tokens, output_tokens }),
        // ... remaining mappings
        _ => None,
    }
}
```

~60 lines. Lives in the `apxm_adapter` module.

---

## What Is NOT Changed

These Codex systems are untouched in Phase 1:
- **TUI rendering** -- ServerNotification to React/Ink rendering pipeline
- **Thread management** -- ThreadItem, thread lifecycle
- **Hooks system** -- pre/post tool execution hooks
- **Authentication** -- OAuth, API key management (though APXM has its own credential store)
- **IDE integration** -- VS Code extension, CLI interface
- **Sandboxing** -- Platform-native sandboxes (Seatbelt/Landlock) remain unchanged; APXM interceptors wrap but do not replace them
- **Tool execution** -- All 26 ToolHandlers (21 standard + 5 multi_agents) and the ToolRouter remain unchanged (tool migration is Phase 2)
- **Guardian approval** -- GuardianReviewSessionManager is unchanged; tool calls from the APXM path still flow through existing approval
- **Session/Turn state** -- SessionState, TurnState remain unchanged (AAM state mapping is Phase 3)

---

## Phase 1 Summary

| Step | Days | New Files | Modified Files | New Lines (est.) |
|------|------|-----------|----------------|-----------------|
| C1: Dependencies | 0.5 | 0 | 2 | ~15 |
| C2: APXM adapter | 3 | 4 | 0 | ~510 |
| C3: Integration | 2 | 0 | 4 | ~65 |
| C4: Notification bridge | 1 | 0 | 1 | ~60 |
| **Total** | **6.5** | **4** | **7** | **~650** |

### Migration Timeline

```
Weeks 1-6:     Phase 1 -- LLM Backend (C1-C4)
  Week 1:      C1 (deps) + C2 (adapter module)
  Week 2:      C2 (tests) + C3 (feature-gated integration)
  Week 3:      C3 (shadow mode) + C4 (notification bridge)
  Weeks 4-6:   Shadow mode testing, switch to APXM-primary with legacy fallback
```

---

## Related Documents

- [Phase 1 Overview](README.md)
- [Investigation Report](INVESTIGATION.md)
- [APXM Phase 1 (A0-A4)](apxm.md)
- [Gemini-CLI Phase 1 (G1-G5)](gemini-cli.md)
- [Plan 2: Codex Changes (all phases)](../plan-2-codex-changes.md)
- [Global Integration Plan](../plan-global-integration.md)
