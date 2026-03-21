# Phase 1 Investigation: LLM Backend -- Source Code Cross-Reference

**Date:** 2026-03-20
**Scope:** APXM (A0-A4), Codex (C1-C4), Gemini-CLI (G1-G5) plan claims vs source code

---

## 1. Verified Claims

### 1.1 APXM Source Code

#### LLMRegistry (apxm-backends)

| Claim | Status | Evidence |
|-------|--------|----------|
| `LLMRegistry` exists | VERIFIED | `$APXM_HOME/crates/apxm-backends/src/llm/registry/mod.rs:29` -- `pub struct LLMRegistry` |
| `StreamChunk` exists with 4 variants | VERIFIED | `$APXM_HOME/crates/apxm-backends/src/llm/backends/traits.rs:12-21` -- `Token`, `ToolCallStart`, `ToolCallDelta`, `Done` |
| `generate_stream()` exists on `LLMBackend` trait | VERIFIED | `traits.rs:33-41` -- default impl wraps `generate()` into single `Done` chunk |
| `generate()` exists on `LLMRegistry` | VERIFIED | `registry/mod.rs:270` -- with fallback chain logic |
| OpenAI, Anthropic, Google, Ollama backends exist | VERIFIED | `backends/openai/`, `backends/anthropic/`, `backends/google/`, `backends/ollama/` directories |
| MockBackend exists | VERIFIED | `backends/mock.rs` with real `generate_stream()` override at line 270 |
| `resolve_backend_for_streaming()` on LLMRegistry | VERIFIED | `registry/mod.rs:380-398` |

**Key finding:** There is NO `generate_stream()` method directly on `LLMRegistry`. The registry has `generate()` (non-streaming, line 270) and `resolve_backend_for_streaming()` (line 380). The plan claims `generate_events()` will be added (A2.4), which is correct -- it does not exist yet. However, the plan references an existing `generate_stream()` on the registry which does NOT exist. Only individual backends have `generate_stream()` via the `LLMBackend` trait.

#### Real Streaming Status

| Backend | Has `generate_stream()` override? | Status |
|---------|----------------------------------|--------|
| OpenAI | NO (uses default `Done` wrapper) | Plan correctly identifies this needs implementation (A2.5) |
| Anthropic | NO (uses default `Done` wrapper) | Plan correctly identifies this (A2.6) |
| Google | NO (uses default `Done` wrapper) | Plan correctly identifies this (A2.7) |
| Ollama | NO (uses default `Done` wrapper) | Not addressed in plan |
| Mock | YES (word-level token streaming, line 270) | Only mock has real streaming |

#### ExecutionEvent variants

| Claim | Status | Evidence |
|-------|--------|----------|
| 16 `ExecutionEvent` variants listed | VERIFIED | `$APXM_HOME/crates/apxm-runtime/src/executor/events.rs:16-101` |
| Specific variants match | VERIFIED | `LlmToken`, `ToolStart`, `ToolEnd`, `OperationStart`, `OperationEnd`, `PlanCreated`, `PlanStepStarted`, `PlanStepCompleted`, `MemoryRead`, `MemoryWrite`, `CheckpointSaved`, `CheckpointRestored`, `SchedulerDecision`, `GpuUtilization`, `TokenUsage`, `MemoizationHit` -- all 16 match |
| `ExecutionEventEmitter` trait exists | VERIFIED | `events.rs:123-164` -- all 16 event emission methods present |

#### apxm-events crate

| Claim | Status | Evidence |
|-------|--------|----------|
| `apxm-events` does NOT exist yet | VERIFIED | `ls $APXM_HOME/crates/ | grep events` returns empty -- the crate directory does not exist |
| Plan marks this as NEW (A1) | CONSISTENT | Correctly identified as new work |

#### Capability System

| Claim | Status | Evidence |
|-------|--------|----------|
| `CapabilityExecutor` trait exists | VERIFIED | `capability/executor.rs:17` -- `pub trait CapabilityExecutor: Send + Sync` with `execute()` and `metadata()` |
| `CapabilitySystem` exists | VERIFIED | `capability/mod.rs:59` -- full coordinator with registry, interceptors, approval |
| `CapabilityMetadata` exists | VERIFIED | `capability/metadata.rs:10` -- 10 fields: `name`, `description`, `parameters_schema`, `returns`, `cost_estimate`, `latency_estimate_ms`, `requires_auth`, `tags`, `read_only`, `metadata` |
| `CapabilityRegistry` (DashMap-backed) | VERIFIED | `capability/registry.rs:16-22` -- `DashMap<String, Arc<dyn CapabilityExecutor>>` |
| Interceptor pipeline (`pre_invoke`/`post_invoke`) | VERIFIED | `capability/interceptor.rs:20-33` -- `CapabilityInterceptor` trait with `pre_invoke`/`post_invoke` |
| `InterceptDecision` enum | VERIFIED | `interceptor.rs:9-16` -- `Allow`, `Deny { reason }`, `EditArgs { args }` |
| `register_interceptor()` on `CapabilitySystem` | VERIFIED | `capability/mod.rs:115-122` |
| Approval channel exists | VERIFIED | `capability/approval.rs:116-129` -- `ApprovalChannel` trait with `request_approval()` |
| `ApprovalStore` with scopes | VERIFIED | `approval.rs:39-103` -- `Once`, `Session`, `Always` scopes |
| 4 built-in tools | VERIFIED | `apxm-tools/src/`: `BashCapability`, `ReadCapability`, `WriteCapability`, `SearchWebCapability` |
| `ApxmPaths` in `paths.rs` | VERIFIED | `apxm-core/src/paths.rs` exists (111 lines) |

### 1.2 Codex Source Code

| Claim | Status | Evidence |
|-------|--------|----------|
| `apxm_provider_config.rs` exists | VERIFIED | `$HOME/projects/agents/codex/codex-rs/core/src/apxm_provider_config.rs` (336 lines) |
| File loads APXM backend configs | VERIFIED | Imports `ApxmLlmControlPlane`, `ResolvedApxmBackendConfig` from `apxm_core`, loads from `~/.apxm/config.toml` |
| Line count: plan says 337 | VERIFIED | Actual: 336 lines (off by 1 -- trivially close) |
| `submission_loop()` exists at line ~4138 | VERIFIED | `codex.rs:4138` -- `async fn submission_loop(sess: Arc<Session>, config: Arc<Config>, rx_sub: Receiver<Submission>)` |
| `codex.rs` size: plan says ~5400 | **DISCREPANCY** | Actual: 7321 lines (36% larger than claimed) |
| apxm-core dependency via relative path | VERIFIED | `core/Cargo.toml:21` -- `apxm-core = { path = "../../../apxm/crates/apxm-core" }` (not workspace git dep) |
| No workspace-level APXM dependency | VERIFIED | `codex-rs/Cargo.toml` has no `apxm` references |
| `ResponseEvent` enum (plan says 12 variants) | VERIFIED | `codex-api/src/common.rs:66-95` -- 12 variants: `Created`, `OutputItemDone`, `OutputItemAdded`, `ServerModel`, `ServerReasoningIncluded`, `Completed`, `OutputTextDelta`, `ReasoningSummaryDelta`, `ReasoningContentDelta`, `ReasoningSummaryPartAdded`, `RateLimits`, `ModelsEtag` |
| `client.rs` size: plan says ~800 | **DISCREPANCY** | Actual: 1823 lines (2.3x larger) |
| `client_common.rs` size: plan says ~400 | CLOSE | Actual: 328 lines (18% smaller) |
| `event_mapping.rs` size: plan says ~200 | CLOSE | Actual: 173 lines (14% smaller) |
| `codex-api/common.rs` size: plan says ~200 | **DISCREPANCY** | Actual: 281 lines (41% larger) |
| `codex-api/provider.rs` size: plan says 104 | **DISCREPANCY** | Actual: 170 lines (63% larger) |
| `codex-api/sse/responses.rs` size: plan says 1060 | VERIFIED | Actual: 1059 lines (off by 1) |
| `app-server-protocol/common.rs` size: plan says ~900 | **DISCREPANCY** | Actual: 1720 lines (91% larger) |
| `app-server-protocol/v2.rs` size: plan says ~300 | **DISCREPANCY** | Actual: 7978 lines (26x larger) |
| `ServerNotification` has 49 variants | VERIFIED | `common.rs:874-942` -- counted 49 variants in `server_notification_definitions!` macro |
| `ThreadItem` has 16 variants | VERIFIED | `v2.rs:4126` -- counted 16 variants |

#### ToolHandler Implementations

| Claim | Status | Evidence |
|-------|--------|----------|
| Plan says "21 ToolHandler implementations" | **DISCREPANCY** | Actual: **26** `impl ToolHandler` blocks found across handler files |

The 26 implementations include the 21 listed in the plan PLUS 5 in `multi_agents/` (spawn, wait, send_input, resume_agent, close_agent). The plan does note these 5 separately ("Additionally, the `multi_agents/` submodule contains 5 internal handlers") so the plan is internally consistent but the headline "21" count underrepresents the total. There are also 2 code_mode handlers (`wait_handler.rs`, `execute_handler.rs`) that may or may not implement `ToolHandler`.

The 21 listed handlers all match actual file names and struct names exactly.

| Claim | Status | Evidence |
|-------|--------|----------|
| `GuardianReviewSessionManager` exists | VERIFIED | `guardian/review_session.rs`, `guardian/review.rs`, `guardian/mod.rs` |

### 1.3 Gemini-CLI Source Code

| Claim | Status | Evidence |
|-------|--------|----------|
| `ApxmContentGenerator` exists | VERIFIED | `$HOME/projects/agents/gemini-cli/packages/core/src/core/apxmContentGenerator.ts` |
| File size: plan says 954 | VERIFIED | Actual: 953 lines (off by 1) |
| Has `generateWithOpenAI`, `generateWithAnthropic`, `generateWithOllama`, `generateWithGoogle` | VERIFIED | Lines 691, 763, 826, 676 respectively |
| `executeTurn()` at line 316 of `local-executor.ts` | VERIFIED | `local-executor.ts:316` -- `private async executeTurn(` |
| `local-executor.ts` size: plan says ~700+ | **DISCREPANCY** | Actual: 1479 lines (2x larger) |
| `CoreToolCallStatus` has 7 states | VERIFIED | `scheduler/types.ts:25-33` -- `Validating`, `Scheduled`, `Error`, `Success`, `Executing`, `Cancelled`, `AwaitingApproval` |
| `GeminiEventType` has 18 types | VERIFIED | `core/turn.ts:52-71` -- 18 values |
| `apxmConfig.ts` size: plan says 583 | VERIFIED | Actual: 583 lines |
| `contentGenerator.ts` size: plan says ~230 | **DISCREPANCY** | Actual: 320 lines (39% larger) |
| `turn.ts` size: plan says 448 | VERIFIED | Actual: 447 lines (off by 1) |
| `geminiChat.ts` size: plan says ~500 | **DISCREPANCY** | Actual: 1075 lines (2.15x larger) |
| `scheduler/types.ts` size: plan says 208 | VERIFIED | Actual: 207 lines (off by 1) |
| `HookEventName` has 11 events | VERIFIED | `hooks/types.ts:34-46` -- 11 values |
| `processFunctionCalls()` exists | VERIFIED | `local-executor.ts:993` |
| `DeclarativeTool` interface exists | VERIFIED | `tools/tools.ts:420` |
| `McpClientManager` exists | VERIFIED | `tools/mcp-client-manager.ts` |
| `PolicyDecision`, `PolicyRule` types exist in `policy.ts` | VERIFIED | `scheduler/policy.ts:10,12` |

---

## 2. Discrepancies Summary

### 2.1 Critical Discrepancies

| # | Claim | Actual | Impact |
|---|-------|--------|--------|
| D1 | `LLMRegistry` has `generate_stream()` | No such method; only `generate()` and `resolve_backend_for_streaming()` | Plan A2.4 correctly proposes adding `generate_events()`, but the narrative in A2.2 implies extending an existing method |
| D2 | 21 Codex ToolHandler implementations | 26 total (21 named + 5 multi_agents) | Phase 2 tool adapter count may be underestimated |

### 2.2 Line Count Discrepancies (significant, >50%)

| File | Plan Claims | Actual | Delta |
|------|------------|--------|-------|
| `codex.rs` | ~5400 | 7321 | +36% |
| `client.rs` | ~800 | 1823 | +128% |
| `common.rs` (app-server-protocol) | ~900 | 1720 | +91% |
| `v2.rs` (app-server-protocol) | ~300 | 7978 | +2559% |
| `local-executor.ts` | ~700+ | 1479 | +111% |
| `geminiChat.ts` | ~500 | 1075 | +115% |

These discrepancies suggest the plan was written against an older revision of the codebase. The actual integration points are in larger, more complex files than estimated.

### 2.3 Minor Discrepancies (<20% off)

These are close enough to be acceptable estimation variance:
- `apxm_provider_config.rs`: 337 claimed, 336 actual
- `turn.ts`: 448 claimed, 447 actual
- `apxmContentGenerator.ts`: 954 claimed, 953 actual
- `scheduler/types.ts`: 208 claimed, 207 actual
- `codex-api/sse/responses.rs`: 1060 claimed, 1059 actual

### 2.4 Structural Observations

1. **No `apxm-events` crate exists yet** -- the plan correctly identifies this as new work, but all consumer plans depend on it. This is the critical path item.

2. **No real streaming in any production backend** -- only `MockBackend` overrides `generate_stream()`. OpenAI, Anthropic, Google, and Ollama all fall through to the default single-`Done`-chunk wrapper. This means A2.5-A2.7 are genuinely new work, not modifications.

3. **`StreamChunk` has exactly 4 variants today** -- the plan proposes adding 3 new variants (`Thought`, `Usage`, `RetryNotice`). This is new work, correctly described.

4. **Codex already depends on `apxm-core` via relative path** -- the plan correctly identifies this needs migration to a git dependency (A0.2/C1.1).

5. **Ollama backend streaming is not addressed** -- the plan covers OpenAI, Anthropic, and Google streaming upgrades but omits Ollama. Ollama uses default `Done` wrapper like all others.

---

## 3. Integration Test Recommendations

### 3.1 Phase 1 Contract Tests (Phase Boundary)

These tests verify the contract between APXM and its consumers:

#### APXM -> Consumer Contract

| Test | What It Verifies | Location |
|------|-----------------|----------|
| **CT-1: `StreamChunk` serde round-trip** | All 7 StreamChunk variants (4 existing + 3 new) serialize/deserialize correctly | `apxm-backends/tests/` |
| **CT-2: `ApxmEvent` serde round-trip** | All 33 EventPayload variants round-trip through JSON | `apxm-events/tests/` |
| **CT-3: `LLMRequest` schema completeness** | `LLMRequest` carries all fields needed by consumers (model, backend, temperature, max_tokens, tools, thinking_config) | `apxm-backends/tests/` |
| **CT-4: `LLMResponse` -> `LlmDoneEvent` mapping** | `StreamAssembler` correctly converts `LLMResponse` fields to `LlmDoneEvent` (content, model, finish_reason, usage, tool_calls, thinking) | `apxm-backends/tests/` |
| **CT-5: SSE wire format** | `apxm-llm-service` SSE output matches `event: apxm\ndata: {json}\n\n` format exactly | `apxm-llm-service/tests/` |
| **CT-6: `ExecutionEvent` -> `EventPayload` bridge** | All 16 `ExecutionEventEmitter` methods produce correct `EventPayload` variants through `EventBusEmitter` | `apxm-runtime/tests/` |

#### Consumer -> APXM Contract

| Test | What It Verifies | Location |
|------|-----------------|----------|
| **CT-7: Codex `Prompt` -> `LLMRequest`** | All `Prompt` fields map to `LLMRequest` fields without data loss (messages, tools, model, temperature) | `codex-rs/core/tests/` |
| **CT-8: Gemini `GenerateContentParameters` -> `ApxmGenerateRequest`** | Gemini SDK request format converts correctly to APXM request format | `gemini-cli/packages/core/tests/` |
| **CT-9: Feature gate isolation** | `cargo build -p codex-core` (without `apxm-llm` feature) compiles with no APXM-related code | `codex-rs/` CI |

### 3.2 Phase 1 Integration Tests (End-to-End within Phase)

| Test | What It Verifies | Mock Boundaries |
|------|-----------------|-----------------|
| **IT-1: APXM event stream golden path** | `LLMRequest` -> `MockBackend` -> `StreamAssembler` -> `ApxmEvent` stream with correct sequence numbers, trace IDs, and payload types | MockBackend (no network) |
| **IT-2: Codex APXM adapter round-trip** | `Prompt` -> `ApxmModelClient.stream_turn()` -> `CodexApxmEvent` stream -> `ServerNotification` | MockBackend in LLMRegistry |
| **IT-3: Gemini-CLI event translation** | `ApxmEvent` JSON -> `toGeminiEvent()` -> `ServerGeminiStreamEvent` for all 18 `GeminiEventType` values | Pure function, no mocks needed |
| **IT-4: Gemini-CLI SSE client** | `ApxmServiceClient.generateStream()` correctly parses multi-line SSE stream with all event types | Mock HTTP server |
| **IT-5: `apxm-llm-service` health + stream** | Start service, health check, POST generate-stream, verify SSE events arrive | In-process axum test server |
| **IT-6: Codex shadow mode** | Run both legacy and APXM paths, compare `ServerNotification` sequences for divergence < 1% | Real API keys (E2E) |
| **IT-7: Retry event surfacing** | 429 response -> `RetryEvent` -> Codex `ThreadStatusChanged` / Gemini `GeminiEventType.Retry` | Mock HTTP returning 429 then 200 |
| **IT-8: Tool call assembly** | Multi-fragment `ToolCallStart` + N x `ToolCallDelta` + `Done` -> single `ToolCallEvent` with complete args | MockBackend with fragmented tool calls |
| **IT-9: Graceful degradation** | Gemini-CLI falls back to direct provider dispatch when `apxm-llm-service` is unavailable | Kill sidecar process |

### 3.3 Mock/Stub Boundaries

| Boundary | What to Mock | Why |
|----------|-------------|-----|
| **LLM Provider APIs** | HTTP responses from OpenAI/Anthropic/Google | Avoid live API calls in CI; deterministic SSE stream |
| **`apxm-llm-service` process** | In-process axum test server | Avoid spawning a separate process in unit tests |
| **Codex `ModelClient`** | Mock that returns known `ResponseEvent` sequences | Needed for shadow mode comparison |
| **Gemini-CLI `ContentGenerator`** | Mock that returns known `GenerateContentResponse` sequences | Needed for fallback testing |
| **File system (`~/.apxm/`)** | Temp directory with test configs | Avoid polluting real user config |

### 3.4 Test Location Map

```
apxm/crates/apxm-events/tests/
  serde_roundtrip.rs       -- CT-2 (33 variants)
  event_bus.rs             -- multi-producer/multi-consumer

apxm/crates/apxm-backends/tests/
  stream_assembler.rs      -- CT-1, CT-4, IT-8
  openai_streaming.rs      -- mock SSE responses
  anthropic_streaming.rs   -- mock SSE responses
  google_streaming.rs      -- mock JSON stream
  retry.rs                 -- IT-7 (APXM side)
  generate_events.rs       -- IT-1

apxm/crates/apxm-llm-service/tests/
  sse_wire_format.rs       -- CT-5
  health_endpoint.rs       -- IT-5
  stream_endpoint.rs       -- IT-5

apxm/crates/apxm-runtime/tests/
  event_bridge.rs          -- CT-6

codex/codex-rs/core/tests/
  apxm_adapter/
    event_translator.rs    -- CT-7
    request_translator.rs  -- CT-7
    client.rs              -- IT-2
    notification_bridge.rs -- IT-7 (Codex side)

gemini-cli/packages/core/src/core/apxm/
  __tests__/
    types.test.ts          -- type validation
    event-translator.test.ts -- IT-3
    client.test.ts         -- IT-4
    service-manager.test.ts
```

---

## 4. Risk Assessment

### 4.1 High Risk

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| **`codex.rs` is 7321 lines** (plan estimated ~5400) -- integration point at `submission_loop()` is in a much larger, more complex file than expected | HIGH | MEDIUM | The plan's feature-gated approach (`#[cfg(feature)]`) isolates changes well. Risk is in understanding the surrounding code, not in the change itself. |
| **No real streaming in any backend** -- A2.5-A2.7 are all greenfield SSE parsing implementations | HIGH | HIGH | Each provider's SSE format is different. Budget more testing time for edge cases (partial chunks, network interrupts, malformed SSE). |
| **`apxm-events` blocks everything** -- A1 is on the critical path; consumers cannot start until it ships | HIGH | HIGH | Prioritize A1 above all else. Consider shipping a minimal version (LLM layer only) to unblock consumers while runtime/session events are still being designed. |

### 4.2 Medium Risk

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| **Ollama streaming omitted** -- plan does not address Ollama streaming upgrade but Ollama backend exists | MEDIUM | LOW | Ollama can use default `Done` wrapper initially. Add streaming later. |
| **Line count estimates across many files are stale** -- 6 files are >50% larger than claimed, suggesting the plan was written against an older codebase | MEDIUM | MEDIUM | Re-baseline line counts before implementation. The larger files mean more context to understand but the actual changes are isolated. |
| **OpenAI Responses API backend (A2.8)** is 600 lines of new code -- the single largest addition and untested API surface | MEDIUM | HIGH | Start with Chat Completions streaming (A2.5), which Codex can also use. Responses API can follow. |

### 4.3 Low Risk

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| Feature gating breaks existing Codex builds | LOW | HIGH | CT-9 explicitly tests this |
| `ServerNotification` count changes | LOW | LOW | Plan correctly counts 49 variants; even if new ones are added, the APXM adapter only needs to map the subset it cares about |
| Gemini-CLI sidecar lifecycle issues | MEDIUM | LOW | Graceful degradation fallback (G5) handles this |

---

## 5. Specific Recommendations

1. **Re-baseline file sizes** -- Multiple files have grown 2-26x since the plan was written. While the changes themselves are well-isolated, anyone implementing should use current line numbers.

2. **Ship `apxm-events` LLM layer first** -- The 33-variant `EventPayload` enum can be staged: ship the 9 LLM variants first to unblock Codex and Gemini-CLI, then add runtime (16) and session (8) variants.

3. **Add Ollama streaming** -- Even if deprioritized, document that Ollama will use the `Done` wrapper fallback in Phase 1.

4. **Clarify `generate_stream()` vs `generate_events()`** -- The plan narrative in A2.2 implies extending an existing method. Clarify that `generate_events()` is an entirely new method on `LLMRegistry`, not a modification of `generate_stream()` (which only exists on `LLMBackend` trait, not on `LLMRegistry`).

5. **Test the multi_agents handlers** -- The plan says "21 ToolHandler implementations" but there are 26. The 5 `multi_agents` handlers (`spawn`, `wait`, `send_input`, `resume_agent`, `close_agent`) may need APXM adapters in Phase 2 even though the plan defers them to Phase 4 (`FLOW_CALL`/`WAIT_ALL`).
