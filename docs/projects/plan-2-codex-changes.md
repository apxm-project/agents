# Plan 2: Codex Changes -- Migrating Codex onto the A-PXM Substrate

**Author:** Architecture Team
**Date:** 2026-03-20
**Status:** Draft v6 -- Revised with investigation findings
**Scope:** All changes needed inside `codex/codex-rs/` across all five adoption phases -- from LLM backend swap through tool migration, AAM state model, agent loop as graph, and compiler integration
**Depends on:** Plan 1 (APXM Changes) -- each phase depends on the corresponding Plan 1 deliverables
**Does NOT depend on:** Plan 3 (Gemini-CLI) -- these are independent

---

## Foundational Correction

A-PXM is **not a runtime**. It is a **Program Execution Model** -- a formal specification of how agent programs are represented, optimized, and executed, materialized as a compiler (MLIR-based) and a runtime (dataflow scheduler + AAM). The punch line is the compiler: once Codex expresses its agent loop as an AIS graph, that graph can be compiled, analyzed, and optimized -- FuseAskOps, CSE, DCE, parallelism extraction.

But the compiler requires the runtime, the runtime requires the AAM, and the AAM requires the tools and LLM backends. So we build from the bottom up, across five phases:

```
Phase 1:  LLM Backend        -- Replace LLM transport with apxm-backends
Phase 2:  Tool Migration      -- Register Codex ToolHandlers as APXM capabilities (21 standard in Phase 2, 5 multi_agents in Phase 4)
Phase 3:  AAM State Model     -- Map SessionState/TurnState to AAM (B, G, C)
Phase 4:  Agent Loop as Graph -- Express submission_loop() as AIS graph
Phase 5:  Compiler Integration -- Compile, analyze, and optimize the graph
```

Each phase is independently valuable. Phase 1 alone gives unified streaming. Phase 2 alone gives typed tool dispatch. Phase 3 alone gives inspectable agent state. You do not need to commit to Phase 5 to benefit from Phase 1.

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

### 26 ToolHandler Implementations (21 Standard + 5 Multi-Agent)

Codex routes tool calls through a `ToolRouter` to 26 total `ToolHandler` implementations. The 21 standard handlers are listed below; the 5 `multi_agents/` handlers (spawn, wait, send_input, resume_agent, close_agent) are deferred to Phase 4 as `FLOW_CALL`/`WAIT_ALL` graph operations:

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

## Phase 1: LLM Backend (C1--C4)

**Goal:** Replace Codex's LLM transport layer with `apxm-backends`, preserving streaming UX, approval flow, and multi-provider support.

**Depends on Plan 1:** `apxm-events` crate with 33 event payloads, `apxm-backends` with `StreamAssembler` and 5 LLM backends.

### C1: Add APXM Dependencies (0.5 days)

#### C1.1 Workspace root

**File:** `codex/codex-rs/Cargo.toml`

APXM crates consumed as **external git dependencies**, not relative paths:

```toml
[workspace.dependencies]
apxm-core     = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
apxm-backends = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
apxm-events   = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
```

The existing `apxm-core` dependency in `core/Cargo.toml` (currently `path = "../../../apxm/crates/apxm-core"`) is migrated to the workspace git dependency.

#### C1.2 Core crate

**File:** `codex/codex-rs/core/Cargo.toml`

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

### C2: Create APXM Adapter (3 days)

#### C2.1 Event translator (pure function)

**New file:** `codex/codex-rs/core/src/apxm_adapter/event_translator.rs`

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

#### C2.2 Request translator

**New file:** `codex/codex-rs/core/src/apxm_adapter/request_translator.rs`

Converts Codex `Prompt` to APXM `LLMRequest`:

```rust
use apxm_backends::llm::LLMRequest;
use crate::client_common::Prompt;

pub fn prompt_to_llm_request(prompt: &Prompt, config: &ApxmTurnConfig) -> LLMRequest {
    LLMRequest {
        messages: items_to_messages(&prompt.input),
        model: config.model.clone(),
        backend: config.backend.clone(),
        temperature: config.temperature,
        max_tokens: config.max_tokens,
        tools: tools_to_definitions(&prompt.tools),
        // ... map remaining fields
    }
}
```

~100 lines.

#### C2.3 APXM LLM client

**New file:** `codex/codex-rs/core/src/apxm_adapter/client.rs`

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

#### C2.4 Module structure

**New file:** `codex/codex-rs/core/src/apxm_adapter/mod.rs`

```rust
mod client;
mod event_translator;
mod request_translator;

pub use client::ApxmModelClient;
pub use event_translator::{to_codex_event, CodexApxmEvent};
pub use request_translator::prompt_to_llm_request;
```

#### C2.5 Files created

| File | Lines (est.) |
|------|-------------|
| `core/src/apxm_adapter/mod.rs` | ~10 |
| `core/src/apxm_adapter/event_translator.rs` | ~120 |
| `core/src/apxm_adapter/request_translator.rs` | ~100 |
| `core/src/apxm_adapter/client.rs` | ~80 |
| Tests | ~200 |
| **Total** | **~510** |

### C3: Feature-Gated Integration (2 days)

#### C3.1 Wire into agent loop

**File:** `codex/codex-rs/core/src/codex.rs` (at `submission_loop()`, line ~4138)

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

#### C3.2 Configuration

**File:** `codex/codex-rs/core/src/config/types.rs`

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

#### C3.3 Shadow mode validation

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

#### C3.4 Files modified

| File | Change | Lines (est.) |
|------|--------|-------------|
| `core/Cargo.toml` | Migrate path dep to workspace git dep, add features | +8 |
| `core/src/lib.rs` | Add `#[cfg(feature = "apxm-llm")] mod apxm_adapter;` | +2 |
| `core/src/codex.rs` | Feature-gated branch in `submission_loop()` | +40 |
| `core/src/config/types.rs` | `ApxmBackendConfig` struct | +15 |

### C4: ServerNotification Bridging (1 day)

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

### Phase 1 Summary

| Step | Days | New Files | Modified Files | New Lines (est.) |
|------|------|-----------|----------------|-----------------|
| C1: Dependencies | 0.5 | 0 | 2 | ~15 |
| C2: APXM adapter | 3 | 4 | 0 | ~510 |
| C3: Integration | 2 | 0 | 4 | ~65 |
| C4: Notification bridge | 1 | 0 | 1 | ~60 |
| **Total** | **6.5** | **4** | **7** | **~650** |

---

## Phase 2: Tool Migration (C5--C7)

**Goal:** Register all 21 standard Codex `ToolHandler` implementations as APXM `CapabilityExecutor` instances. (The 5 `multi_agents` handlers are deferred to Phase 4 as `FLOW_CALL`/`WAIT_ALL` graph operations -- 26 total.) Tool approval flows through APXM's interceptor pipeline instead of direct `GuardianReviewSessionManager` calls.

**Depends on Plan 1:** `CapabilitySystem` with interceptor pipeline, `CapabilityExecutor` trait, `~/.apxm/tools.toml` persistence.

### C5: ToolHandler to CapabilityExecutor Adapters (3 days)

Each of Codex's 21 standard `ToolHandler` implementations gets a thin adapter that wraps the existing handler as an APXM `CapabilityExecutor`:

```rust
use apxm_runtime::capability::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::CapabilityMetadata,
};
use apxm_core::types::values::Value;
use async_trait::async_trait;

/// Thin adapter: wraps an existing Codex ToolHandler as an APXM capability.
pub struct ShellToolAdapter {
    handler: ShellHandler,
    metadata: CapabilityMetadata,
}

#[async_trait]
impl CapabilityExecutor for ShellToolAdapter {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        // Convert APXM Value args -> JSON string (ToolHandler expects &str arguments)
        let arguments = serde_json::to_string(&args)?;
        // Delegate to existing handler
        let result = self.handler.handle(/* invocation */).await;
        // Convert ToolOutput -> APXM Value
        Ok(tool_output_to_value(result))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}
```

Each adapter is ~20 lines of boilerplate. A `register_codex_tools()` function registers all 21:

```rust
pub fn register_codex_tools(
    system: &CapabilitySystem,
    session: &Session,
) -> Result<(), RuntimeError> {
    system.register(Arc::new(ShellToolAdapter::new(&session)))?;
    system.register(Arc::new(ReadFileAdapter::new(&session)))?;
    system.register(Arc::new(ApplyPatchAdapter::new(&session)))?;
    system.register(Arc::new(McpAdapter::new(&session)))?;
    // ... remaining 17 handlers
    Ok(())
}
```

**Deliverables:**
- `core/src/apxm_adapter/tool_adapters.rs` -- 21 standard adapter structs (~420 lines; 5 multi_agents handlers deferred to Phase 4 as FLOW_CALL/WAIT_ALL)
- `core/src/apxm_adapter/tool_registration.rs` -- registration function (~80 lines)
- Unit tests verifying round-trip conversion for each handler (~300 lines)

### C6: Approval Flow through Interceptor Pipeline (2 days)

Replace direct `GuardianReviewSessionManager` calls with an APXM `CapabilityInterceptor` that delegates to the existing guardian:

```rust
pub struct GuardianInterceptor {
    guardian: Arc<GuardianReviewSessionManager>,
}

#[async_trait]
impl CapabilityInterceptor for GuardianInterceptor {
    fn name(&self) -> &str { "codex-guardian" }

    async fn pre_invoke(
        &self, capability: &str, args: &HashMap<String, Value>,
    ) -> InterceptDecision {
        let verdict = self.guardian.review(capability, args).await;
        match verdict {
            Verdict::Allow => InterceptDecision::Allow,
            Verdict::Deny(reason) => InterceptDecision::Deny(reason),
            // Escalation handled via ApprovalChannel (no Escalate variant exists)
            Verdict::RequireUserApproval => InterceptDecision::Deny {
                reason: "Guardian requires user approval".into(),
            },
        }
    }
}
```

The VERIFY AIS operation maps to this interceptor: the guardian's risk score IS the verification evidence, and its verdict IS the VERIFY output.

**Deliverables:**
- `core/src/apxm_adapter/guardian_interceptor.rs` -- interceptor implementation (~60 lines)
- Integration with `CapabilitySystem::register_interceptor()` during session init

### C7: Tool Persistence and MCP Registration (1 day)

- Codex's 26 tool registrations (21 standard + 5 multi_agents) persist to `~/.apxm/tools.toml` so they are visible to `apxm tools list`. The 5 multi_agents handlers (spawn, wait, send_input, resume_agent, close_agent) are deferred to Phase 4 as `FLOW_CALL`/`WAIT_ALL` graph operations.
- MCP tools discovered by `McpHandler` and `McpResourceHandler` are registered as APXM MCP capabilities
- `apxm tools list --json` includes Codex-registered tools

**Deliverables:**
- Startup code writes Codex tool metadata to `~/.apxm/tools.toml`
- MCP-discovered tools registered via `CapabilitySystem::register()`
- Integration test: `apxm tools list` shows all Codex tools

### Phase 2 Summary

| Step | Days | New Files | Modified Files | New Lines (est.) |
|------|------|-----------|----------------|-----------------|
| C5: Tool adapters | 3 | 2 | 1 | ~800 |
| C6: Guardian interceptor | 2 | 1 | 1 | ~120 |
| C7: Tool persistence | 1 | 0 | 2 | ~100 |
| **Total** | **6** | **3** | **4** | **~1020** |

---

## Phase 3: AAM State Model (C8--C9)

**Goal:** Map Codex's `SessionState` and `TurnState` to the AAM triple `(B, G, C)`, backed by the file tree under `~/.apxm/workspaces/`. Agent state becomes inspectable, diffable, and persistent.

**Depends on Plan 1:** Hierarchical AAM with scoped state, file-backed persistence, `apxm state show` CLI command.

### C8: State Mapping (3 days)

Map Codex's existing state structures to AAM components:

| Codex State | AAM Component | File Representation |
|-------------|--------------|---------------------|
| Conversation history | **B** (Beliefs) | `data/conversation.json` |
| Current model/provider | **C** (Capabilities) | `tools/llm.toml` |
| Pending tool calls | **B** (Beliefs) | `data/pending_tools.json` |
| Approval decisions | **B** (Beliefs) | `data/approvals.json` |
| Turn count | **B** (Beliefs) | `data/session_state.json` |
| Active instructions | **G** (Goals) | `goals.toml` |
| Available tools | **C** (Capabilities) | `tools/*.toml` |
| System prompt | **B** (Beliefs) | `data/system_prompt.md` |

**The file tree IS the AAM:**

```
~/.apxm/workspaces/<codex-session-id>/
+-- data/                              B (Beliefs)
|   +-- conversation.json              - Chat history
|   +-- pending_tools.json             - In-flight tool calls
|   +-- approvals.json                 - Cached approval decisions
|   +-- session_state.json             - Turn count, model config, etc.
+-- goals.toml                         G (Goals)
|   goal = "Fix the failing test"
|   priority = 1
+-- tools/                             C (Capabilities)
    +-- shell.toml                     - Shell execution capability
    +-- read_file.toml                 - File read capability
    +-- apply_patch.toml               - File mutation capability
    +-- mcp/                           - MCP-discovered tools
        +-- filesystem.toml
```

**Implementation approach:**

1. At session start, create a workspace directory under `~/.apxm/workspaces/` keyed by Codex session ID
2. Write initial AAM state: beliefs from config, goals from instructions, capabilities from tool registry
3. On each state mutation (`SessionState` or `TurnState` update), write the corresponding file
4. On session resume, load AAM state from the workspace directory

**Deliverables:**
- `core/src/apxm_adapter/state_bridge.rs` -- bidirectional `SessionState` <-> AAM mapping (~200 lines)
- Workspace lifecycle management (create, populate, read) (~100 lines)
- `apxm state show <session-id>` works for Codex sessions

### C9: Memory Tier Integration (2 days)

Map Codex's context compaction to APXM's three-tier memory:

| Codex Memory Concept | APXM Memory Tier |
|---------------------|-----------------|
| Current turn working context | **STM** -- in-memory per-execution scratch |
| Session memories (persisted across turns) | **Episodic** -- append-only event log |
| Persistent memories (cross-session) | **LTM** -- SQLite persistence at `~/.apxm/memory/ltm.sqlite` |
| Context compaction (2-phase) | **QMEM** (query) + **UMEM** (update) on memory tiers |

Codex's 2-phase context compaction pipeline (extract with gpt-5.1-codex-mini, consolidate with gpt-5.3-codex) maps to QMEM + UMEM nodes in the AIS graph, making the compaction visible to the compiler as a first-class operation rather than an opaque function call.

**Deliverables:**
- STM populated from `TurnState` at turn start
- LTM bridge for persistent memories across sessions
- Episodic memory populated from conversation history
- QMEM/UMEM operations exposed for context compaction

### Phase 3 Summary

| Step | Days | New Files | Modified Files | New Lines (est.) |
|------|------|-----------|----------------|-----------------|
| C8: State mapping | 3 | 2 | 2 | ~400 |
| C9: Memory tiers | 2 | 1 | 1 | ~200 |
| **Total** | **5** | **3** | **3** | **~600** |

---

## Phase 4: Agent Loop as Graph (C10--C11)

**Goal:** Express Codex's `submission_loop()` as an AIS graph instead of imperative Rust code. The graph enables automatic parallelism, compile-time validation, and sets up Phase 5.

**Depends on Plan 1:** AIS graph authoring and validation, dataflow scheduler, FLOW_CALL/WAIT_ALL operations.

### C10: submission_loop() as AIS Graph (4 days)

The core Codex turn loop (`submission_loop()` at `codex.rs:4138`) follows a repeating pattern:

1. Build prompt from conversation history
2. Call LLM
3. If tool calls in response, execute them (potentially in parallel)
4. Wait for all tool results
5. Guardian reviews tool results
6. Save results to memory
7. Loop back to step 1 (with updated context)

This maps to an AIS graph:

```
                                  +----------+
                     +----------->| INV      |------+
                     |            | tool_a   |      |
+-------+  +--------+            +----------+      |    +-----------+   +---------+
| ASK   |->|BRANCH_ |                              +--->| WAIT_ALL  |-->| VERIFY  |
| prompt |  |ON_VALUE|            +----------+      |    |           |   | approval|
+-------+  | calls  +----------->| INV      |------+    +-----------+   +----+----+
            +--------+            | tool_b   |                               |
                     |            +----------+                               v
                     |                                                 +---------+
                     +------------------------------------------------>| UMEM    |
                                (no tool calls -- direct response)     | save    |
                                                                       +---------+
```

Key properties of this graph:
- The N `INV` nodes for tool calls share no data edges -- the scheduler runs them **concurrently** with zero developer effort (replacing Codex's manual `FuturesOrdered` + per-tool `RwLock`)
- `VERIFY` runs after `WAIT_ALL` -- tool approval happens only after all tools complete
- `UMEM` saves results regardless of branch taken
- The compiler can validate structural correctness: no tool execution without approval, no approval of a tool that was never called

**Multi-agent coordination:**

Codex's multi-agent system (`spawn.rs`, `wait.rs`, `resume_agent.rs`, `close_agent.rs`, `send_input.rs`) maps to FLOW_CALL and WAIT_ALL:

```
+-------+     +-----------+     +-----------+     +-----------+
| PLAN  |---->| FLOW_CALL |---->| FLOW_CALL |---->| WAIT_ALL  |---> MERGE
| decomp|     | agent_a   |     | agent_b   |     | all agents|
+-------+     +-----------+     +-----------+     +-----------+
```

**Deliverables:**
- `codex-turn.json` -- AIS graph representing a single Codex turn
- Graph authoring code that emits the turn graph from Codex's configuration
- Integration with APXM's dataflow scheduler for execution
- Validation that graph-based execution produces identical outputs to imperative code

### C11: Context Compaction as Graph Nodes (1 day)

Codex's 2-phase context compaction becomes visible in the graph:

```
+---------+     +---------+     +---------+
| QMEM    |---->| ASK     |---->| UMEM    |
| load    |     | compact |     | save    |
| history |     | context |     | summary |
+---------+     +---------+     +---------+
```

The compiler sees this subgraph and can:
- Fuse the QMEM + ASK if the query is a simple lookup
- Schedule compaction in parallel with other non-dependent operations
- Validate that compaction writes do not conflict with concurrent reads

**Deliverables:**
- Context compaction expressed as QMEM + ASK + UMEM subgraph
- Integration with existing compaction triggers

### Phase 4 Summary

| Step | Days | New Files | Modified Files | New Lines (est.) |
|------|------|-----------|----------------|-----------------|
| C10: Turn graph | 4 | 2 | 2 | ~500 |
| C11: Compaction graph | 1 | 0 | 1 | ~80 |
| **Total** | **5** | **2** | **3** | **~580** |

---

## Phase 5: Compiler Integration (C12)

**Goal:** Compile the Codex turn graph, run optimization passes, and measure improvement. This is the punch line -- the reason everything else was built.

**Depends on Plan 1:** MLIR-based compiler with 12 passes (at O2), including `fuse-ask-ops`, `cse`, `symbol-dce`, `.apxmobj` artifact format. See Plan 1 A8 for optimization levels and full pass list.

### C12: Compile and Optimize (ongoing)

Once `codex-turn.json` exists (Phase 4), the compiler becomes available:

```bash
apxm compile codex-turn.json -o codex-turn.apxmobj -O2
```

**What the compiler provides:**

| Optimization | What It Does | Expected Impact |
|-------------|-------------|-----------------|
| **fuse-ask-ops** | Merges producer-consumer ASK chains into single API calls | Fewer API calls, lower latency |
| **cse** | Eliminates duplicate LLM calls with identical inputs | Saves dollars and seconds |
| **symbol-dce** | Removes operations whose outputs are never consumed | Leaner graphs |
| **Parallelism extraction** | Infers concurrency from DAG structure | Automatic tool parallelism |
| **Compile-time validation** | Catches structural errors before any LLM call | 49x faster error detection |

**Structural errors caught at compile time:**
- Tool execution without approval (INV node with no VERIFY dependency) -- **implemented** (structural graph validation)
- Approval of a tool that was never called (VERIFY with no INV predecessor) -- **implemented**
- Unreachable operations (symbol-dce detects dead nodes) -- **implemented**
- Cycles in DAG structure -- **implemented** (Kahn's algorithm)
- Type mismatches between connected nodes -- **NOT YET IMPLEMENTED** (prerequisite for Phase 5)
- Missing required attributes on operations -- **NOT YET IMPLEMENTED** (prerequisite for Phase 5)

**Key insight:** Instead of a simple plan, Codex creates an **apxm-graph that gets compiled and analyzed**. The graph IS the plan, and the compiler IS the analysis engine.

**Deliverables:**
- `apxm compile codex-turn.json` produces optimized `.apxmobj`
- Measured reduction in API calls from FuseAskOps on real Codex workflows
- Measured compile-time error detection rate vs. runtime error detection
- `apxm decompile codex-turn.apxmobj` shows optimized graph structure

### Phase 5 Summary

| Step | Days | New Files | Modified Files | New Lines (est.) |
|------|------|-----------|----------------|-----------------|
| C12: Compile + optimize | 3+ | 1 | 1 | ~200 |
| **Total** | **3+** | **1** | **1** | **~200** |

---

## What Is NOT Changed

These Codex systems are untouched across all phases:
- **TUI rendering** -- ServerNotification to React/Ink rendering pipeline
- **Thread management** -- ThreadItem, thread lifecycle
- **Hooks system** -- pre/post tool execution hooks
- **Authentication** -- OAuth, API key management (though APXM has its own credential store)
- **IDE integration** -- VS Code extension, CLI interface
- **Sandboxing** -- Platform-native sandboxes (Seatbelt/Landlock) remain unchanged; APXM interceptors wrap but do not replace them

---

## All-Phase Summary

| Phase | Steps | Days | New Files | Modified Files | New Lines (est.) |
|-------|-------|------|-----------|----------------|-----------------|
| 1: LLM Backend | C1--C4 | 6.5 | 4 | 7 | ~650 |
| 2: Tool Migration | C5--C7 | 6 | 3 | 4 | ~1020 |
| 3: AAM State Model | C8--C9 | 5 | 3 | 3 | ~600 |
| 4: Agent Loop as Graph | C10--C11 | 5 | 2 | 3 | ~580 |
| 5: Compiler Integration | C12 | 3+ | 1 | 1 | ~200 |
| **Total** | **C1--C12** | **~25.5** | **13** | **18** | **~3050** |

---

## Migration Timeline

```
Weeks 1-6:     Phase 1 -- LLM Backend (C1-C4)
  Week 1:      C1 (deps) + C2 (adapter module)
  Week 2:      C2 (tests) + C3 (feature-gated integration)
  Week 3:      C3 (shadow mode) + C4 (notification bridge)
  Weeks 4-6:   Shadow mode testing, switch to APXM-primary with legacy fallback

Weeks 7-12:    Phase 2 -- Tool Migration (C5-C7)
  Weeks 7-9:   C5 (26 tool adapters: 21 standard + 5 multi_agents)
  Weeks 10-11: C6 (guardian interceptor)
  Week 12:     C7 (tool persistence, MCP registration)

Weeks 13-17:   Phase 3 -- AAM State Model (C8-C9)
  Weeks 13-15: C8 (state mapping, workspace lifecycle)
  Weeks 16-17: C9 (memory tier integration)

Weeks 18-22:   Phase 4 -- Agent Loop as Graph (C10-C11)
  Weeks 18-21: C10 (submission_loop() as AIS graph)
  Week 22:     C11 (context compaction as graph nodes)

Weeks 23+:     Phase 5 -- Compiler Integration (C12)
  Ongoing:     Compile, measure, iterate
```

---

## Risk Analysis

| Risk | Probability | Impact | Phase | Mitigation |
|------|------------|--------|-------|------------|
| Git dependency resolution for APXM crates | Medium | High | 1 | Start with git deps, publish to crates.io later |
| Event type mismatch between ApxmEvent and ResponseEvent | Low | Medium | 1 | Shadow mode comparison catches divergence |
| Tool adapter boilerplate for 26 handlers (21 standard + 5 multi_agents) | Medium | Low | 2 | Macro-based code generation for adapter trait; 5 multi_agents handlers deferred to Phase 4 |
| GuardianReviewSessionManager coupling to interceptor | Medium | Medium | 2 | Interceptor wraps guardian, does not replace it |
| SessionState to AAM mapping complexity | Medium | Medium | 3 | Incremental: start with conversation history, add fields |
| Hierarchical AAM not yet fully implemented | High | Medium | 3 | Start with flat AAM (already works), add hierarchy later |
| submission_loop() too complex for single graph | Medium | High | 4 | Decompose into subgraphs (turn graph + compaction graph + multi-agent graph) |
| MLIR dependency for compiler | Low | Medium | 5 | Compiler is optional; Phases 1-4 use runtime only |
| Performance regression during migration | Low | High | All | Shadow mode comparison at each phase boundary |

---

## Validation Criteria

### Phase 1 Complete:
- [ ] `cargo build -p codex-core --features apxm-llm` compiles
- [ ] `cargo build -p codex-core` (without feature) still compiles -- no regressions
- [ ] APXM deps use git URLs, not relative paths
- [ ] Event translator unit tests: each `CodexApxmEvent` maps to correct `ServerNotification`
- [ ] Request translator: Codex `Prompt` to `LLMRequest` round-trip preserves all fields
- [ ] Shadow mode: <1% divergence between legacy and APXM paths on same prompts
- [ ] Streaming UX: TUI renders equivalent output from both paths
- [ ] Tool calls: approval flow works identically through APXM path
- [ ] Retry: APXM retry events surface as `ThreadStatusChanged` in TUI

### Phase 2 Complete:
- [ ] All 21 standard Codex `ToolHandler` implementations registered as APXM `CapabilityExecutor` (5 multi_agents deferred to Phase 4)
- [ ] `apxm tools list` shows all Codex-registered tools
- [ ] Guardian approval flows through APXM interceptor pipeline
- [ ] MCP-discovered tools registered as APXM capabilities
- [ ] `~/.apxm/tools.toml` persists Codex tool registrations
- [ ] Tool execution produces identical results through APXM capability system

### Phase 3 Complete:
- [ ] Agent state inspectable: `apxm state show <session-id>` for Codex sessions
- [ ] AAM (B, G, C) backed by file tree under `~/.apxm/workspaces/`
- [ ] State survives session restarts (load from workspace directory)
- [ ] Three-tier memory operational (STM, LTM, Episodic)
- [ ] Context compaction uses QMEM/UMEM operations

### Phase 4 Complete:
- [ ] `codex-turn.json` -- valid AIS graph representing a Codex turn
- [ ] `apxm validate codex-turn.json` passes
- [ ] Graph-based execution produces identical outputs to imperative `submission_loop()`
- [ ] Multi-agent spawn/wait expressed as FLOW_CALL/WAIT_ALL
- [ ] Automatic parallelism demonstrated on concurrent tool calls

### Phase 5 Complete:
- [ ] `apxm compile codex-turn.json` produces optimized `.apxmobj`
- [ ] FuseAskOps reduces API calls on real Codex workflows
- [ ] Compile-time validation catches structural errors before any LLM call
- [ ] Measured improvement reported (latency, API calls, error detection speed)

---

## Related Documents

- [Global Integration Plan](plan-global-integration.md) -- five-phase adoption path for all consumers
- [Plan 1: APXM Changes](plan-1-apxm-changes.md) -- APXM-side work this plan depends on
- [Plan 3: Gemini-CLI Changes](plan-3-gemini-changes.md) -- independent parallel effort
- [Case Study: Codex on A-PXM](codex/case-study.md) -- the LLVM-GCC parallel
- [Codex-on-APXM Implementation Plan](codex/plan.md) -- 5-phase reconstruction plan
- [AAM: Agent Abstract Machine](../pxm/aam.md) -- formal state model `(B, G, C)`
- [Vision: The LLVM for Agents](../pxm/vision.md) -- file tree as AAM, compiler as punch line
- [PXM Foundations](../pxm/foundations.md) -- why agent workflows need a formal execution model
- [AIS Operations](../pxm/ais.md) -- 39 typed operations
