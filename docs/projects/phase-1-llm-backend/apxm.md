# Phase 1: APXM Changes -- LLM Backend

**Draft v6 -- Revised with investigation findings**

**Timeline:** Weeks 1-8 (11.5 days estimated)
**Depends on:** Nothing (this is the foundation)
**Blocks:** [Codex Phase 1](codex.md), [Gemini-CLI Phase 1](gemini-cli.md)
**Global plan:** [Plan 1: APXM Changes (all phases)](../plan-1-apxm-changes.md)

---

## Phase A0: Packaging & External Dependency (1 day)

### Why This Comes First

Current plans reference APXM crates via relative paths (`../../../apxm/crates/apxm-core`). This is fragile, non-portable, and assumes a specific monorepo directory layout. Both Codex and Gemini-CLI must be able to depend on APXM as an external package.

### A0.1 Version tagging

Tag the APXM workspace with a semver version:

```bash
# After Phase A1-A4 are complete, tag the first consumer-ready release
git tag v0.1.0
```

### A0.2 Git dependencies for Rust consumers (Codex)

Codex's `Cargo.toml` references APXM crates via git:

```toml
[workspace.dependencies]
apxm-core     = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
apxm-backends = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
apxm-events   = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
apxm-runtime  = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
```

No relative paths. No monorepo coupling. Version-tracked.

### A0.3 Binary installation for non-Rust consumers (Gemini-CLI)

APXM installs to `~/.apxm/` (already used for credentials):

```
~/.apxm/
+-- bin/
|   +-- apxm                     # CLI (compiler + runtime)
|   +-- apxm-server              # HTTP+SSE LLM service (Phase A3)
+-- config.toml                   # Backend configuration
+-- credentials.toml              # LLM provider credentials (0600)
+-- tools.toml                    # Registered tools/capabilities (Phase A5)
+-- memory/                       # Default memory tier storage
|   +-- ltm.sqlite
|   +-- episodes.jsonl
+-- workspaces/                   # Hierarchical AAM state (Phase A6)
    +-- <workspace-id>/
        +-- scope.toml
        +-- data/                 # B (Beliefs)
        +-- goals/                # G (Goals)
        +-- tools/                # C (Capabilities)
```

Installation via `cargo install`:

```bash
cargo install apxm-cli --git https://github.com/user/apxm --tag v0.1.0
# Installs `apxm` binary; also builds `apxm-server` if feature is enabled
```

### A0.4 `ApxmPaths` discovery utility

**File:** `apxm-core/src/paths.rs` (already exists)

Ensure `ApxmPaths::discover()` returns the `~/.apxm/` path and validates that required binaries and configuration exist. Consumers call this at startup:

```rust
let apxm_home = apxm_core::paths::ApxmPaths::discover()
    .unwrap_or_else(|_| panic!("APXM not installed. Run: cargo install apxm-cli"));
```

### A0.5 Files modified

| File | Change |
|------|--------|
| `apxm/Cargo.toml` | Add `version = "0.1.0"` to workspace |
| `apxm-core/src/paths.rs` | Ensure `discover()` validates `~/.apxm/bin/` |
| `apxm-cli/Cargo.toml` | Add `[[bin]]` entry for `apxm-server` (feature-gated) |

---

## Phase A1: Create `apxm-events` crate (3 days)

### Current State: Events are Inline

As of March 2026, APXM does NOT have a separate `apxm-events` crate. Event types live inline in `apxm-runtime/src/executor/events.rs` -- tightly coupled to the runtime executor. The `ExecutionEventEmitter` trait and `ExecutionEvent` enum (16 variants: `LlmToken`, `ToolStart`, `ToolEnd`, `OperationStart`, `OperationEnd`, `PlanCreated`, `PlanStepStarted`, `PlanStepCompleted`, `MemoryRead`, `MemoryWrite`, `CheckpointSaved`, `CheckpointRestored`, `SchedulerDecision`, `GpuUtilization`, `TokenUsage`, `MemoizationHit`) are runtime-internal and not designed for external consumption.

This phase extracts and expands those types into a standalone crate that both the runtime and external consumers (Codex, Gemini-CLI) can depend on without pulling in the full runtime.

### A1.1 Crate scaffold

**New directory:** `apxm/crates/apxm-events/`

```
apxm-events/
  Cargo.toml
  src/
    lib.rs           -- ApxmEvent, EventMeta, EventSource, EventPayload enum
    llm/
      mod.rs         -- LLM event structs (Token, Thought, ToolCall, LlmDone, etc.)
    runtime/
      mod.rs         -- Runtime event structs (OperationStart/End, ToolStart/End, etc.)
    session/
      mod.rs         -- Session event structs (TurnStart/End, Error, Approval, etc.)
    emitter.rs       -- EventBus (tokio::broadcast), EventEmitter trait, BusEmitter
    filter.rs        -- EventFilter convenience predicates
```

**Cargo.toml:**
```toml
[package]
name = "apxm-events"
version.workspace = true
edition.workspace = true
description = "APXM unified event types for LLM streaming, runtime, and session events"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1.0", features = ["sync"] }
uuid = { version = "1.0", features = ["v4"] }

[features]
schema = ["dep:schemars"]

[dependencies.schemars]
version = "0.8"
optional = true
```

**Workspace update:** Add `"crates/apxm-events"` to `apxm/Cargo.toml` `[workspace] members`.

### A1.2 Core types

**`lib.rs`** -- Envelope + payload enum:

```rust
pub mod llm;
pub mod runtime;
pub mod session;
pub mod emitter;
pub mod filter;

/// Universal event envelope. Every APXM event uses this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApxmEvent {
    pub meta: EventMeta,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMeta {
    pub seq: u64,
    pub timestamp: std::time::SystemTime,
    pub trace_id: String,
    pub source: EventSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSource {
    pub component: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
}

/// Master payload enum -- 33 variants across 3 layers.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventPayload {
    // Layer 1: LLM (9 variants)
    Token(llm::TokenEvent),
    Thought(llm::ThoughtEvent),
    ToolCall(llm::ToolCallEvent),
    LlmDone(llm::LlmDoneEvent),
    Usage(llm::UsageEvent),
    Retry(llm::RetryEvent),
    Warning(llm::WarningEvent),
    Citation(llm::CitationEvent),
    ProviderEvent(llm::ProviderEventData),

    // Layer 2: Runtime (16 variants)
    OperationStart(runtime::OperationStartEvent),
    OperationEnd(runtime::OperationEndEvent),
    ToolStart(runtime::ToolStartEvent),
    ToolEnd(runtime::ToolEndEvent),
    PlanCreated(runtime::PlanCreatedEvent),
    PlanStepStarted(runtime::PlanStepStartedEvent),
    PlanStepCompleted(runtime::PlanStepCompletedEvent),
    MemoryRead(runtime::MemoryAccessEvent),
    MemoryWrite(runtime::MemoryAccessEvent),
    CheckpointSaved(runtime::CheckpointEvent),
    CheckpointRestored(runtime::CheckpointEvent),
    SchedulerDecision(runtime::SchedulerDecisionEvent),
    NodeTokenUsage(runtime::NodeTokenUsageEvent),
    MemoizationHit(runtime::MemoizationHitEvent),
    ContextCompacted(runtime::ContextCompactedEvent),
    ModelRerouted(runtime::ModelReroutedEvent),

    // Layer 3: Session (8 variants)
    TurnStart(session::TurnStartEvent),
    TurnEnd(session::TurnEndEvent),
    Error(session::ErrorEvent),
    ApprovalRequested(session::ApprovalRequestedEvent),
    ApprovalDecision(session::ApprovalDecisionEvent),
    Cancelled(session::CancelledEvent),
    LoopDetected(session::LoopDetectedEvent),
    ContextWindowWarning(session::ContextWindowWarningEvent),
}
```

The three layers map to the adoption phases:
- **LLM (9)**: Consumed in Phase 1 by Codex/Gemini-CLI adapters
- **Runtime (16)**: Bridged in Phase 1 (A4), fully utilized in Phases 4-5
- **Session (8)**: Enables Codex's approval flow and Gemini-CLI's loop detection as typed events

### A1.3 LLM event structs (`llm/mod.rs`)

All 9 structs fully typed. Key types:

- `TokenEvent { text: String }`
- `ThoughtEvent { text: String, summary: Option<ThoughtSummary> }`
- `ToolCallEvent { id: String, name: String, arguments: serde_json::Value }` -- **complete, assembled**
- `LlmDoneEvent` -- 13 fields including `content`, `model`, `finish_reason: FinishReasonInfo`, `usage: TokenUsageInfo`, `tool_calls: Vec<ToolCallInfo>`, `thinking: Option<String>`, `response_id: Option<String>`, `citations`, `safety_ratings`, `provider_metadata`
- `UsageEvent { input_tokens, output_tokens, cached_tokens, total_tokens }`
- `RetryEvent { attempt, max_attempts, retry_after_secs, reason }`
- `WarningEvent { code, message }`
- `CitationEvent { citations: Vec<CitationInfo> }`
- `ProviderEventData { provider, event_type, data: Value }`

Supporting types: `ThoughtSummary`, `ToolCallInfo`, `TokenUsageInfo`, `CitationInfo`, `SafetyRatingInfo`, `FinishReasonInfo` (String-based with constructors).

### A1.4 Runtime event structs (`runtime/mod.rs`)

16 structs covering operation lifecycle, planning, memory, checkpoints, scheduling, and accounting. These correspond to the current `ExecutionEvent` variants in `apxm-runtime/src/executor/events.rs` but with richer metadata fields and the `ApxmEvent` envelope.

### A1.5 Session event structs (`session/mod.rs`)

8 structs: `TurnStartEvent`, `TurnEndEvent`, `ErrorEvent`, `ApprovalRequestedEvent`, `ApprovalDecisionEvent`, `CancelledEvent`, `LoopDetectedEvent`, `ContextWindowWarningEvent`.

### A1.6 EventBus + EventEmitter (`emitter.rs`)

- `EventBus` backed by `tokio::broadcast::channel`
- `EventEmitter` trait: `fn emit(&self, event: ApxmEvent)`
- `BusEmitter`: cloneable handle implementing `EventEmitter`
- `FilteredReceiver`: wraps broadcast receiver with predicate

### A1.7 Tests

- Serde round-trip for every `EventPayload` variant (33 tests)
- `kind_str()` consistency test
- EventBus multi-producer/multi-consumer test
- FilteredReceiver test

### A1.8 Files created

| File | Lines (est.) |
|------|-------------|
| `apxm-events/Cargo.toml` | ~25 |
| `apxm-events/src/lib.rs` | ~120 |
| `apxm-events/src/llm/mod.rs` | ~200 |
| `apxm-events/src/runtime/mod.rs` | ~120 |
| `apxm-events/src/session/mod.rs` | ~100 |
| `apxm-events/src/emitter.rs` | ~80 |
| `apxm-events/src/filter.rs` | ~60 |
| Tests | ~300 |
| **Total** | **~1005** |

### A1.9 Files modified

| File | Change |
|------|--------|
| `apxm/Cargo.toml` | Add `"crates/apxm-events"` to workspace members |

### A1.10 Forward compatibility decisions

| Decision | Why It Matters |
|----------|---------------|
| `#[non_exhaustive]` on `EventPayload` | New AIS operation events can be added without breaking consumer adapters |
| `EventMeta` includes `trace_id` | Correlates events across process boundaries (Gemini-CLI -> apxm-server -> backend) |
| `EventBus` uses `tokio::broadcast` | Multiple consumers (TUI, metrics, telemetry, scheduler) is the Phase 4+ normal case |
| `StreamAssembler` is internal to `apxm-backends` | Can be replaced with a graph-aware assembler in Phase 4 without changing consumer APIs |
| Runtime events share `EventPayload` enum | One vocabulary, one bus, one schema across all phases |

---

## Phase A2: Upgrade `apxm-backends` (4 days)

### A2.1 Add `apxm-events` dependency

**File:** `apxm-backends/Cargo.toml`
```toml
apxm-events = { version = "0.0.1", path = "../apxm-events" }
```

### A2.2 Extend `StreamChunk` (internal, backwards-compatible)

**File:** `apxm-backends/src/llm/backends/traits.rs`

Add 3 new variants to the existing 4:

```rust
pub enum StreamChunk {
    Token(String),                                       // existing
    ToolCallStart { id: String, name: String },          // existing
    ToolCallDelta { id: String, arguments_delta: String },// existing
    Done(LLMResponse),                                   // existing
    // NEW
    Thought { text: String },
    Usage { input_tokens: usize, output_tokens: usize, total_tokens: usize },
    RetryNotice { attempt: u32, max_attempts: u32, retry_after_secs: f64, reason: String },
}
```

These new variants let backend implementors emit richer events. `StreamChunk` stays `pub(crate)` -- never exposed to consumers.

### A2.3 Create `StreamAssembler` (internal)

**New file:** `apxm-backends/src/llm/assembler.rs`

Internal component that converts `StreamChunk` -> `ApxmEvent`:
- Accumulates `ToolCallStart` + `ToolCallDelta` fragments -> emits single `ToolCallEvent` on `Done`
- Assigns monotonic sequence numbers
- Attaches `EventMeta` (trace ID, source, timestamp)
- Converts `LLMResponse` -> `LlmDoneEvent` with full type mapping

~120 lines.

### A2.4 Add `LLMRegistry::generate_events()` method (NEW)

**File:** `apxm-backends/src/llm/registry/mod.rs`

This is an **entirely new method** on `LLMRegistry`. There is NO existing `generate_stream()` on `LLMRegistry` -- only `generate()` (non-streaming, line 270) and `resolve_backend_for_streaming()` (line 380). The `generate_stream()` method exists on the `LLMBackend` **trait**, and only `MockBackend` overrides the default single-`Done`-chunk implementation. `generate_events()` bridges the registry to the new `StreamAssembler`:

```rust
impl LLMRegistry {
    pub fn generate_events(
        &self,
        request: LLMRequest,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<ApxmEvent>> + Send + '_>> {
        // 1. Resolve backend
        // 2. Create StreamAssembler for that backend
        // 3. Call backend.generate_stream()
        // 4. flat_map each StreamChunk through assembler.process()
    }
}
```

### Current streaming status

**No production backend has real streaming today.** All four backends (OpenAI, Anthropic, Google, Ollama) use the default `generate_stream()` implementation inherited from the `LLMBackend` trait, which wraps `generate()` into a single `Done` chunk. Only `MockBackend` overrides this default with word-level token streaming (line 270 of `backends/mock.rs`). Sections A2.5-A2.7 below are therefore **greenfield SSE parsing implementations**, not modifications of existing streaming code.

### A2.5 Implement real streaming for OpenAI backend

**File:** `apxm-backends/src/llm/backends/openai/backend.rs`

Currently `generate_stream()` uses the default wrapper (single `Done` chunk). This is a new SSE parsing implementation:
- SSE parsing with `reqwest` streaming response
- `data: [DONE]` termination handling
- Delta assembly: `choices[0].delta.content` -> `StreamChunk::Token`
- Tool call deltas: `choices[0].delta.tool_calls` -> `ToolCallStart`/`ToolCallDelta`
- Usage chunk at end -> `StreamChunk::Usage`
- Error recovery -> `StreamChunk::RetryNotice`

~200 lines of new code in the existing file.

### A2.6 Implement real streaming for Anthropic backend

**File:** `apxm-backends/src/llm/backends/anthropic/backend.rs`

SSE stream parsing:
- `content_block_start` / `content_block_delta` / `content_block_stop`
- Text deltas -> `StreamChunk::Token`
- `tool_use` blocks -> `ToolCallStart` + argument deltas
- `message_delta` with `stop_reason` -> `Done`
- `message_start.usage` -> `StreamChunk::Usage`

~180 lines of new code.

### A2.7 Implement real streaming for Google backend

**File:** `apxm-backends/src/llm/backends/google/backend.rs`

Google API streams `GenerateContentResponse` objects:
- `candidates[0].content.parts[].text` -> `StreamChunk::Token`
- `candidates[0].content.parts[].thought` -> `StreamChunk::Thought`
- `candidates[0].content.parts[].functionCall` -> `ToolCallStart` (complete, not fragmented)
- `usageMetadata` -> `StreamChunk::Usage`
- `candidates[0].finishReason` -> `Done`

~150 lines of new code.

### A2.7b Ollama backend streaming (deferred)

**File:** `apxm-backends/src/llm/backends/ollama/backend.rs`

Ollama also uses the default `Done`-wrapper fallback today. Unlike the three backends above, Ollama streaming is **not implemented in Phase 1**. Ollama will continue to use the single-`Done`-chunk wrapper. Real streaming for Ollama (which uses NDJSON, not SSE) can be added in a later phase or as a follow-up to A2.5-A2.7 once those patterns are proven.

No new code. No line count impact.

### A2.8 Add OpenAI Responses API backend (NEW)

**New file:** `apxm-backends/src/llm/backends/openai/responses.rs`

Codex uses OpenAI's Responses API (POST /v1/responses), NOT Chat Completions. Key differences:
- Items format instead of messages
- Stateful via `previous_response_id`
- WebSocket transport option
- Built-in tools (file_search, code_interpreter, web_search)

This backend:
- Implements `LLMBackend` trait
- Translates `LLMRequest` -> Responses API request format
- SSE stream with `item/started`, `item/delta`, `item/completed` events
- Maps to `StreamChunk` variants

~600 lines. This is the largest single addition.

### A2.9 Add retry logic with event emission

**File:** `apxm-backends/src/llm/registry.rs`

When retrying requests (429, 5xx):
- Emit `StreamChunk::RetryNotice` between attempts
- StreamAssembler converts to `ApxmEvent(Retry(RetryEvent {...}))`
- Configurable per-backend: max attempts, backoff strategy, fallback chain

### A2.10 Tests

- StreamAssembler: tool call assembly (multi-fragment), sequence numbering, Done conversion
- OpenAI streaming: mock SSE responses, verify StreamChunk sequence
- Anthropic streaming: mock SSE, verify content_block assembly
- Google streaming: mock JSON stream, verify thought/content/tool extraction
- Retry: verify RetryNotice emission, fallback chain execution
- `generate_events()`: end-to-end with MockBackend, verify ApxmEvent output

### A2.11 Files created/modified

| File | Action | Lines (est.) |
|------|--------|-------------|
| `apxm-backends/Cargo.toml` | Modified (add apxm-events dep) | +1 |
| `apxm-backends/src/llm/backends/traits.rs` | Modified (3 new StreamChunk variants) | +10 |
| `apxm-backends/src/llm/assembler.rs` | **NEW** | ~120 |
| `apxm-backends/src/llm/registry.rs` | Modified (generate_events, retry) | +80 |
| `apxm-backends/src/llm/backends/openai/backend.rs` | Modified (real streaming) | +200 |
| `apxm-backends/src/llm/backends/openai/responses.rs` | **NEW** (Responses API) | ~600 |
| `apxm-backends/src/llm/backends/anthropic/backend.rs` | Modified (real streaming) | +180 |
| `apxm-backends/src/llm/backends/google/backend.rs` | Modified (real streaming) | +150 |
| Tests | New/modified | ~500 |
| **Total** | | **~1841** |

---

## Phase A3: `apxm-server` HTTP+SSE service (2 days)

### A3.1 Crate scaffold

**Existing crate:** `apxm/crates/apxm-server/` (already named `apxm-server` in Cargo.toml)

This is a binary crate -- an HTTP server that exposes APXM's LLM capabilities to non-Rust consumers (Gemini-CLI TypeScript, future Python SDK, or any framework that speaks HTTP+SSE). In Phase 4 this service extends with graph execution endpoints.

```
apxm-server/
  Cargo.toml
  src/
    main.rs          -- axum server, CLI args, graceful shutdown
    routes/
      mod.rs
      generate.rs    -- POST /v1/generate (non-streaming)
      stream.rs      -- POST /v1/generate-stream (SSE, emits ApxmEvent JSON)
      models.rs      -- GET /v1/models (available backends/models)
      health.rs      -- GET /v1/health
      schema.rs      -- GET /v1/schema (JSON Schema for ApxmEvent, optional)
    service.rs       -- Wraps LLMRegistry, manages lifecycle
```

**Cargo.toml:**
```toml
[package]
name = "apxm-server"
version.workspace = true
edition.workspace = true

[dependencies]
apxm-backends = { path = "../apxm-backends" }
apxm-events = { path = "../apxm-events" }
apxm-core = { path = "../apxm-core" }
axum = { version = "0.8", features = ["json"] }
tokio = { version = "1.0", features = ["full"] }
tokio-stream = "0.1"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tower = "0.5"
tower-http = { version = "0.6", features = ["cors"] }
tracing = "0.1"
tracing-subscriber = "0.3"
clap = { version = "4", features = ["derive"] }
anyhow = "1.0"
```

### A3.2 Wire format

The wire format IS `ApxmEvent` serialized as JSON. No separate spec needed.

**Request format** (POST /v1/generate-stream):
```json
{
  "messages": [...],
  "model": "gemini-2.5-pro",
  "backend": "google",
  "temperature": 0.7,
  "max_tokens": 4096,
  "tools": [...],
  "thinking_config": { "enabled": true, "budget_tokens": 8192 },
  "provider_config": { ... }
}
```

**Response format** (SSE):
```
event: apxm
data: {"meta":{...},"payload":{"kind":"token","text":"Hello"}}

event: apxm
data: {"meta":{...},"payload":{"kind":"tool_call","id":"...","name":"...","arguments":{...}}}

event: apxm
data: {"meta":{...},"payload":{"kind":"llm_done",...}}
```

### A3.3 Key routes

- `POST /v1/generate`: JSON request -> JSON response (wraps `registry.generate()`)
- `POST /v1/generate-stream`: JSON request -> SSE stream of `ApxmEvent` (wraps `registry.generate_events()`)
- `GET /v1/models`: List available backends and models
- `GET /v1/health`: Health check with backend connectivity
- `GET /v1/schema`: JSON Schema for `ApxmEvent` (requires `schema` feature on apxm-events)

### A3.4 Tests

- Request parsing (all fields, optional fields, defaults)
- SSE encoding (verify each ApxmEvent variant serializes correctly)
- Health endpoint
- Error handling (invalid model, backend unavailable)

### A3.5 Files created

| File | Lines (est.) |
|------|-------------|
| `apxm-server/Cargo.toml` | ~30 |
| `apxm-server/src/main.rs` | ~80 |
| `apxm-server/src/routes/mod.rs` | ~20 |
| `apxm-server/src/routes/generate.rs` | ~60 |
| `apxm-server/src/routes/stream.rs` | ~100 |
| `apxm-server/src/routes/models.rs` | ~40 |
| `apxm-server/src/routes/health.rs` | ~30 |
| `apxm-server/src/routes/schema.rs` | ~20 |
| `apxm-server/src/service.rs` | ~80 |
| Tests | ~200 |
| **Total** | **~660** |

---

## Phase A4: Migrate `apxm-runtime` events (1.5 days)

### A4.1 Add `apxm-events` dependency

**File:** `apxm-runtime/Cargo.toml`
```toml
apxm-events = { path = "../apxm-events" }
```

### A4.2 Bridge `ExecutionEventEmitter` to `EventBus`

**File:** `apxm-runtime/src/executor/events.rs`

Create a bridge that wraps `EventBus` as an `ExecutionEventEmitter`:

```rust
pub struct EventBusEmitter {
    bus: apxm_events::BusEmitter,
    source: apxm_events::EventSource,
    seq: AtomicU64,
    trace_id: String,
}

impl ExecutionEventEmitter for EventBusEmitter {
    fn emit_operation_start(&self, node_id: u64, op_type: &str) {
        self.bus.emit(ApxmEvent {
            meta: self.make_meta(),
            payload: EventPayload::OperationStart(OperationStartEvent {
                node_id,
                op_type: op_type.to_string(),
            }),
        });
    }
    // ... map all 16 existing methods
}
```

The existing `ExecutionEventEmitter` trait stays -- this is a non-breaking addition. The bridge converts runtime events to `ApxmEvent` and publishes them on the shared `EventBus`.

### A4.3 Wire into `ExecutionContext`

**File:** `apxm-runtime/src/executor/context.rs`

`ExecutionContext::with_event_emitter()` already exists. Add a convenience method:

```rust
impl ExecutionContext {
    pub fn with_event_bus(mut self, bus: &apxm_events::EventBus) -> Self {
        self.event_emitter = Some(Arc::new(EventBusEmitter::new(bus)));
        self
    }
}
```

### A4.4 Files modified

| File | Change | Lines (est.) |
|------|--------|-------------|
| `apxm-runtime/Cargo.toml` | Add apxm-events dep | +1 |
| `apxm-runtime/src/executor/events.rs` | Add EventBusEmitter | +80 |
| `apxm-runtime/src/executor/context.rs` | Add with_event_bus() | +10 |
| Tests | Bridge tests | ~60 |
| **Total** | | **~151** |

---

## Phase 1 Summary

| Phase | Scope | Days (est.) | New Files | Modified Files | New Lines (est.) |
|-------|-------|------------|-----------|----------------|-----------------|
| A0: Packaging | External dependency setup | 1 | 0 | 3 | ~30 |
| A1: apxm-events | Shared event vocabulary | 3 | 7 | 1 | ~1005 |
| A2: apxm-backends | Greenfield streaming (OpenAI/Anthropic/Google) + Responses API; Ollama deferred | 4 | 2 | 5 | ~1841 |
| A3: apxm-server | HTTP+SSE bridge | 2 | 9 | 1 | ~660 |
| A4: apxm-runtime migration | Unified event bus | 1.5 | 0 | 3 | ~151 |
| **Phase 1 Total** | | **11.5** | **18** | **13** | **~3687** |

### Dependency Graph (build order)

```
apxm-core (no changes needed)
    |
    v
apxm-events [NEW] (A1)
    |         \
    v          v
apxm-backends  apxm-runtime
(A2)           (A4)
    |
    v
apxm-server [NEW] (A3)
```

**Critical path:** A0 -> A1 -> A2 -> A3 (sequential, each depends on previous)
**Parallelizable:** A4 can run in parallel with A2/A3 (only depends on A1)

---

## Related Documents

- [Phase 1 Overview](README.md)
- [Investigation Report](INVESTIGATION.md)
- [Codex Phase 1 (C1-C4)](codex.md)
- [Gemini-CLI Phase 1 (G1-G5)](gemini-cli.md)
- [Plan 1: APXM Changes (all phases)](../plan-1-apxm-changes.md)
- [Global Integration Plan](../plan-global-integration.md)
