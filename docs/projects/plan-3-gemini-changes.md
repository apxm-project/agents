# Plan 3: Gemini-CLI Changes -- Migrating Gemini-CLI onto the A-PXM Substrate

**Author:** Architecture Team
**Date:** 2026-03-20
**Status:** Draft v6 -- Revised with investigation findings
**Scope:** All changes needed inside `google/gemini-cli/` across all five phases of A-PXM adoption -- from LLM backend through tools, AAM state, agent loop as graph, and compiler integration
**Depends on:** Plan 1 (APXM Changes) -- specifically all APXM-side deliverables per phase
**Does NOT depend on:** Plan 2 (Codex) -- these are independent

---

## Foundational Correction

A-PXM is a **Program Execution Model**, not a runtime. It is a formal specification of how agent programs are represented, optimized, and executed, materialized as a compiler (MLIR-based) and a runtime (dataflow scheduler + AAM). Gemini-CLI does not "use A-PXM as a library" -- it progressively expresses its agent behavior in terms that A-PXM can represent, schedule, and ultimately compile.

The **punch line is the compiler**. Once Gemini-CLI's `executeTurn()` loop is expressed as an AIS graph, that graph can be compiled, analyzed, and optimized -- FuseAskOps, CSE, DCE, parallelism extraction. But the compiler requires the runtime, the runtime requires the AAM, and the AAM requires the tools and LLM backends. So we build from the bottom up.

---

## Overview

Gemini-CLI is a TypeScript monorepo (7 packages). An APXM integration **already exists**:
- `ApxmContentGenerator` (954 lines) -- multi-provider LLM adapter reading from `~/.apxm/config.toml`
- `apxmConfig.ts` (583 lines) -- configuration loading, provider resolution, routing
- Activated via `GEMINI_CLI_USE_APXM=true` or presence of `~/.apxm/credentials.toml`

**Current state:** `ApxmContentGenerator` works but is **non-streaming** (wraps full response in a single-yield generator) and duplicates the provider dispatch logic that already exists in APXM's Rust `apxm-backends` crate.

**Target state (Phase 5):** Gemini-CLI's `executeTurn()` loop is expressed as an AIS graph, compiled to `.apxmobj`, and executed on the A-PXM dataflow scheduler -- with typed tool dispatch, inspectable AAM state, and compiler-optimized turn structure.

---

## Current Architecture (Key Files)

| File | Role | Lines |
|------|------|-------|
| `core/src/core/apxmContentGenerator.ts` | Multi-provider LLM adapter (OpenAI, Anthropic, Google, Ollama) | 954 |
| `core/src/core/apxmConfig.ts` | Config loading from `~/.apxm/{config,credentials}.toml` | 583 |
| `core/src/core/contentGenerator.ts` | `ContentGenerator` interface + auth type detection | 320 |
| `core/src/core/turn.ts` | `Turn.run()` -- streams `ServerGeminiStreamEvent` (18 types), `GeminiEventType` enum | 447 |
| `core/src/core/geminiChat.ts` | `GeminiChat` -- conversation management, compression, retry | 1075 |
| `core/src/agents/local-executor.ts` | `LocalAgentExecutor` -- `executeTurn()` (line 316), `executeFinalWarningTurn()`, `processFunctionCalls()` | 1479 |
| `core/src/scheduler/scheduler.ts` | Tool scheduling, parallel execution, confirmation flow | 785 |
| `core/src/scheduler/types.ts` | `CoreToolCallStatus` (7 states), `ToolCallRequestInfo`, `ToolCallResponseInfo` | 207 |
| `core/src/scheduler/policy.ts` | `checkPolicy()` -- PolicyEngine integration for tool approval | 260 |
| `core/src/scheduler/confirmation.ts` | Tool confirmation flow -- `resolveConfirmation()` | 339 |
| `core/src/hooks/types.ts` | `HookEventName` enum (11 events) | ~60 |
| `core/src/hooks/hookSystem.ts` | Hook aggregation and dispatch | 444 |
| `core/src/tools/tools.ts` | `DeclarativeTool`, `ToolInvocation`, `ToolCallConfirmationDetails` | 1001 |
| `core/src/tools/tool-registry.ts` | `ToolRegistry` -- tool discovery and lookup | 762 |
| `core/src/output/types.ts` | `JsonStreamEventType` (6 types), `StreamStats` | 117 |

### Key Interfaces

```typescript
// ContentGenerator -- the interface ApxmContentGenerator implements
interface ContentGenerator {
  generateContent(request, promptId, role): Promise<GenerateContentResponse>;
  generateContentStream(request, promptId, role): Promise<AsyncGenerator<GenerateContentResponse>>;
  countTokens(request): Promise<CountTokensResponse>;
  embedContent(request): Promise<EmbedContentResponse>;
}

// ServerGeminiStreamEvent -- 18-variant union from Turn.run()
type ServerGeminiStreamEvent =
  | ServerGeminiContentEvent          // text token
  | ServerGeminiThoughtEvent          // thinking/reasoning
  | ServerGeminiToolCallRequestEvent  // function call
  | ServerGeminiToolCallResponseEvent // tool result
  | ServerGeminiToolCallConfirmationEvent
  | ServerGeminiFinishedEvent         // response complete
  | ServerGeminiRetryEvent            // retry notice
  | ServerGeminiErrorEvent            // error
  | ServerGeminiChatCompressedEvent   // context compaction
  | ServerGeminiCitationEvent         // citation
  | ServerGeminiLoopDetectedEvent     // loop guard
  | ServerGeminiMaxSessionTurnsEvent  // turn limit
  | ServerGeminiUserCancelledEvent    // abort
  | ServerGeminiContextWindowWillOverflowEvent
  | ServerGeminiInvalidStreamEvent
  | ServerGeminiModelInfoEvent
  | ServerGeminiAgentExecutionStoppedEvent
  | ServerGeminiAgentExecutionBlockedEvent;

// CoreToolCallStatus -- 7-state tool state machine
enum CoreToolCallStatus {
  Validating, Scheduled, Executing, Success, Error, Cancelled, AwaitingApproval
}

// HookEventName -- 11 hook event types
enum HookEventName {
  BeforeTool, AfterTool, BeforeAgent, Notification, AfterAgent,
  SessionStart, SessionEnd, PreCompress, BeforeModel, AfterModel,
  BeforeToolSelection
}
```

---

## Gemini-CLI Operations --> AIS Mapping

Every operation Gemini-CLI performs today has a direct AIS equivalent. This mapping is the blueprint for all five phases:

| Gemini-CLI Operation | Current Implementation | AIS Equivalent | AAM Effect | Phase |
|---------------------|----------------------|---------------|------------|-------|
| LLM call (prompt --> response) | `ContentGenerator.generateContentStream()` | **ASK** / **THINK** | Reads B, writes B | 1 |
| Thinking / thought events | `ServerGeminiThoughtEvent` | **THINK** (`thinking_budget`) | Reads B, writes B | 1 |
| Tool execution | `processFunctionCalls()` | **INV** (capability) | Reads C, writes B | 2 |
| Tool confirmation | `shouldConfirmExecute()` callback | **VERIFY** (approval) | Reads B, writes B | 2 |
| Parallel tool calls | Scheduler dispatches concurrent calls | **INV** x N + **WAIT_ALL** | Concurrent B writes | 2 |
| Max turns / timeout | `checkTermination()`, `DeadlineTimer` | **GUARD** (precondition) | -- | 2 |
| MCP tool discovery | `McpClientManager` | **AAM Capabilities** (writes C) | Writes C | 2 |
| PolicyEngine rules | `checkPolicy()` in `policy.ts` | **Interceptor pipeline** | -- | 2 |
| Context compression | `ChatCompressionService.tryCompressChat()` | **QMEM** + **UMEM** | Reads/writes B | 3 |
| Loop detection | `LoopDetectedEvent` pattern matching | **REFLECT** (pattern detection) | Reads Episodic | 3 |
| Session state | `GeminiChat`, conversation history | **AAM Beliefs** | -- | 3 |
| Registered tools (MCP + built-in) | `ToolRegistry` | **AAM Capabilities** | -- | 3 |
| Agent goal | User query / task definition | **AAM Goals** | -- | 3 |
| Hooks (11 event types) | `HookEventName` enum, `hookSystem.ts` | **FENCE** (ordering) | -- | 4 |
| Recovery turn | `executeFinalWarningTurn()` | **TRY_CATCH** (exception) | Reads B, writes B | 4 |
| Agent delegation | `LocalAgentExecutor` | **FLOW_CALL** (cross-agent) | Writes C, reads B | 4 |
| `executeTurn()` loop | `local-executor.ts:316` | **AIS Graph** (DAG) | Full AAM transition | 4 |

---

## Phase 1: LLM Backend (G1-G5)

**Goal:** Replace the in-TypeScript provider dispatch with HTTP calls to `apxm-server`, gaining real streaming and the full APXM backend ecosystem.

**APXM as external dependency:** The `apxm-server` binary lives at `~/.apxm/bin/apxm-server`. Gemini-CLI auto-starts it as a sidecar process, communicates via HTTP+SSE, and receives `ApxmEvent` payloads that are translated to `ServerGeminiStreamEvent` for the existing TUI.

### What Is NOT Changed (Phase 1)

These Gemini-CLI systems are untouched in Phase 1:
- **Tool execution** -- DeclarativeTool, ToolRegistry, MessageBus, PolicyEngine
- **MCP support** -- stdio/SSE/HTTP MCP connections
- **Scheduler** -- tool scheduling, parallel execution, tool state machine (Validating --> Scheduled --> AwaitingApproval --> Executing --> Success/Error/Cancelled)
- **TUI** -- React Ink components, output formatting
- **Chat compression** -- ChatCompressionService
- **Hooks** -- 11 hook event types
- **Auth** -- OAuth, API key, Vertex AI, Compute ADC (APXM replaces these with its own credential store)
- **Turn class** -- `Turn.run()` stays the same; it receives `ServerGeminiStreamEvent` from wherever they come from

### G1: TypeScript types for APXM events (1 day)

**New file:** `google/gemini-cli/packages/core/src/core/apxm/types.ts`

**Option A (recommended):** Auto-generate from `apxm-server` schema endpoint:

```bash
curl http://localhost:9100/v1/schema > apxm-event-schema.json
npx json-schema-to-typescript apxm-event-schema.json -o src/core/apxm/types.ts
```

**Option B (manual):** Write types by hand matching the Rust `ApxmEvent` definitions.

Key types (~200 lines):

```typescript
export interface ApxmEvent {
  meta: EventMeta;
  payload: EventPayload;
}

export interface EventMeta {
  seq: number;
  timestamp: string;
  trace_id: string;
  source: EventSource;
}

// Discriminated union on "kind" field
export type EventPayload =
  | TokenPayload          // kind: 'token'
  | ThoughtPayload        // kind: 'thought'
  | ToolCallPayload       // kind: 'tool_call'
  | LlmDonePayload        // kind: 'llm_done'
  | UsagePayload           // kind: 'usage'
  | RetryPayload           // kind: 'retry'
  | WarningPayload         // kind: 'warning'
  | CitationPayload        // kind: 'citation'
  | ProviderEventPayload   // kind: 'provider_event'
  | ErrorPayload           // kind: 'error'
  | ContextCompactedPayload // kind: 'context_compacted'
  | ModelReroutedPayload   // kind: 'model_rerouted'
  | CancelledPayload       // kind: 'cancelled'
  | LoopDetectedPayload    // kind: 'loop_detected'
  | ContextWindowWarningPayload; // kind: 'context_window_warning'
```

Each payload interface mirrors the Rust `ApxmEvent` variant exactly. See v4 for complete field-level definitions.

### G2: SSE client for `apxm-server` (1.5 days)

Two new files:

**`google/gemini-cli/packages/core/src/core/apxm/client.ts`** (~100 lines)

`ApxmServiceClient` -- HTTP+SSE client for `apxm-server`:
- `generateStream(request, signal?): AsyncGenerator<ApxmEvent>` -- POST to `/v1/generate-stream`, parse SSE `data:` lines
- `generate(request): Promise<ApxmEvent[]>` -- collect all events
- `health(): Promise<boolean>` -- GET `/v1/health`

```typescript
export interface ApxmGenerateRequest {
  messages: Array<{ role: string; content: unknown }>;
  model?: string;
  backend?: string;
  temperature?: number;
  max_tokens?: number;
  tools?: Array<{ name: string; description: string; parameters: unknown }>;
  thinking_config?: { enabled: boolean; budget_tokens?: number };
  provider_config?: Record<string, unknown>;
}
```

**`google/gemini-cli/packages/core/src/core/apxm/service-manager.ts`** (~60 lines)

`ApxmServiceManager` -- sidecar lifecycle management:
- Discovers binary at `$APXM_HOME/bin/apxm-server` (default: `~/.apxm/bin/apxm-server`)
- `start()` -- spawns process, waits for health check (10s timeout)
- `stop()` -- kills process
- Port default: 9100

### G3: Event translator (pure function) (1 day)

**New file:** `google/gemini-cli/packages/core/src/core/apxm/event-translator.ts` (~120 lines)

Pure function. No state. No buffering. Translates `ApxmEvent` --> `ServerGeminiStreamEvent`.

```typescript
export function toGeminiEvent(event: ApxmEvent): ServerGeminiStreamEvent | null {
  const p = event.payload;
  switch (p.kind) {
    case 'token':     return { type: GeminiEventType.Content, value: p.text, traceId: event.meta.trace_id };
    case 'thought':   return { type: GeminiEventType.Thought, value: p.summary ?? { subject: '', description: p.text } };
    case 'tool_call': return { type: GeminiEventType.ToolCallRequest, value: { callId: p.id, name: p.name, args: p.arguments, isClientInitiated: false, prompt_id: event.meta.trace_id } };
    case 'llm_done':  return { type: GeminiEventType.Finished, value: { reason: mapFinishReason(p.finish_reason.reason), usageMetadata: { ... } } };
    case 'retry':     return { type: GeminiEventType.Retry };
    case 'error':     return { type: GeminiEventType.Error, value: { error: { message: p.message, status: p.status } } };
    case 'context_compacted': return { type: GeminiEventType.ChatCompressed, value: { ... } };
    case 'cancelled': return { type: GeminiEventType.UserCancelled };
    case 'loop_detected': return { type: GeminiEventType.LoopDetected };
    case 'context_window_warning': return { type: GeminiEventType.ContextWindowWillOverflow, value: { ... } };
    case 'model_rerouted': return { type: GeminiEventType.ModelInfo, value: `Model rerouted: ${p.original_model} -> ${p.new_model} (${p.reason})` };
    case 'citation':  return { type: GeminiEventType.Citation, value: `Citations:\n${p.citations.map(c => c.url ?? c.title ?? '').join('\n')}` };
    case 'warning':   return { type: GeminiEventType.Error, value: { error: { message: `[${p.code}] ${p.message}` } } };
    default: return null;
  }
}
```

Unit-testable with all 18 `GeminiEventType` mappings covered.

### G4: Upgrade `ApxmContentGenerator` (2 days)

**File:** `google/gemini-cli/packages/core/src/core/apxmContentGenerator.ts`

**Strategy:** The existing `ApxmContentGenerator` class stays but its internals change:
- **Remove (~400 lines):** `generateWithOpenAI()`, `generateWithAnthropic()`, `generateWithOllama()`, `generateWithGoogle()`, per-provider response/request interfaces, per-provider message/tool translators
- **Keep:** `apxmConfig.ts` (config loading), `resolveApxmRoute()` (backend/model resolution), `normalizeContents()` / `toContent()` / `toPart()` (Gemini SDK format utilities), `toGenerateContentResponse()` (response wrapping), `estimateTokenCount()` (local heuristic)
- **Add:** `ApxmServiceClient` integration, `ApxmServiceManager` lifecycle, request translation (`GenerateContentParameters` --> `ApxmGenerateRequest`)

The constructor now takes an `ApxmServiceConfig` and manages the sidecar:

```typescript
constructor(apxmConfig: ResolvedApxmConfig, serviceConfig?: ApxmServiceConfig) {
  this.serviceClient = new ApxmServiceClient(serviceConfig ?? { baseUrl: 'http://localhost:9100' });
  this.serviceManager = new ApxmServiceManager(findApxmServiceBinary());
}

async *generateContentStream(request, promptId, role): AsyncGenerator<GenerateContentResponse> {
  await this.ensureServiceRunning();
  const apxmRequest = this.toApxmRequest(request);
  for await (const event of this.serviceClient.generateStream(apxmRequest)) {
    const geminiEvent = toGeminiEvent(event);
    if (geminiEvent) yield this.eventToPartialResponse(event, geminiEvent);
  }
}
```

### G5: Integration and testing (1.5 days)

**Wiring:** The existing content generator factory in `contentGenerator.ts` already handles APXM via `AuthType.APXM`. `ApxmContentGenerator.create()` now calls `ensureServiceRunning()` to start the sidecar.

**Graceful degradation:** If `apxm-server` is not available (binary not found, port in use), fall back to existing direct-HTTP provider dispatch with a warning.

**Module structure:**

```
google/gemini-cli/packages/core/src/core/apxm/
  types.ts           -- ApxmEvent TypeScript types
  client.ts          -- ApxmServiceClient (SSE)
  service-manager.ts -- Sidecar process management
  event-translator.ts -- toGeminiEvent() pure function
  index.ts           -- re-exports
```

**Tests:**
- Type validation: each `ApxmEvent` variant deserializes correctly from JSON
- Event translator: unit tests for all 18 `toGeminiEvent()` mappings
- SSE client: mock server with known events, verify parsed output
- Integration: full flow from request --> service --> events --> Gemini events
- Streaming verification: tokens appear incrementally in TUI (not all at once)

### Phase 1 Summary

| Task | Days | New Files | Modified Files | New Lines | Deleted Lines |
|------|------|-----------|----------------|-----------|---------------|
| G1: TypeScript types | 1 | 1 | 0 | ~200 | 0 |
| G2: SSE client + service manager | 1.5 | 2 | 0 | ~160 | 0 |
| G3: Event translator | 1 | 1 | 0 | ~120 | 0 |
| G4: Upgrade generator | 2 | 0 | 1 | ~100 | ~400 |
| G5: Integration + tests | 1.5 | 1 | 1 | ~100 | 0 |
| **Total** | **7** | **5** | **2** | **~680** | **~400** |

**Net change:** +280 lines (680 added, 400 deleted from provider dispatch)

---

## Phase 2: Tool Migration (G6-G8)

**Goal:** Register Gemini-CLI's tool implementations as APXM capabilities, route tool calls through APXM's CapabilitySystem, and gain the interceptor pipeline (approval, sandbox, audit).

**Prerequisite:** Plan 1 delivers the capability system extensions and interceptor pipeline.

### What Changes (Phase 2)

- Tool calls go through APXM CapabilitySystem instead of direct `DeclarativeTool.execute()`
- Tool confirmation flow maps to VERIFY operation semantics
- PolicyEngine rules become APXM interceptors
- MCP tools register through APXM's MCP integration
- Tool state machine is formalized in APXM terms

### What Is NOT Changed (Phase 2)

- `DeclarativeTool` implementations themselves (the code that runs tools stays)
- TUI rendering of tool results
- Hook system
- Chat compression
- `GeminiChat` conversation management

### G6: Register Gemini-CLI tools as APXM capabilities (3 days)

**G6.1 Capability bridge**

Each `DeclarativeTool` is wrapped as an APXM `CapabilityExecutor` via HTTP registration. Gemini-CLI registers each tool at startup via `apxm-server`:

```typescript
// Pseudocode -- registers built-in + MCP tools with APXM
async function registerToolsWithApxm(
  toolRegistry: ToolRegistry,
  apxmClient: ApxmServiceClient,
): Promise<void> {
  for (const tool of toolRegistry.getAllTools()) {
    await apxmClient.registerCapability({
      name: tool.name,
      description: tool.schema.description,
      parameters: tool.schema.parameters,
      metadata: {
        read_only: isReadOnlyTool(tool),
        latency_tier: estimateToolLatency(tool),
      },
    });
  }
}
```

**G6.2 MCP tools**

MCP tools discovered by `McpClientManager` register through APXM's MCP integration. Instead of Gemini-CLI calling MCP servers directly, APXM's capability system handles MCP dispatch, gaining unified tool auditing and interceptor support.

**G6.3 Tool state machine formalization**

Gemini-CLI's current 7-state tool state machine maps cleanly to APXM's capability lifecycle:

| Gemini-CLI State (`CoreToolCallStatus`) | APXM Operation | AIS Equivalent |
|----------------------------------------|----------------|----------------|
| `Validating` | Capability lookup + schema validation | Pre-INV validation |
| `AwaitingApproval` | Interceptor pipeline (approval) | **VERIFY** |
| `Scheduled` | Capability ready, awaiting executor | INV queued |
| `Executing` | CapabilityExecutor.invoke() running | **INV** (active) |
| `Success` | Result recorded | INV completed |
| `Error` | Error captured | INV failed (TRY_CATCH) |
| `Cancelled` | Abort signal propagated | INV cancelled |

### G7: Parallel tool calls as INV x N + WAIT_ALL (2 days)

**G7.1 Fan-out/fan-in**

Gemini-CLI's scheduler already dispatches concurrent tool calls. The APXM mapping formalizes this as:

```
Model returns N function calls
  --> N x INV operations (no edges between them --> parallel)
  --> WAIT_ALL (join barrier)
  --> results collected as tool responses
```

The scheduler's `Promise.all()` concurrency pattern becomes automatic DAG parallelism. In Phase 2, this is a conceptual mapping only -- actual graph execution happens in Phase 4.

**G7.2 Tool confirmation as VERIFY**

The `shouldConfirmExecute()` callback on `ToolInvocation` maps to a **VERIFY** operation:

```
INV(tool_a) --> VERIFY(approval_required?) --> INV(tool_a, execute)
```

The existing confirmation flow via `MessageBus` (publish request, wait for response via `correlationId`) becomes the VERIFY operation's implementation. The `ToolConfirmationOutcome` (approve/deny) maps to VERIFY's verdict.

### G8: PolicyEngine rules as interceptors (1 day)

**G8.1 Policy bridge**

Gemini-CLI's `checkPolicy()` function (in `scheduler/policy.ts`) evaluates `PolicyRule` instances against tool calls. These map to APXM interceptor pipeline entries:

| PolicyEngine Concept | APXM Interceptor |
|---------------------|-------------------|
| `PolicyDecision.ALLOW` | Interceptor passes |
| `PolicyDecision.DENY` | Interceptor rejects (typed error) |
| `PolicyDecision.ASK_USER` | Interceptor triggers VERIFY |
| `ApprovalMode` per tool | Interceptor configuration |
| `PolicyRule.denyMessage` | Interceptor error payload |

The interceptor pipeline provides audit logging for free -- every policy decision is recorded in the execution trace.

### Phase 2 Summary

| Task | Days | Key Deliverable |
|------|------|----------------|
| G6: Capability registration | 3 | All tools registered as APXM capabilities |
| G7: Parallel calls + VERIFY | 2 | Fan-out/fan-in formalized, confirmation as VERIFY |
| G8: PolicyEngine as interceptors | 1 | Policy rules mapped to interceptor pipeline |
| **Total** | **6** | -- |

### Phase 2 Validation

- [ ] All `DeclarativeTool` implementations registered as APXM capabilities
- [ ] MCP-discovered tools registered through APXM capability system
- [ ] Tool confirmation flow works through VERIFY semantics
- [ ] PolicyEngine decisions recorded in APXM audit trail
- [ ] `~/.apxm/tools.toml` persists Gemini-CLI tool registrations
- [ ] Tool execution latency not measurably worse than direct dispatch

---

## Phase 3: AAM State Model (G9-G10)

**Goal:** Map Gemini-CLI's scattered internal state to AAM's formal `(B, G, C)` triple, backed by the hierarchical file tree under `~/.apxm/workspaces/`. Enable `apxm state show <session>` for any Gemini-CLI session.

**Prerequisite:** Plan 1 delivers hierarchical AAM (scoped state, file-backed).

### What Changes (Phase 3)

- Session state becomes typed, inspectable AAM
- Three-tier memory replaces ad-hoc in-memory state
- Agent sessions become diffable, persistent, auditable

### What Is NOT Changed (Phase 3)

- `executeTurn()` loop structure (still imperative -- graph form is Phase 4)
- Tool dispatch mechanism (still via APXM capabilities from Phase 2)
- TUI rendering

### G9: Map GeminiChat state to AAM (B, G, C) (4 days)

**G9.1 Beliefs mapping**

Every piece of Gemini-CLI session state maps to a typed AAM Belief:

| Gemini-CLI State | AAM Component | Typed Key | Source |
|-----------------|--------------|-----------|--------|
| Conversation history | **B** (Beliefs) | `conversation: Vec<Content>` | `GeminiChat` |
| Active model / provider | **B** (Beliefs) | `model: ModelConfig` | `apxmConfig` resolution |
| Pending tool confirmations | **B** (Beliefs) | `pending_confirms: Vec<ToolCallId>` | Scheduler state |
| Compression state | **B** (Beliefs) | `compression: CompressionStatus` | `ChatCompressionService` |
| Turn counter | **B** (Beliefs) | `turn_count: number` | `LocalAgentExecutor` |
| User hints / injections | **B** (Beliefs) | `user_hints: Vec<String>` | Injection service |
| Background completions | **B** (Beliefs) | `bg_completions: Vec<String>` | Fast-ack helper |

**G9.2 Goals mapping**

| Gemini-CLI Concept | AAM Component | Representation |
|-------------------|--------------|----------------|
| User query / task | **G** (Goals) | `Goal("answer_user_query", priority=1)` |
| Sub-agent task | **G** (Goals) | `Goal(definition.query, parent=root_goal)` |
| `complete_task` tool | **G** (Goals) | Goal completion signal |

**G9.3 Capabilities mapping**

| Gemini-CLI Concept | AAM Component | Representation |
|-------------------|--------------|----------------|
| Built-in tools | **C** (Capabilities) | `Map<ToolName, ToolSchema>` |
| MCP-discovered tools | **C** (Capabilities) | `Map<McpToolName, McpToolSchema>` |
| Agent delegation | **C** (Capabilities) | `Map<AgentName, AgentSignature>` |
| LLM provider | **C** (Capabilities) | `Map<"llm", ModelCapability>` |

**G9.4 File tree structure**

The file tree IS the AAM. A Gemini-CLI session produces:

```
~/.apxm/workspaces/gemini-session-<id>/
  data/                         B (Beliefs)
    conversation.json           - Chat history
    pending_tools.json          - In-flight tool calls
    compression.json            - Compression state
    turn_state.json             - Turn counter, hints, bg completions
  goals.toml                    G (Goals)
    goal = "Fix the failing test"
    priority = 1
  tools/                        C (Capabilities)
    shell_command.toml           - Shell execution capability
    read_file.toml              - File read capability
    mcp/                        - MCP-discovered tools
      filesystem.toml
```

**Deliverable:** `apxm state show gemini-session-<id>` prints the full AAM snapshot for any Gemini-CLI session.

### G10: Three-tier memory (2 days)

APXM's memory hierarchy maps directly to Gemini-CLI's data patterns:

| Memory Tier | Gemini-CLI Equivalent | What It Holds |
|------------|----------------------|--------------|
| **STM** (Short-Term, ~us) | Current turn context, recent tool output | Working memory for the active turn -- intermediate results that don't survive session restart |
| **LTM** (Long-Term, ~ms) | Environment memory, project memory, JIT context | Persistent user preferences, project facts, cached tool results that survive across sessions |
| **Episodic** (append-only) | *(not currently tracked)* | Execution trace: every tool call, LLM response, and state transition -- the substrate for REFLECT and loop detection |

**G10.1 STM integration**

Current turn context (working message, partial tool results) stores in APXM STM. Microsecond access. Volatile.

**G10.2 LTM integration**

Gemini-CLI's environment memory and project memory map to APXM LTM (SQLite-backed at `~/.apxm/memory/ltm.sqlite`). Persistent across sessions.

**G10.3 Episodic memory**

Every tool call, LLM response, and state transition is appended to the episodic trace (`~/.apxm/memory/episodes.jsonl`). This is the substrate for:
- Loop detection (currently `LoopDetectedEvent` pattern matching) --> formalized as REFLECT on episodic trace
- Debugging (currently "attach debugger") --> `apxm state show` on episodic trace
- Audit (currently ad-hoc logging) --> typed, structured execution history

### Phase 3 Summary

| Task | Days | Key Deliverable |
|------|------|----------------|
| G9: AAM state mapping | 4 | Full (B, G, C) mapping, file-tree backing |
| G10: Three-tier memory | 2 | STM/LTM/Episodic for Gemini-CLI sessions |
| **Total** | **6** | -- |

### Phase 3 Validation

- [ ] `apxm state show <session>` works for Gemini-CLI sessions
- [ ] AAM (B, G, C) backed by file tree under `~/.apxm/workspaces/`
- [ ] Beliefs survive session restart (via file persistence)
- [ ] Episodic trace records every tool call and LLM response
- [ ] Loop detection reads from episodic trace (not ad-hoc pattern matching)
- [ ] Three-tier memory operational (STM for current turn, LTM for project memory, Episodic for trace)

---

## Phase 4: Agent Loop as Graph (G11-G12)

**Goal:** Express `executeTurn()` as an AIS graph instead of imperative code. Replace the ~200-line imperative turn loop with a typed, compiler-validated DAG. The `apxm-server` crate handles graph execution, capability system management, and AAM state.

**Prerequisite:** Phases 2 and 3 complete (tools are capabilities, state is AAM).

### What Changes (Phase 4)

- `executeTurn()` becomes a declarative AIS graph
- Hooks become FENCE operations in the graph
- Recovery turn becomes a TRY_CATCH subgraph
- Sub-agent delegation becomes FLOW_CALL

### What Is NOT Changed (Phase 4)

- TUI (still renders `ServerGeminiStreamEvent` or ApxmEvent)
- User-facing CLI interface
- Configuration system

### G11: Express `executeTurn()` as AIS graph (4 days)

**G11.1 Turn graph structure**

The current `executeTurn()` at `local-executor.ts:316` has interleaved concerns -- compression, LLM call, tool dispatch, confirmation, loop detection, recovery -- all in one function. The AIS graph separates each concern into a typed, independently-optimizable node:

```
                     executeTurn() as AIS Graph

  ┌──────────────┐
  |  GUARD        |  <-- checkTermination() + DeadlineTimer
  |  max_turns,   |      (precondition enforcement)
  |  timeout      |
  └──────┬───────┘
         |
         v
  ┌──────────────┐
  |  QMEM + UMEM |  <-- tryCompressChat()
  |  (compress)   |      (context compaction if needed)
  └──────┬───────┘
         |
         v
  ┌──────────────┐                    ┌──────────┐
  |  ASK / THINK  |---- tool_calls? -->|  BRANCH_ON_VALUE  |
  |  (LLM call)   |    ┌──────────────|  on calls         |
  └──────────────┘    |              └────────┬───────────┘
                      |                   | no tool calls
                      v                   v
             ┌───────────────┐     ┌──────────────┐
             |  INV tool_a    |     |  UMEM         |
             |  INV tool_b    |-┐   |  save_response|
             |  INV tool_c    | |   └──────────────┘
             └───────────────┘ |
                               v
                      ┌───────────────┐
                      |  WAIT_ALL      |  <-- parallel tool calls join
                      └───────┬───────┘
                              |
                              v
                      ┌───────────────┐
                      |  VERIFY        |  <-- shouldConfirmExecute()
                      |  (optional)    |      per-tool confirmation
                      └───────┬───────┘
                              |
                              v
                      ┌───────────────┐   ┌───────────────┐
                      |  TRY_CATCH     |-->|  REFLECT       |  <-- loop detection
                      |  (recovery)    |   |  (pattern      |      on episodic trace
                      └───────────────┘   |   detection)   |
                                          └───────────────┘
```

The three INV operations run concurrently -- the scheduler sees no edge between them and dispatches them in parallel. The GUARD node at the top enforces max turns and timeout before any LLM call happens.

**Key benefit:** The current `executeTurn()` interleaves termination checks, LLM calls, tool dispatch, confirmation waiting, compression, and loop detection in one function. The AIS graph makes each concern a separate, testable, independently-optimizable node.

**G11.2 Graph construction**

The turn graph is constructed at session start (or at configuration change) and submitted to APXM for execution:

```typescript
// Pseudocode -- construct turn graph from agent definition
function buildTurnGraph(definition: LocalAgentDefinition): ApxmGraph {
  return {
    name: `gemini-turn-${definition.name}`,
    nodes: [
      { id: 1, name: "guard", op: "GUARD", attributes: { max_turns: definition.maxTurns, timeout_ms: definition.maxTimeMinutes * 60000 } },
      { id: 2, name: "compress", op: "QMEM", attributes: { action: "check_compress" } },
      { id: 3, name: "ask", op: "ASK", attributes: { prompt: "{{current_message}}" } },
      { id: 4, name: "branch", op: "BRANCH_ON_VALUE", attributes: { on: "has_tool_calls" } },
      // ... tool INV nodes generated dynamically per response
      { id: 10, name: "wait", op: "WAIT_ALL", attributes: {} },
      { id: 11, name: "verify", op: "VERIFY", attributes: { claim: "tool_results" } },
      { id: 12, name: "reflect", op: "REFLECT", attributes: { check: "loop_detection" } },
    ],
    edges: [
      { from: 1, to: 2, dependency: "Control" },
      { from: 2, to: 3, dependency: "Control" },
      { from: 3, to: 4, dependency: "Data" },
      // ... dynamic edges for tool fan-out/fan-in
    ],
    parameters: [{ name: "current_message", type: "str" }],
    metadata: { source: "gemini-cli", version: "1.0" },
  };
}
```

### G12: Hooks as FENCE operations, recovery as TRY_CATCH (2 days)

**G12.1 Hooks --> FENCE mapping**

Gemini-CLI's 11 hook event types map to FENCE operations that enforce ordering in the graph:

| HookEventName | Graph Position | FENCE Semantics |
|--------------|---------------|-----------------|
| `SessionStart` | Before first GUARD | Session initialization barrier |
| `BeforeAgent` | Before GUARD | Pre-agent setup |
| `BeforeModel` | Before ASK/THINK | Pre-LLM barrier |
| `AfterModel` | After ASK/THINK | Post-LLM barrier |
| `BeforeToolSelection` | Before BRANCH_ON_VALUE | Pre-tool-selection barrier |
| `BeforeTool` | Before each INV | Pre-tool barrier |
| `AfterTool` | After each INV | Post-tool barrier |
| `PreCompress` | Before QMEM+UMEM | Pre-compression barrier |
| `Notification` | After any node | Event emission point |
| `AfterAgent` | After REFLECT | Post-agent cleanup |
| `SessionEnd` | After everything | Session teardown barrier |

FENCE operations do not change the computation -- they enforce ordering and provide hook injection points. The hook system continues to work, but hooks are now structurally positioned in the graph rather than scattered through imperative code.

**G12.2 Recovery turn --> TRY_CATCH subgraph**

`executeFinalWarningTurn()` becomes a TRY_CATCH subgraph:

```
Main turn graph fails (timeout, max_turns, protocol violation)
  --> TRY_CATCH boundary
    --> ASK("You have one final chance to complete the task...")
    --> GUARD(timeout=60s)  // grace period
    --> INV(complete_task)   // agent must call complete_task
  <-- TRY_CATCH returns result or null
```

**G12.3 Sub-agent delegation --> FLOW_CALL**

`LocalAgentExecutor` spawning a sub-agent becomes a **FLOW_CALL** operation. The sub-agent's turn graph is a separate AIS graph invoked by reference. The parent's AAM scope creates a child scope (Inherit policy) for the sub-agent.

### Phase 4 Summary

| Task | Days | Key Deliverable |
|------|------|----------------|
| G11: Turn graph construction | 4 | `executeTurn()` as AIS graph |
| G12: Hooks + recovery + delegation | 2 | FENCEs, TRY_CATCH, FLOW_CALL |
| **Total** | **6** | -- |

### Phase 4 Validation

- [ ] `executeTurn()` expressible as AIS graph with correct topology
- [ ] Parallel tool calls demonstrate automatic parallelism from DAG
- [ ] Hook events fire at correct graph positions via FENCE
- [ ] Recovery turn works as TRY_CATCH subgraph
- [ ] Sub-agent delegation works as FLOW_CALL
- [ ] No behavioral regression vs imperative implementation

---

## Phase 5: Compiler Integration (G13)

**Goal:** Compile Gemini-CLI's turn graph to optimized `.apxmobj` artifacts. The compiler analyzes the graph for parallelism, dead operations, and fusion opportunities. This is the punch line -- instead of a simple plan, you create an apxm-graph that gets compiled and analyzed.

**Prerequisite:** Phase 4 complete (turn loop is a graph).

### G13: Compile and optimize turn graphs (3 days)

**G13.1 Compile turn graph**

```bash
apxm compile gemini-turn.json -o gemini-turn.apxmobj -O2
```

The compiler produces an optimized `.apxmobj` artifact from the turn graph. Optimization passes include:

| Pass | Effect on Gemini-CLI Turn Graph |
|------|-------------------------------|
| **fuse-ask-ops** | If multiple sequential ASK calls exist (e.g., compression summary + main prompt), fuse into single API call |
| **cse** (Common Subexpression Elimination) | Eliminate duplicate tool invocations with identical arguments |
| **symbol-dce** (Dead Code Elimination) | Remove operations whose outputs are never consumed (e.g., REFLECT node when loop detection is disabled) |
| **Parallelism extraction** | Confirm N INV nodes with no inter-edges are dispatched concurrently |
| **Canonicalization** | Normalize graph patterns for consistent downstream optimization |

**G13.2 Compile-time validation**

The compiler catches structural errors before any LLM call:
- Missing edges (unreachable nodes) -- **implemented** (symbol-dce pass)
- DAG constraint violations (cycles) -- **implemented** (Kahn's algorithm in validate.rs)
- Dead operations that consume resources but produce unused output -- **implemented**
- Type mismatches (wrong attribute types for operations) -- **NOT YET IMPLEMENTED** (prerequisite for Phase 5)
- Missing required attributes per operation -- **NOT YET IMPLEMENTED** (prerequisite for Phase 5)

**GUARD wire index prerequisite:** The Gemini-CLI turn graph starts with GUARD, which currently has no wire index. Wire index 26 must be assigned before this graph can be compiled to `.apxmobj`. See Plan 1 A8.0 prerequisites.

This provides **49x faster error detection** (compile time vs runtime) -- errors caught in milliseconds instead of after expensive LLM API calls.

**G13.3 Artifact execution**

```bash
apxm run gemini-turn.apxmobj --emit-metrics metrics.json
```

The compiled artifact runs on the A-PXM dataflow scheduler with the same semantics as the interpreted graph but with optimized operation scheduling and pre-validated structure.

### Phase 5 Summary

| Task | Days | Key Deliverable |
|------|------|----------------|
| G13: Compile + optimize + validate | 3 | `.apxmobj` artifacts from Gemini-CLI turn graphs |
| **Total** | **3** | -- |

### Phase 5 Validation

- [ ] `apxm compile gemini-turn.json` produces valid `.apxmobj`
- [ ] FuseAskOps reduces API calls on workflows with sequential LLM calls
- [ ] Compile-time validation catches structural errors before any LLM call
- [ ] Compiled artifact execution produces identical results to uncompiled
- [ ] Optimization metrics (parallelism, fused ops, eliminated ops) reported

---

## Timeline

```
Phase 1 -- LLM Backend (Weeks 1-6)
  G1: TypeScript types                           Week 1
  G2: SSE client + service manager               Week 1-2
  G3: Event translator                           Week 2
  G4: Upgrade ApxmContentGenerator               Week 3-4
  G5: Integration + testing                      Week 4-6
  Validation with live apxm-server                Week 5-6

Phase 2 -- Tool Migration (Weeks 7-12)
  G6: Capability registration                    Week 7-9
  G7: Parallel calls + VERIFY                    Week 10-11
  G8: PolicyEngine as interceptors               Week 12

Phase 3 -- AAM State Model (Weeks 13-18)
  G9: AAM state mapping (B, G, C)                Week 13-16
  G10: Three-tier memory                         Week 17-18

Phase 4 -- Agent Loop as Graph (Weeks 19-24)
  G11: Turn graph construction                   Week 19-22
  G12: Hooks + recovery + delegation             Week 23-24

Phase 5 -- Compiler Integration (Weeks 25-28)
  G13: Compile + optimize + validate             Week 25-27
  Validation + measurement                       Week 28
```

---

## Risk Analysis

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| `apxm-server` binary packaging complexity | Medium | High | Start with manual `cargo install`, automate later |
| SSE client edge cases (reconnection, partial lines) | Medium | Medium | Thorough mock-server testing in G5 |
| Event translator mismatches (Rust vs TS types drift) | Medium | Medium | Schema endpoint (`/v1/schema`) + codegen |
| Tool registration overhead | Low | Medium | Thin adapter pattern (~10 LOC per tool) |
| Hierarchical AAM not yet implemented | High | Medium | Start with flat AAM (already works), add hierarchy in Phase 3 |
| Turn graph expressiveness gap | Medium | High | Start with simple single-ASK graphs, grow incrementally |
| MLIR dependency for compiler | Low | Medium | Compiler (Phase 5) is optional; Phases 1-4 use runtime only |
| Performance regression during migration | Low | High | Shadow mode comparison at each phase; graceful fallback in Phase 1 |
| Hook ordering changes in graph form | Medium | Medium | FENCE operations preserve existing hook semantics |

---

## Validation Criteria (All Phases)

### Phase 1 Complete:
- [ ] `npm run typecheck` passes with new types
- [ ] Event translator unit tests cover all 18 GeminiEventType mappings
- [ ] SSE client correctly parses multi-line SSE stream
- [ ] `GEMINI_CLI_USE_APXM=true gemini "hello"` works with apxm-server running
- [ ] Streaming: tokens appear incrementally in TUI (not all at once)
- [ ] Tool calls: function calls work through APXM path
- [ ] Thinking: thought events render in TUI
- [ ] Retry: 429 responses show retry indicator
- [ ] Fallback: graceful degradation when service is unavailable
- [ ] No regressions: existing Gemini API path unaffected

### Phase 2 Complete:
- [ ] All DeclarativeTool implementations registered as APXM capabilities
- [ ] MCP tools registered via APXM capability system
- [ ] Tool approval flows through APXM interceptor pipeline
- [ ] `~/.apxm/tools.toml` persists registrations

### Phase 3 Complete:
- [ ] Agent state inspectable: `apxm state show <session>`
- [ ] AAM (B, G, C) backed by file tree under `~/.apxm/workspaces/`
- [ ] Three-tier memory operational (STM, LTM, Episodic)

### Phase 4 Complete:
- [ ] `executeTurn()` expressible as AIS graph
- [ ] Automatic parallelism demonstrated on concurrent tool calls
- [ ] Hooks fire at correct graph positions via FENCE

### Phase 5 Complete:
- [ ] `apxm compile gemini-turn.json` produces optimized `.apxmobj`
- [ ] FuseAskOps reduces API calls on real workflows
- [ ] Compile-time validation catches structural errors before any LLM call

---

## Related Documents

- [Global Integration Plan](plan-global-integration.md) -- Five-phase adoption, architecture overview, AIS mapping
- [Plan 1: APXM Changes](plan-1-apxm-changes.md) -- All APXM-side deliverables
- [Plan 2: Codex Changes](plan-2-codex-changes.md) -- Codex-side changes (independent)
- [AAM: Agent Abstract Machine](../pxm/aam.md) -- Formal state model: `AAM = (B, G, C)`
- [Vision: The LLVM for Agents](../pxm/vision.md) -- File tree as AAM, CapabilityExecutor, compiler as punch line
- [Hierarchical AAM Diagrams](../implementation/runtime/hierarchical-aam.md) -- 14 diagrams: flat --> hierarchical migration
- [AIS Operations](../pxm/ais.md) -- 39 typed operations
- [Tools CLI Design](../cli/tools-cli-design.md) -- `apxm tool` command specification
