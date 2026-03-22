# Phase 1: Gemini-CLI Changes -- LLM Backend

**v7 -- Revised with gap analysis**

**Timeline:** Weeks 1-6 (7 days estimated)
**Depends on:** [APXM Phase 1 (A0-A4)](apxm.md) -- specifically `apxm-server` binary with HTTP+SSE endpoints
**Does NOT depend on:** [Codex Phase 1](codex.md) -- these are independent
**Global plan:** [Plan 3: Gemini-CLI Changes (all phases)](../plan-3-gemini-changes.md)

---

## Current Architecture (Key Files)

| File | Role | Lines |
|------|------|-------|
| `core/src/core/apxmContentGenerator.ts` | Multi-provider LLM adapter (OpenAI, Anthropic, Google, Ollama) | 953 |
| `core/src/core/apxmConfig.ts` | Config loading from `~/.apxm/{config,credentials}.toml` | 583 |
| `core/src/core/contentGenerator.ts` | `ContentGenerator` interface + auth type detection | 320 |
| `core/src/core/turn.ts` | `Turn.run()` -- streams `ServerGeminiStreamEvent` (18 types), `GeminiEventType` enum | 447 |
| `core/src/core/geminiChat.ts` | `GeminiChat` -- conversation management, compression, retry | 1075 |
| `core/src/agents/local-executor.ts` | `LocalAgentExecutor` -- `executeTurn()` (line 316), `executeFinalWarningTurn()`, `processFunctionCalls()` | 1479 |
| `core/src/scheduler/scheduler.ts` | Tool scheduling, parallel execution, confirmation flow | ~400 |
| `core/src/scheduler/types.ts` | `CoreToolCallStatus` (7 states), `ToolCallRequestInfo`, `ToolCallResponseInfo` | 207 |
| `core/src/scheduler/policy.ts` | `checkPolicy()` -- PolicyEngine integration for tool approval | ~100 |
| `core/src/scheduler/confirmation.ts` | Tool confirmation flow -- `resolveConfirmation()` | ~100 |
| `core/src/hooks/types.ts` | `HookEventName` enum (11 events) | ~60 |
| `core/src/hooks/hookSystem.ts` | Hook aggregation and dispatch | ~200 |
| `core/src/tools/tools.ts` | `DeclarativeTool`, `ToolInvocation`, `ToolCallConfirmationDetails` | ~400 |
| `core/src/tools/tool-registry.ts` | `ToolRegistry` -- tool discovery and lookup | ~200 |
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

## Gemini-CLI Operations to AIS Mapping

Every operation Gemini-CLI performs today has a direct AIS equivalent. This mapping is the blueprint for all five phases:

| Gemini-CLI Operation | Current Implementation | AIS Equivalent | AAM Effect | Phase |
|---------------------|----------------------|---------------|------------|-------|
| LLM call (prompt to response) | `ContentGenerator.generateContentStream()` | **ASK** / **THINK** | Reads B, writes B | 1 |
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

## G1: TypeScript types for APXM events (1 day)

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

---

## G2: SSE client for `apxm-server` (1.5 days)

Two new files:

### `google/gemini-cli/packages/core/src/core/apxm/client.ts` (~100 lines)

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
  trace_id?: string;  // propagated to EventMeta.trace_id on all response events
}
```

The `generateStream` method sends `trace_id` in the request body (or sets `X-Trace-ID` header when using HTTP transport in Phase 4+). If not provided, the server generates a UUID v4.

### `google/gemini-cli/packages/core/src/core/apxm/service-manager.ts` (~80 lines)

`ApxmServiceManager` -- sidecar lifecycle management:
- Discovers binary at `$APXM_HOME/bin/apxm-server` (default: `~/.apxm/bin/apxm-server`)
- `start()` -- spawns process, waits for health check (10s timeout, 500ms poll interval)
- `stop()` -- sends SIGTERM, waits 5s, then SIGKILL if still alive
- Port default: 9100

#### Port conflict handling

If port 9100 is in use:
1. Check if existing process is a healthy `apxm-server` (GET `/v1/health`) -- if so, reuse it
2. If not an `apxm-server` or unhealthy, try ports 9101-9109 in sequence
3. If all ports exhausted, fall back to direct provider dispatch with warning

#### Multi-instance coordination

Multiple Gemini-CLI instances may start concurrently. To avoid race conditions:
- Use a file lock at `~/.apxm/run/apxm-server.lock` before spawning
- Lock holder writes `{ "pid": <pid>, "port": <port> }` to `~/.apxm/run/apxm-server.json`
- Non-holders read the JSON and connect to the existing instance
- On process exit, release lock and clean up JSON file

---

## G3: Event translator (pure function) (1 day)

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

---

## G4: Upgrade `ApxmContentGenerator` (2 days)

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

---

## G5: Integration and testing (1.5 days)

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

---

## What Is NOT Changed (Phase 1)

These Gemini-CLI systems are untouched in Phase 1:
- **Tool execution** -- DeclarativeTool, ToolRegistry, MessageBus, PolicyEngine
- **MCP support** -- stdio/SSE/HTTP MCP connections
- **Scheduler** -- tool scheduling, parallel execution, tool state machine (Validating --> Scheduled --> AwaitingApproval --> Executing --> Success/Error/Cancelled)
- **TUI** -- React Ink components, output formatting
- **Chat compression** -- ChatCompressionService
- **Hooks** -- 11 hook event types
- **Auth** -- OAuth, API key, Vertex AI, Compute ADC (APXM replaces these with its own credential store)
- **Turn class** -- `Turn.run()` stays the same; it receives `ServerGeminiStreamEvent` from wherever they come from

---

## Phase 1 Summary

| Task | Days | New Files | Modified Files | New Lines | Deleted Lines |
|------|------|-----------|----------------|-----------|---------------|
| G1: TypeScript types | 1 | 1 | 0 | ~200 | 0 |
| G2: SSE client + service manager | 1.5 | 2 | 0 | ~180 | 0 |
| G3: Event translator | 1 | 1 | 0 | ~120 | 0 |
| G4: Upgrade generator | 2 | 0 | 1 | ~100 | ~400 |
| G5: Integration + tests | 1.5 | 1 | 1 | ~100 | 0 |
| **Total** | **7** | **5** | **2** | **~700** | **~400** |

**Net change:** +300 lines (700 added, 400 deleted from provider dispatch)

### Migration Timeline

```
Phase 1 -- LLM Backend (Weeks 1-6)
  G1: TypeScript types                           Week 1
  G2: SSE client + service manager               Week 1-2
  G3: Event translator                           Week 2
  G4: Upgrade ApxmContentGenerator               Week 3-4
  G5: Integration + testing                      Week 4-6
  Validation with live apxm-server          Week 5-6
```

---

## Related Documents

- [Phase 1 Overview](README.md)
- [Investigation Report](INVESTIGATION.md)
- [APXM Phase 1 (A0-A4)](apxm.md)
- [Codex Phase 1 (C1-C4)](codex.md)
- [Plan 3: Gemini-CLI Changes (all phases)](../plan-3-gemini-changes.md)
- [Global Integration Plan](../plan-global-integration.md)
