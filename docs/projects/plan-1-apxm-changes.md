# Plan 1: APXM Changes -- Enabling the A-PXM Substrate for External Consumers

**Author:** Architecture Team
**Date:** 2026-03-20
**Status:** Draft v6 -- Revised with investigation findings
**Scope:** All changes needed inside `apxm/crates/` across ALL five adoption phases to enable Codex and Gemini-CLI integration
**Depends on:** Nothing (this is the foundation)
**Blocks:** Plan 2 (Codex), Plan 3 (Gemini-CLI)
**Aligned with:** [Global Integration Plan v5](plan-global-integration.md)

---

## Foundational Premise: What We Are Building

A-PXM is a **Program Execution Model** -- a formal specification of how agent programs are represented, optimized, and executed. It is not a runtime, not a framework, not a wrapper. The relationship to its components is the same as von Neumann's model to CPUs and compilers:

```
Von Neumann PXM:                    Agent PXM (A-PXM):
  Abstract Machine = (PC, Regs, Mem)   Abstract Machine = (B, G, C)
  Runtime = CPU silicon + RAM          Runtime = Dataflow scheduler + Memory tiers
  Compiler = gcc/clang                 Compiler = MLIR passes + AIS -> .apxmobj
```

The **punch line is the compiler** (Phase 5). But the compiler requires the runtime, the runtime requires the AAM, and the AAM requires the tools and LLM backends. So we build from the bottom up.

This plan covers every APXM-side change across all five phases. Phase 1 (LLM Backend) has the most detail because it is nearest. Phases 2-5 have clear deliverables but less implementation specificity -- they will be refined as Phase 1 completes.

---

## Phase Overview

| Phase | Label | Plan 1 Scope | Phases Below |
|-------|-------|-------------|-------------|
| **Phase 1** | LLM Backend | A0-A4: Packaging, events, streaming, service, runtime migration | A0, A1, A2, A3, A4 |
| **Phase 2** | Tool Migration | A5: Capability system extensions | A5 |
| **Phase 3** | AAM State Model | A6: Hierarchical AAM | A6 |
| **Phase 4** | Agent Loop as Graph | A7: Graph execution extensions | A7 |
| **Phase 5** | Compiler Integration | A8: Compiler for consumer graphs | A8 |

---

## Phase A0: Packaging & External Dependency (Phase 1, 1 day)

### Why This Comes First

Current plans reference APXM crates via relative paths (e.g. `../../../../apxm/crates/apxm-core` from `openai/codex/codex-rs/core`). This is fragile, non-portable, and assumes a specific monorepo directory layout. Both Codex and Gemini-CLI must be able to depend on APXM as an external package.

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
├── bin/
│   ├── apxm                     # CLI (compiler + runtime)
│   └── apxm-server              # HTTP+SSE LLM service (already exists)
├── config.toml                   # Backend configuration
├── credentials.toml              # LLM provider credentials (0600)
├── tools.toml                    # Registered tools/capabilities (Phase A5)
├── memory/                       # Default memory tier storage
│   ├── ltm.sqlite
│   └── episodes.jsonl
└── workspaces/                   # Hierarchical AAM state (Phase A6)
    └── <workspace-id>/
        ├── scope.toml
        ├── data/                 # B (Beliefs)
        ├── goals/                # G (Goals)
        └── tools/                # C (Capabilities)
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

## Phase A1: Create `apxm-events` crate (Phase 1, 3 days)

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

## Phase A2: Upgrade `apxm-backends` (Phase 1, 4 days)

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

### A2.4 Add `LLMRegistry::generate_events()` method

**File:** `apxm-backends/src/llm/registry.rs` (or equivalent)

New method alongside existing `generate()` and `generate_stream()`:

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

### A2.5 Implement real streaming for OpenAI backend

**File:** `apxm-backends/src/llm/backends/openai/backend.rs`

Currently `generate_stream()` uses the default wrapper (single `Done` chunk) -- as do ALL production backends (Anthropic, Google, Ollama). Only `MockBackend` has a real `generate_stream()` override (word-level token streaming). A2.5-A2.7 are all greenfield SSE parsing implementations. Must implement:
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

## Phase A3: `apxm-server` crate (Phase 1, 2 days)

> **Status:** This crate already exists as `apxm-server` at `apxm/crates/apxm-server/` (`Cargo.toml` line 2: `name = "apxm-server"`). Routes are defined inline in `apxm-server/src/main.rs` (not in separate route files as originally planned below). The descriptions below document the original plan structure for reference.

### A3.1 Crate scaffold

**Directory:** `apxm/crates/apxm-server/`

This is a binary crate -- an HTTP server that exposes APXM's LLM capabilities to non-Rust consumers (Gemini-CLI TypeScript, future Python SDK). The service also handles graph execution endpoints (`POST /v1/execute`, `POST /v1/execute/stream`).

```
apxm-server/
  Cargo.toml
  src/
    main.rs          -- axum server, CLI args, graceful shutdown, all routes inline
```

> **Note:** The original plan proposed separate route files (`routes/generate.rs`, `routes/stream.rs`, etc.). In the actual implementation, all routes are defined inline in `main.rs`.

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

### A3.5 Files

> **Note:** The crate already exists. Routes are inline in `main.rs`, not in separate files as originally planned. The table below is the original estimate; actual file structure is `apxm-server/src/main.rs` with all routes defined inline.

| File | Lines (est.) |
|------|-------------|
| `apxm-server/Cargo.toml` | ~30 |
| `apxm-server/src/main.rs` | All routes inline |
| Tests | ~200 |

---

## Phase A4: Migrate `apxm-runtime` events (Phase 1, 1.5 days)

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

## Phase A5: Capability System Extensions (Phase 2, ~3-4 weeks)

### Current State

The capability system already exists and is more mature than one might expect:

- **`CapabilityExecutor` trait** (`apxm-runtime/src/capability/executor.rs`): `async fn execute(args) -> Value` + `fn metadata() -> &CapabilityMetadata`
- **`CapabilityMetadata`** (`apxm-runtime/src/capability/metadata.rs`): Already has `name`, `description`, `parameters_schema`, `returns`, `cost_estimate`, `latency_estimate_ms`, `requires_auth`, `tags`, `read_only`, `metadata` fields
- **`CapabilityRegistry`** (`apxm-runtime/src/capability/registry.rs`): `DashMap`-backed concurrent registry with `register`, `get`, `list_names`, `list_metadata`, `unregister`
- **`CapabilitySystem`** (`apxm-runtime/src/capability/mod.rs`): Full coordinator with JSON Schema validation, timeout enforcement, interceptor pipeline (`pre_invoke`/`post_invoke`), approval channel, AAM integration
- **Four built-in tools** (`apxm-tools/src/`): `BashCapability`, `ReadCapability`, `WriteCapability`, `SearchWebCapability`

What is missing is the infrastructure to register **consumer tools** (Codex's 26 ToolHandlers -- 21 standard + 5 multi_agents, Gemini-CLI's tools) and persist those registrations. The 5 multi_agents handlers (spawn, wait, send_input, resume_agent, close_agent) are deferred to Phase 4 as `FLOW_CALL`/`WAIT_ALL` graph operations.

### A5.1 Adapter trait for consumer tools

Consumers need a lightweight way to wrap existing tool implementations as APXM capabilities:

```rust
/// Simplifies wrapping foreign tool handlers as CapabilityExecutors.
/// Consumer implements this; APXM provides the CapabilityExecutor wrapper.
pub trait ToolAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    fn execute(&self, args: serde_json::Value) -> BoxFuture<'_, Result<serde_json::Value, String>>;
}
```

A blanket `impl CapabilityExecutor for T where T: ToolAdapter` bridges the gap -- consumers write ~20 LOC per tool, not ~60.

### A5.2 Tool persistence layer

**New file:** `~/.apxm/tools.toml`

Registered tools persist across sessions:

```toml
[tools.shell]
type = "binary"
command = "bash"
description = "Execute shell commands"
schema = '{"type":"object","properties":{"command":{"type":"string"}}}'
read_only = false

[tools.filesystem_read]
type = "binary"
command = "cat"
description = "Read file contents"
read_only = true
```

**`ToolStore`** -- load from / save to `~/.apxm/tools.toml`, produces `Vec<CapabilityMetadata>` for registration at startup.

### A5.3 HTTP capability bridge

For non-Rust tools (Gemini-CLI's TypeScript tools), the service exposes a capability invocation endpoint:

```
POST /v1/capabilities/{name}/invoke
```

And a registration endpoint:

```
POST /v1/capabilities/register
```

This allows Gemini-CLI to register its tools at startup and invoke them through the APXM capability system.

### A5.4 Interceptor pipeline refinement

The interceptor pipeline already exists (`approval.rs`, `interceptor.rs`). Extend it for consumer use cases:

- **SandboxInterceptor**: Validate tool args against sandbox policy before execution
- **AuditInterceptor**: Log every capability invocation with args and result to episodic memory
- **CostInterceptor**: Track cumulative cost estimates and enforce budgets

### A5.5 CLI commands

Extend the existing `apxm tool` CLI:

```bash
apxm tool add my-tool --type binary --command ./my-tool --description "..."
apxm tool list [--json]
apxm tool show my-tool [--json]
apxm tool remove my-tool
apxm tool test my-tool
apxm tool doctor           # validate all registrations
```

### A5.6 Deliverables

| Deliverable | Location |
|-------------|----------|
| `ToolAdapter` trait + blanket impl | `apxm-runtime/src/capability/adapter.rs` |
| `ToolStore` persistence | `apxm-runtime/src/capability/store.rs` |
| HTTP capability bridge routes | `apxm-server/src/main.rs` (inline routes) |
| Sandbox/Audit/Cost interceptors | `apxm-runtime/src/capability/interceptor/` |
| CLI `tools` commands | `apxm-cli/src/commands/tools.rs` (already scaffolded) |

### A5.7 Acceptance criteria

- [ ] All 26 Codex ToolHandlers (21 standard + 5 multi_agents) registerable via `ToolAdapter` (~20 LOC each). The 5 multi_agents handlers (spawn, wait, send_input, resume_agent, close_agent) are deferred to Phase 4 as `FLOW_CALL`/`WAIT_ALL` graph operations.
- [ ] Gemini-CLI tools registerable via HTTP bridge
- [ ] Registrations persist in `~/.apxm/tools.toml`
- [ ] Interceptor pipeline (approval + sandbox + audit) works end-to-end
- [ ] `apxm tool list --json` returns all registered capabilities

---

## Phase A6: Hierarchical AAM (Phase 3, ~4-5 weeks)

### Current State

The AAM currently uses flat in-memory structures (see `apxm-runtime/src/aam/`):

- **Beliefs**: `HashMap<String, Value>` -- single global key-value store
- **Goals**: `PriorityQueue<Goal>` with `GoalTree` for parent-child relationships (from `apxm-core/src/types/aam.rs`)
- **Capabilities**: `HashMap<String, CapabilityRecord>` -- flat registry

The `apxm-core/src/types/aam.rs` module already defines scope-related types:

- **`ScopePolicy`** enum: `Inherit`, `Isolate`, `Snapshot`, `Filter(Vec<String>)`
- **`ScopeSpec`**: per-dimension (`beliefs`, `capabilities`, `goals`) scope specification
- **`GoalTree`**: parent-child goal relationships with `CompletionPolicy` (`AllChildren`, `AnyChild`, `Manual`)

These types are already wired into runtime execution: `Aam::child_scope()` is fully implemented with all four scope policies (Inherit, Isolate, Snapshot, Filter) and verified by six unit tests (`aam/mod.rs:762-956`). `ExecutionContext` carries `scope_id` and `scope_registry`. What remains is **file-tree backing** -- the `~/.apxm/workspaces/` directory materialization, the `StateProjector::promote()` direction (child to parent), and workspace directory management. The file-tree-backed hierarchical AAM described in `docs/implementation/runtime/hierarchical-aam.md` is the target.

### A6.1 WorkspaceManager

Manages `~/.apxm/workspaces/` -- creates, opens, and archives workspace directories:

```rust
pub struct WorkspaceManager {
    root: PathBuf,  // ~/.apxm/workspaces/
}

impl WorkspaceManager {
    /// Create a new workspace with scoped AAM directories
    pub fn create_workspace(&self, id: &str, scope: ScopeSpec) -> Result<WorkspacePath>;

    /// Open an existing workspace, loading its AAM state
    pub fn open_workspace(&self, id: &str) -> Result<Workspace>;

    /// Archive a completed workspace
    pub fn archive_workspace(&self, id: &str) -> Result<()>;
}
```

Each workspace gets the directory structure:

```
~/.apxm/workspaces/<workspace-id>/
├── scope.toml           # Scope metadata (id, parent, policy, revision)
├── data/                # B (Beliefs) -- TOML/JSON data files
├── goals/               # G (Goals) -- goal tree as TOML
│   └── current.toml
└── tools/               # C (Capabilities) -- tool definitions
    └── <registered-tools>.toml
```

### A6.2 Materializer

Writes AAM state to the file tree and reads it back:

```rust
pub struct Materializer;

impl Materializer {
    /// Project in-memory AAM state to the file tree
    pub fn materialize(workspace: &WorkspacePath, aam: &AamState) -> Result<()>;

    /// Load AAM state from the file tree
    pub fn hydrate(workspace: &WorkspacePath) -> Result<AamState>;
}
```

### A6.3 StateProjector

Enforces scoping rules (Inherit, Isolate, Snapshot, Filter) between parent and child workspaces:

```rust
pub struct StateProjector;

impl StateProjector {
    /// Create a child AAM scope from a parent, applying the scope policy
    pub fn project(parent: &AamState, policy: &ScopeSpec) -> AamState;

    /// Promote child state changes back to parent (explicit, policy-gated)
    pub fn promote(child: &AamState, parent: &mut AamState, keys: &[String]) -> Result<()>;
}
```

### A6.4 Three-tier memory backing

Map the memory hierarchy to the file tree:

| Tier | Current | Proposed |
|------|---------|----------|
| **STM** | In-memory `InMemoryBackend` | In-memory `InMemoryBackend` (unchanged -- volatile by design) |
| **LTM** | `~/.apxm/memory/ltm.sqlite` (global) | Per-workspace `data/*.toml` files (persistent, scoped) |
| **Episodic** | `~/.apxm/memory/episodes.jsonl` (global) | Per-workspace `episodes.jsonl` (scoped trace) |

### A6.5 CLI commands

```bash
apxm state show <session-id>          # Display AAM state for a workspace
apxm state show <session-id> --json   # Machine-readable AAM dump
apxm state list                       # List all workspaces
apxm state diff <session-a> <session-b>  # Diff two workspace states
```

### A6.6 Deliverables

| Deliverable | Location |
|-------------|----------|
| WorkspaceManager | `apxm-runtime/src/workspace/manager.rs` |
| Materializer | `apxm-runtime/src/workspace/materializer.rs` |
| StateProjector | `apxm-runtime/src/workspace/projector.rs` |
| Scoped execution context | `apxm-runtime/src/executor/context.rs` (modify) |
| CLI `state` commands | `apxm-cli/src/commands/state.rs` |

### A6.7 Acceptance criteria

- [ ] `apxm state show <session>` displays B, G, C from the file tree
- [ ] AAM state backed by `~/.apxm/workspaces/<id>/` directories
- [ ] Scoping rules (Inherit, Isolate, Filter) enforced between parent/child scopes
- [ ] Three-tier memory operational (STM in-memory, LTM file-backed, Episodic append-only)
- [ ] State survives process restarts (persistent file backing)

---

## Phase A7: Graph Execution Extensions (Phase 4, ~2-3 weeks)

### Current State

The runtime already executes AIS graphs -- that is its primary function. The dataflow scheduler, token-counting readiness detection, and operation handlers all exist. What Phase 4 requires is ensuring the runtime can execute graphs **submitted by external consumers** (Codex and Gemini-CLI), not just graphs compiled from internal sources.

### A7.1 Graph validation for consumer-authored graphs

Consumer-authored graphs may have errors that internally-compiled graphs would not. Strengthen validation:

- Node ID uniqueness enforcement
- Edge target validation (no dangling references)
- Required attributes per operation type (uses `get_operation_spec()` from `apxm-ais`)
- DAG cycle detection
- Type compatibility checks on Data edges

Most of this already exists in `apxm-graph/validate.rs`. Ensure it handles edge cases from hand-authored or adapter-generated graphs.

### A7.2 Graph execution endpoint

The `apxm-server` crate already has `POST /v1/execute` and `POST /v1/execute/stream` endpoints. Ensure the graph execution endpoint accepts consumer-authored graphs:

```
POST /v1/execute
Content-Type: application/json

{
  "graph": { ... },        // ApxmGraph JSON
  "parameters": { ... },   // Graph parameter values
  "workspace_id": "..."    // Optional workspace for AAM state
}
```

Response: SSE stream of `ApxmEvent` (reuses the same wire format as `/v1/generate-stream`).

### A7.3 `apxm execute` CLI for consumer graphs

Already exists -- ensure it works with consumer-authored graphs by validating input and providing clear error messages.

### A7.4 Deliverables

| Deliverable | Location |
|-------------|----------|
| Strengthened graph validation | `apxm-graph/src/validate.rs` (modify) |
| `POST /v1/execute` endpoint | `apxm-server/src/main.rs` (inline routes, already exists) |
| ~~Service rename~~ | Already complete -- crate is `apxm-server`, endpoints at `/v1/execute` and `/v1/execute/stream` already exist |

### A7.5 Acceptance criteria

- [ ] Consumer-authored AIS graphs validate and execute via `apxm execute`
- [ ] `POST /v1/execute` endpoint accepts ApxmGraph JSON and streams events
- [ ] Validation errors provide actionable messages for graph authors
- [ ] Runtime handles graphs with ASK, INV, BRANCH_ON_VALUE, WAIT_ALL, VERIFY, UMEM nodes

---

## Phase A8: Compiler Integration (Phase 5, ~3-4 weeks)

### This Is the Punch Line

Instead of imperative loops, consumers express their agent logic as AIS graphs. Those graphs are compiled, analyzed, and optimized. The compiler catches structural errors at compile time (49x faster than runtime detection), fuses redundant operations, eliminates dead code, and extracts parallelism.

### A8.0 Prerequisites (must be completed before Phase 5 begins)

- [ ] **GUARD wire index assigned (26)** -- Without this, graphs containing GUARD cannot be compiled to `.apxmobj`. The Gemini-CLI turn graph starts with GUARD. Hard blocker.
- [ ] **Other wire indices assigned** -- UpdateGoal (25), Claim (27), Pause (28), Resume (29).
- [ ] **Required attribute validation in `validate.rs`** -- Use `OperationSpec.fields` to check required attributes per operation type.
- [ ] **BRANCH_ON_VALUE naming in all consumer code** -- All graph JSON must use `"BRANCH_ON_VALUE"` not `"BRANCH"`.
- [ ] **TRY_CATCH MLIR subgraph verification** -- Verify the scheduler's error routing for subgraph boundaries.

### A8.1 Compiler passes on consumer graphs

The MLIR-based compiler has 12 distinct passes (not 4 as previously documented). At O2, the full pipeline runs: `normalize`, `build-prompt`, `template-specialization`, `unconsumed-value-warning`, `schema-narrowing`, `scheduling`, `fuse-ask-ops`, `condense-ops`, `dead-context-elimination`, `canonicalizer`, `cse`, `symbol-dce`. Pass names use kebab-case. Ensure they work on the graph patterns consumers produce:

| Pass (code name) | Consumer Benefit |
|------|-----------------|
| **fuse-ask-ops** | Merges sequential ASK->ASK chains into one API call (fewer calls, lower cost) |
| **cse** | Eliminates duplicate LLM calls with identical inputs (saves dollars and seconds) |
| **symbol-dce** | Removes operations whose outputs are never consumed (leaner graphs) |
| **canonicalizer** | Normalizes graph patterns for consistent optimization |
| **condense-ops** | Condenses operation sequences (O2+) |
| **dead-context-elimination** | Removes dead context (O2+) |
| **schema-narrowing** | Narrows schemas for tighter validation (O2+) |
| **template-specialization** | Specializes templates for the specific graph (O2+) |

Optimization levels:

| Level | Pass Count | Behavior |
|-------|-----------|----------|
| O0 | 0 | No optimization (passthrough) |
| O1 | 8 | Basic: normalize, build-prompt, scheduling, fusion, canonicalization, CSE, DCE |
| O2 | 12 | O1 + template-specialization, dead-context-elimination, schema-narrowing, condense-ops |
| O3 | 93 | O2 passes iterated 10 times for fixed-point convergence |

### A8.2 `apxm compile` for consumer graphs

```bash
apxm compile codex-turn.apxm -o codex-turn.apxmobj -O2
# Produces optimized artifact with compile-time analysis report

apxm compile codex-turn.apxm --emit-diagnostics diag.json
# Machine-readable: parallelism opportunities, fused ops, dead code, type errors
```

### A8.3 Compile-time validation

Structural errors caught before any LLM call:

- Unreachable nodes (dead code) -- **implemented** (symbol-dce pass)
- Cycles in what should be a DAG -- **implemented** (Kahn's algorithm in validate.rs)
- Node ID uniqueness, edge reference validation -- **implemented**
- Invalid operation sequences (e.g., MERGE without prior BRANCH_ON_VALUE) -- **partially implemented** (structural validation exists)
- Missing required attributes on AIS operations -- **NOT YET IMPLEMENTED** (OperationSpec.fields has required flags, but validate.rs does not use them; must be added before Phase 5)
- Type mismatches on data edges -- **NOT YET IMPLEMENTED** (no type annotations on edge tokens; must be added before Phase 5)
- Capability references to unregistered tools -- **NOT YET IMPLEMENTED** (capability resolution is runtime-only; must be added before Phase 5)

### A8.4 Deliverables

| Deliverable | Location |
|-------------|----------|
| Verify passes on consumer graph patterns | `apxm-compiler/src/passes/` (validate, possibly extend) |
| Compile endpoint on service | `apxm-server/src/main.rs` (inline routes) |
| Diagnostic output for consumers | `apxm-cli/src/commands/compile.rs` (enhance) |

### A8.5 Acceptance criteria

- [ ] `apxm compile agent-graph.apxm` produces optimized `.apxmobj` from consumer graphs
- [ ] FuseAskOps measurably reduces API calls on real Codex/Gemini-CLI workflows
- [ ] Compile-time validation catches structural errors before any LLM call
- [ ] `apxm analyze graph.apxm --json` provides parallelism and optimization report

---

## Summary: All APXM Changes Across Five Phases

| Phase | Scope | Days (est.) | New Files | Modified Files | New Lines (est.) |
|-------|-------|------------|-----------|----------------|-----------------|
| A0: Packaging | External dependency setup | 1 | 0 | 3 | ~30 |
| A1: apxm-events | Shared event vocabulary | 3 | 7 | 1 | ~1005 |
| A2: apxm-backends | Real streaming + Responses API | 4 | 2 | 5 | ~1841 |
| A3: apxm-server | HTTP+SSE bridge | 2 | 9 | 1 | ~660 |
| A4: apxm-runtime migration | Unified event bus | 1.5 | 0 | 3 | ~151 |
| **Phase 1 Total** | | **11.5** | **18** | **13** | **~3687** |
| A5: Capability system | Tool registration + persistence | ~15 | ~8 | ~5 | ~1500 |
| A6: Hierarchical AAM | File-backed scoped state | ~20 | ~10 | ~5 | ~2500 |
| A7: Graph execution | Consumer graph support | ~12 | ~3 | ~3 | ~500 |
| A8: Compiler integration | Passes on consumer graphs | ~15 | ~2 | ~5 | ~800 |
| **Full Total** | | **~74 days** | **~41** | **~31** | **~8987** |

---

## Dependency Graph

### Phase 1 (build order)

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
apxm-server (A3, already exists)
```

**Critical path:** A0 -> A1 -> A2 -> A3 (sequential, each depends on previous)
**Parallelizable:** A4 can run in parallel with A2/A3 (only depends on A1)

### Cross-phase dependencies

```
Phase 1 (A0-A4)
    |
    ├── Phase 2 (A5): Depends on CapabilitySystem (already exists) + apxm-server (A3)
    |
    ├── Phase 3 (A6): Depends on AAM types (already exist) + Phase 2 for tool persistence
    |
    ├── Phase 4 (A7): Depends on runtime + apxm-server (A3) + Phase 3 for scoped state
    |
    └── Phase 5 (A8): Depends on compiler (already exists) + Phase 4 for consumer graphs
```

---

## Risk Analysis

| Risk | Phase | Impact | Mitigation |
|------|-------|--------|------------|
| StreamChunk changes break existing runtime | 1 | Medium | New variants are additive; `generate_stream()` default wrapper still works |
| OpenAI Responses API complexity | 1 | High | Start with SSE transport only, add WebSocket later |
| Real streaming in 4 backends simultaneously | 1 | Medium | Implement one at a time; MockBackend already validates pattern |
| `apxm-server` port conflicts | 1 | Low | Configurable port, health check endpoint |
| `tokio::broadcast` back-pressure | 1 | Low | Configurable capacity (default 1024), slow consumers get `Lagged` error |
| APXM packaging complexity | 1 | Medium | Start with git dependencies, publish to crates.io later |
| Tool registration overhead for 26 handlers (21 standard + 5 multi_agents) | 2 | Medium | `ToolAdapter` trait reduces to ~20 LOC per handler; 5 multi_agents handlers deferred to Phase 4 |
| Hierarchical AAM not yet implemented | 3 | Medium | Start with flat AAM (already works), add hierarchy incrementally |
| AIS graph authoring complexity for consumers | 4 | High | Start with simple single-ASK graphs, grow incrementally |
| MLIR dependency for compiler | 5 | Low | Compiler is optional; Phases 1-4 use runtime only, no MLIR |
| Performance regression during migration | All | High | Shadow mode comparison at each phase |

---

## Validation Criteria

### Phase 1 Complete:
- [ ] APXM installable as external dependency (git or `cargo install`)
- [ ] `cargo build -p apxm-events` compiles clean
- [ ] All 33 EventPayload variants round-trip through serde
- [ ] `cargo build -p apxm-backends` compiles with apxm-events dependency
- [ ] `StreamAssembler` correctly assembles fragmented tool calls
- [ ] OpenAI, Anthropic, Google backends produce correct ApxmEvent streams
- [ ] `apxm-server` starts, responds to health check, streams events
- [ ] Existing `apxm-runtime` tests still pass after A4 migration
- [ ] `cargo test --workspace` passes

### Phase 2 Complete:
- [ ] All 26 Codex ToolHandlers (21 standard + 5 multi_agents) registered as APXM capabilities. The 5 multi_agents handlers are deferred to Phase 4.
- [ ] Gemini-CLI tools registered via HTTP capability bridge
- [ ] Tool approval flows through APXM interceptor pipeline
- [ ] `~/.apxm/tools.toml` persists registrations across sessions

### Phase 3 Complete:
- [ ] Agent state inspectable: `apxm state show <session>`
- [ ] AAM (B, G, C) backed by file tree under `~/.apxm/workspaces/`
- [ ] Three-tier memory operational (STM, LTM, Episodic)
- [ ] Scoping rules (Inherit, Isolate, Filter) enforced

### Phase 4 Complete:
- [ ] Consumer-authored AIS graphs execute via `apxm execute` and `/v1/execute`
- [ ] Automatic parallelism demonstrated on concurrent tool calls
- [ ] Graph validation catches structural errors with actionable messages

### Phase 5 Complete:
- [ ] `apxm compile agent-graph.apxm` produces optimized `.apxmobj`
- [ ] FuseAskOps reduces API calls on real consumer workflows
- [ ] Compile-time validation catches structural errors before any LLM call
- [ ] `apxm analyze graph.apxm --json` provides optimization report

---

## Related Documents

- [Global Integration Plan v5](plan-global-integration.md) -- Five-phase adoption path
- [Plan 2: Codex Changes](plan-2-codex-changes.md) -- Codex-side work
- [Plan 3: Gemini-CLI Changes](plan-3-gemini-changes.md) -- Gemini-CLI-side work
- [PXM Foundations](../pxm/foundations.md) -- Why agent workflows need a formal execution model
- [AAM: Agent Abstract Machine](../pxm/aam.md) -- Formal state model: AAM = (B, G, C)
- [Vision: The LLVM for Agents](../pxm/vision.md) -- File tree as AAM, compiler as punch line
- [Hierarchical AAM Diagrams](../implementation/runtime/hierarchical-aam.md) -- Flat -> hierarchical migration
- [AIS Operations](../pxm/ais.md) -- 39 typed operations
