# Phase 1: LLM Backend

**Draft v6 -- Revised with investigation findings**

**Timeline:** Weeks 1-8
**Dependencies:** None (this is the foundation)

---

## Goal

Replace the LLM transport layer in both consumers (Codex and Gemini-CLI) with APXM's `apxm-backends` crate, gaining unified streaming, retry with event emission, multi-provider support, and credential management through a single substrate. This is the bottom-up foundation -- the compiler (Phase 5) requires the runtime, the runtime requires the AAM, and the AAM requires the tools and LLM backends. Phase 1 builds the bottom layer.

### Universal Substrate Design

Phase 1 designs the event vocabulary (`ApxmEvent`, `EventPayload`) and streaming infrastructure (`StreamAssembler`, `EventBus`, `apxm-server`) to be **consumer-agnostic**. Codex and Gemini-CLI are the first two consumers, but the `ApxmEvent` stream contract is not coupled to either. Any agent framework -- CrewAI, AutoGen, LangGraph, a custom Python CLI, or a future framework that does not yet exist -- can consume `ApxmEvent` SSE streams from `apxm-server` or link `apxm-backends` directly (Rust consumers). The TypeScript types in G1 and the Rust adapter in C2 are consumer-specific thin layers over a shared, stable event vocabulary.

---

## What Each Consumer Delivers

### APXM (Substrate)

The APXM substrate delivers four components that Codex and Gemini-CLI depend on:

| Step | Deliverable |
|------|-------------|
| **A0: Packaging** | External dependency setup -- git deps for Rust consumers, binary installation for non-Rust consumers, `~/.apxm/` directory structure |
| **A1: apxm-events** | New crate with 33 `EventPayload` variants across 3 layers (LLM, Runtime, Session), `EventBus`, `EventEmitter` trait |
| **A2: apxm-backends upgrade** | Real streaming for OpenAI/Anthropic/Google backends (greenfield SSE parsing -- no production backend has real streaming today), `StreamAssembler`, new OpenAI Responses API backend, retry with event emission. Ollama uses `Done`-wrapper fallback initially. |
| **A3: apxm-server** | HTTP+SSE binary service exposing APXM LLM capabilities to non-Rust consumers (routes: generate, generate-stream, models, health, schema). The crate is already named `apxm-server` in Cargo.toml. |
| **A4: Runtime event migration** | `EventBusEmitter` bridge from existing `ExecutionEventEmitter` to unified `EventBus` |

**Estimate:** 11.5 days, 18 new files, 13 modified files, ~3687 new lines.

### Codex (Consumer)

| Step | Deliverable |
|------|-------------|
| **C1: Dependencies** | Workspace git deps for apxm-core/backends/events, feature-gated `apxm-llm` feature |
| **C2: APXM adapter** | Event translator, request translator, `ApxmModelClient` wrapping `LLMRegistry` (~510 lines across 4 new files) |
| **C3: Feature-gated integration** | Wire into `submission_loop()` with `#[cfg(feature = "apxm-llm")]`, shadow mode validation, `ApxmBackendConfig` |
| **C4: Notification bridge** | Map `CodexApxmEvent` to 49-variant `ServerNotification` enum for TUI/JSON-RPC clients |

**Estimate:** 6.5 days, 4 new files, 7 modified files, ~650 new lines.

### Gemini-CLI (Consumer)

> **Bridge decision:** Gemini-CLI uses a **NAPI module** (`napi-rs`) for Phases 1-3, not HTTP+SSE via `apxm-server`. The server is a graph execution gateway (not an LLM proxy — no `/v1/generate-stream` endpoint) and enters the migration at Phase 4. See [Sessions & Server Investigation](../SESSIONS-AND-SERVER-INVESTIGATION.md) for the full analysis. The SSE client (G2) is retained as the interface abstraction — it can target either the NAPI module (Phases 1-3) or `apxm-server` HTTP (Phase 4+).

| Step | Deliverable |
|------|-------------|
| **G1: TypeScript types** | `ApxmEvent` TypeScript types matching Rust definitions (~200 lines) |
| **G2: NAPI bridge + service abstraction** | `ApxmServiceClient` wrapping NAPI module (in-process Rust calls); service abstraction layer supports future HTTP backend switch at Phase 4 |
| **G3: Event translator** | Pure function `toGeminiEvent()` mapping `ApxmEvent` to `ServerGeminiStreamEvent` |
| **G4: Upgrade generator** | Rewrite `ApxmContentGenerator` internals to use `ApxmServiceClient`, removing ~400 lines of per-provider dispatch |
| **G5: Integration + tests** | Wiring, graceful degradation fallback, streaming verification |

**Estimate:** 7 days, 5 new files, 2 modified files, ~680 new lines (+280 net after ~400 deleted).

---

## Validation Criteria

### APXM Substrate

- [ ] APXM installable as external dependency (git or `cargo install`)
- [ ] `cargo build -p apxm-events` compiles clean
- [ ] All 33 `EventPayload` variants round-trip through serde
- [ ] `cargo build -p apxm-backends` compiles with `apxm-events` dependency
- [ ] `StreamAssembler` correctly assembles fragmented tool calls
- [ ] OpenAI, Anthropic, Google backends produce correct `ApxmEvent` streams
- [ ] `apxm-server` starts, responds to health check, streams events
- [ ] Existing `apxm-runtime` tests still pass after A4 migration
- [ ] `cargo test --workspace` passes

### Codex

- [ ] `cargo build -p codex-core --features apxm-llm` compiles
- [ ] `cargo build -p codex-core` (without feature) still compiles -- no regressions
- [ ] APXM deps use git URLs, not relative paths
- [ ] Event translator unit tests: each `CodexApxmEvent` maps to correct `ServerNotification`
- [ ] Request translator: Codex `Prompt` to `LLMRequest` round-trip preserves all fields
- [ ] Shadow mode: <1% divergence between legacy and APXM paths on same prompts
- [ ] Streaming UX: TUI renders equivalent output from both paths
- [ ] Tool calls: approval flow works identically through APXM path
- [ ] Retry: APXM retry events surface as `ThreadStatusChanged` in TUI

### Gemini-CLI

- [ ] `npm run typecheck` passes with new types
- [ ] Event translator unit tests cover all 18 `GeminiEventType` mappings
- [ ] SSE client correctly parses multi-line SSE stream
- [ ] `GEMINI_CLI_USE_APXM=true gemini "hello"` works with `apxm-server` running
- [ ] Streaming: tokens appear incrementally in TUI (not all at once)
- [ ] Tool calls: function calls work through APXM path
- [ ] Thinking: thought events render in TUI
- [ ] Retry: 429 responses show retry indicator
- [ ] Fallback: graceful degradation when service is unavailable
- [ ] No regressions: existing Gemini API path unaffected

### Contract Tests

These tests verify the formal contract between APXM and its consumers:

#### APXM -> Consumer

| Test | Verifies |
|------|----------|
| **CT-1: `StreamChunk` serde round-trip** | All 7 StreamChunk variants (4 existing + 3 new) serialize/deserialize correctly |
| **CT-2: `ApxmEvent` serde round-trip** | All 33 EventPayload variants round-trip through JSON |
| **CT-3: `LLMRequest` schema completeness** | `LLMRequest` carries all fields needed by consumers (model, backend, temperature, max_tokens, tools, thinking_config) |
| **CT-4: `LLMResponse` -> `LlmDoneEvent` mapping** | `StreamAssembler` correctly converts `LLMResponse` fields to `LlmDoneEvent` |
| **CT-5: SSE wire format** | `apxm-server` SSE output matches `event: apxm\ndata: {json}\n\n` format exactly |
| **CT-6: `ExecutionEvent` -> `EventPayload` bridge** | All 16 `ExecutionEventEmitter` methods produce correct `EventPayload` variants through `EventBusEmitter` |

#### Consumer -> APXM

| Test | Verifies |
|------|----------|
| **CT-7: Codex `Prompt` -> `LLMRequest`** | All `Prompt` fields map to `LLMRequest` fields without data loss |
| **CT-8: Gemini `GenerateContentParameters` -> `ApxmGenerateRequest`** | Gemini SDK request format converts correctly to APXM request format |
| **CT-9: Feature gate isolation** | `cargo build -p codex-core` (without `apxm-llm` feature) compiles with no APXM-related code |

---

### Test Strategy

9 integration tests organized by mock boundaries (see [INVESTIGATION.md](INVESTIGATION.md) section 3.2 for full details):

| Test | Scope | Mock Boundary |
|------|-------|---------------|
| IT-1: APXM event stream golden path | `LLMRequest` -> `MockBackend` -> `StreamAssembler` -> `ApxmEvent` | MockBackend (no network) |
| IT-2: Codex APXM adapter round-trip | `Prompt` -> `ApxmModelClient` -> `CodexApxmEvent` -> `ServerNotification` | MockBackend in LLMRegistry |
| IT-3: Gemini-CLI event translation | `ApxmEvent` JSON -> `toGeminiEvent()` -> `ServerGeminiStreamEvent` | Pure function, no mocks |
| IT-4: Gemini-CLI SSE client | SSE stream parsing with all event types | Mock HTTP server |
| IT-5: `apxm-server` health + stream | Start service, POST generate-stream, verify SSE events | In-process axum test server |
| IT-6: Codex shadow mode | Legacy vs APXM path divergence < 1% | Real API keys (E2E) |
| IT-7: Retry event surfacing | 429 -> `RetryEvent` -> consumer retry UI | Mock HTTP returning 429 then 200 |
| IT-8: Tool call assembly | Multi-fragment `ToolCallStart` + N x `ToolCallDelta` + `Done` -> single `ToolCallEvent` | MockBackend with fragmented tool calls |
| IT-9: Graceful degradation | Gemini-CLI falls back when `apxm-server` is unavailable | Kill sidecar process |

---

### Critical Path Note

`apxm-events` (A1) is on the critical path -- both consumers depend on it and cannot start until it ships. Consider shipping LLM-layer events first (9 variants) to unblock consumers while runtime (16) and session (8) variants are designed. The `#[non_exhaustive]` attribute on `EventPayload` makes this safe.

---

### Risks

| Risk | Prob. | Impact | Mitigation |
|------|-------|--------|------------|
| **No real streaming in any production backend** -- A2.5-A2.7 are all greenfield SSE parsing implementations, not modifications of existing code. Each provider's SSE format is different. | HIGH | HIGH | Budget extra testing time for SSE edge cases (partial chunks, network interrupts, malformed SSE). |
| **`apxm-events` blocks everything** -- A1 is the critical path; consumers cannot start until it ships. | HIGH | HIGH | Ship LLM-layer events (9 variants) first to unblock consumers. |
| **Ollama streaming omitted** -- Plan covers OpenAI, Anthropic, Google but omits Ollama. | MEDIUM | LOW | Ollama uses `Done`-wrapper fallback initially. Document explicitly. |
| **Line count estimates are stale** -- 6 files are >50% larger than claimed (plan was written against an older codebase). | MEDIUM | MEDIUM | All file sizes re-baselined in this revision. The changes themselves are well-isolated. |
| **Feature gate breaks existing Codex builds** | LOW | HIGH | CT-9 explicitly tests this. |

---

## Consumer Files

- [APXM Changes (A0-A4)](apxm.md)
- [Codex Changes (C1-C4)](codex.md)
- [Gemini-CLI Changes (G1-G5)](gemini-cli.md)
- [Investigation Report](INVESTIGATION.md)

## Global Plans

- [Plan 1: APXM Changes (all phases)](../plan-1-apxm-changes.md)
- [Plan 2: Codex Changes (all phases)](../plan-2-codex-changes.md)
- [Plan 3: Gemini-CLI Changes (all phases)](../plan-3-gemini-changes.md)
- [Global Integration Plan](../plan-global-integration.md)
