# Phase 1 LLM Backend — Manual Testing Guide

Tests span three repositories:

| Layer | Repository | Test Runner |
|-------|-----------|-------------|
| APXM Substrate | `~/projects/agents/apxm/` | `cargo test` |
| Consumer Adapter (Rust) | `~/projects/agents/openai/codex/` | `cargo test --features apxm-llm` |
| Consumer Adapter (TypeScript) | `~/projects/agents/google/gemini-cli/` | `npx vitest run` |

---

## Prerequisites

### 1. System Dependencies

```bash
# Required for Rust consumer's Linux sandbox:
sudo apt-get install libcap-dev   # Debian/Ubuntu
# or: sudo yum install libcap-devel  # RHEL/CentOS
```

### 2. APXM Build

```bash
cd ~/projects/agents/apxm
cargo build --workspace
# This builds apxm-server, the LLM backends, and credential store
export PATH="$PWD/target/release:$PATH"
```

### 3. Register at least one LLM credential

Credentials are stored in `~/.apxm/credentials.toml` (0600 permissions).
The `apxm-server` auto-loads all registered credentials at startup.

```bash
apxm llm add openai --provider openai --api-key $OPENAI_API_KEY
apxm llm add anthropic --provider anthropic --api-key $ANTHROPIC_API_KEY
apxm llm add google --provider google --api-key $GOOGLE_API_KEY
apxm llm list
apxm llm test
```

### 4. Consumer builds

```bash
# Rust consumer (requires libcap-dev for sandbox)
cd ~/projects/agents/openai/codex/codex-rs
cargo check -p codex-core                      # default build (no APXM)
cargo check -p codex-core --features apxm-llm  # with APXM adapter
cargo build --release --features apxm-llm -p codex-cli  # full CLI binary

# TypeScript consumer
cd ~/projects/agents/google/gemini-cli
npm install
npm run build        # build CLI for E2E testing
npm run typecheck    # or: npx tsc --noEmit
```

---

## Part A: APXM Substrate Tests

### A1. apxm-events — Serde Round-Trip (42 tests)

All 33 `EventPayload` variants serialize/deserialize through JSON, plus EventBus pub/sub.

```bash
cd ~/projects/agents/apxm
cargo test -p apxm-events -- --nocapture
```

**Expected:** 42 tests pass.

---

### A2. StreamAssembler — Fragment Assembly (8+ tests)

Tool-call accumulation, timeout flush, duplicate-start reset, implicit accumulators.

```bash
cargo test -p apxm-backends assembler -- --nocapture
```

**Expected:** All pass. Key behaviors:
- `ToolCallStart` + N x `ToolCallDelta` + `Done` → single `AssembledToolCall` with parsed JSON args
- Duplicate `ToolCallStart` resets accumulator
- `Error` flushes as `partial = true`
- Unparseable JSON → `Value::String` fallback
- 0ms timeout flushes on next `process()` call

---

### A3. Backend Streaming — SSE Parsing

```bash
cargo test -p apxm-backends --lib -- --nocapture
```

**Expected:** 101+ tests pass. Covers OpenAI, Anthropic, Google SSE parsing, StreamChunk variants.

---

### A4. EventBus Pub/Sub

```bash
cargo test -p apxm-events bus -- --nocapture
```

**Key behaviors:**
- No subscribers → `publish()` returns `Err` with event
- Multiple subscribers each get a clone
- `Lagged` error when behind (capacity 1024)
- `Closed` error when bus dropped

---

### A5. LLMRequest trace_id

```bash
cargo test -p apxm-backends trace_id -- --nocapture
```

**Verify:** `LLMRequest::new(...).with_trace_id("abc").trace_id()` returns `Some("abc")`.

---

### A6. Full APXM Workspace

```bash
cargo test --workspace 2>&1 | tail -20
```

**Expected:** All existing + new tests pass. No regressions.

---

## Part B: apxm-server Manual Testing

### B1. Start the server

```bash
cargo run -p apxm-server
# Default: http://localhost:9100
```

### B2. Health check

```bash
curl -s http://localhost:9100/health | jq .
```

### B2b. Model discovery

```bash
curl -s http://localhost:9100/v1/models | jq .
```

**Expected:** List of registered backends (from `~/.apxm/credentials.toml`):
```json
{
  "object": "list",
  "data": [
    { "id": "openai", "object": "model", "created": 0, "owned_by": "apxm" },
    { "id": "anthropic", "object": "model", "created": 0, "owned_by": "apxm" }
  ]
}
```

**Verify:** The list matches `apxm llm list` output.

### B3. Schema endpoint

```bash
curl -s http://localhost:9100/v1/schema | jq .
```

### B4. Non-streaming generate

```bash
curl -s http://localhost:9100/v1/generate \
  -H "Content-Type: application/json" \
  -H "X-Trace-ID: test-trace-001" \
  -d '{
    "messages": [{"role": "user", "content": "Say hello in one word"}],
    "model": "gpt-4o-mini",
    "backend": "openai"
  }' | jq .
```

**Verify:** Response contains `content`, `model`, `finish_reason`, `usage`, `trace_id`.

### B5. Trace-ID propagation

```bash
curl -s http://localhost:9100/v1/generate \
  -H "X-Trace-ID: my-custom-trace" \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"hi"}],"model":"gpt-4o-mini","backend":"openai"}' \
  | jq '.trace_id'
# Expected: "my-custom-trace"
```

### B6. Streaming generate (SSE)

```bash
curl -N http://localhost:9100/v1/generate-stream \
  -H "Content-Type: application/json" \
  -H "X-Trace-ID: stream-test-001" \
  -d '{
    "messages": [{"role": "user", "content": "Count from 1 to 5"}],
    "model": "gpt-4o-mini",
    "backend": "openai"
  }'
```

**Expected SSE wire format:**
```
event: apxm
data: {"meta":{"seq":1,"timestamp_ms":...,"trace_id":"stream-test-001","source":"backend"},"payload":{"kind":"token","text":"1"}}

event: apxm
data: {"meta":{"seq":N,...},"payload":{"kind":"done","content":"...","finish_reason":"stop",...}}
```

**Verify:**
- Tokens arrive incrementally (not buffered)
- Every line prefixed with `event: apxm`
- `data:` is valid JSON with `meta` + `payload`
- `meta.trace_id` matches `X-Trace-ID` header
- `meta.seq` increments monotonically
- Stream ends with `kind: "done"` payload

### B7. Streaming with tool calls

```bash
curl -N http://localhost:9100/v1/generate-stream \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "What files are in the current directory?"}],
    "model": "gpt-4o-mini",
    "backend": "openai",
    "tools": [{
      "name": "bash",
      "description": "Run a bash command",
      "parameters": {
        "type": "object",
        "properties": {"command": {"type": "string"}},
        "required": ["command"]
      }
    }]
  }'
```

**Expected stream events:**
1. `kind: "tool_call_start"` — `id` + `name`
2. `kind: "tool_call_delta"` (1 or more) — argument fragments
3. `kind: "done"` — `tool_calls` array with assembled call

### B8. Multi-provider

```bash
# Anthropic
curl -s http://localhost:9100/v1/generate \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"Say hello"}],"model":"claude-sonnet-4-20250514","backend":"anthropic"}' | jq .

# Google
curl -s http://localhost:9100/v1/generate \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"Say hello"}],"model":"gemini-2.0-flash","backend":"google"}' | jq .
```

### B9. Extended thinking

```bash
curl -N http://localhost:9100/v1/generate-stream \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "Think step by step: what is 17 * 23?"}],
    "model": "claude-sonnet-4-20250514",
    "backend": "anthropic",
    "thinking": {"enabled": true, "budget_tokens": 1000}
  }'
```

**Expected:** `kind: "thought"` events appear before `kind: "token"` events.

### B10. Error handling

```bash
# Invalid backend
curl -s http://localhost:9100/v1/generate \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"hi"}],"model":"x","backend":"nonexistent"}' | jq .

# Empty messages
curl -s http://localhost:9100/v1/generate \
  -H "Content-Type: application/json" \
  -d '{"messages":[],"model":"gpt-4o-mini","backend":"openai"}' | jq .
```

---

## Part C: Rust Consumer Adapter Tests (53 tests)

The Rust consumer adapter is feature-gated behind `apxm-llm`.

### C1. Feature gate isolation

```bash
cd ~/projects/agents/openai/codex

# Without feature — must compile clean, no APXM code
cargo check -p codex-core
```

**Expected:** Compiles with zero APXM-related code.

### C2. Adapter unit tests (all 3 modules)

```bash
cargo test -p codex-core --features apxm-llm -- apxm_adapter --nocapture
```

This runs tests across:
- **event_translator** (3 tests): `StreamChunk` → `CodexApxmEvent` mapping for Token, ToolCallStart, ToolCallDelta
- **tests.rs** (42 tests):
  - Event translator: all 7 StreamChunk variants, all finish reasons, empty content, unicode
  - Request translator: system prompt, config overrides, trace_id propagation
  - Notification bridge integration: full round-trip StreamChunk → notification
  - Type verification: Clone, PartialEq
- **notification_bridge** (8 tests): TextDelta → AgentMessageDelta, ReasoningDelta → ReasoningSummaryTextDelta, UsageUpdate → ThreadTokenUsageUpdated, ResponseCompleted/ToolCallStarted/Error → None

**Expected:** 53 tests pass.

### C3. Key behaviors to verify

```
StreamChunk::Token("hello")    → CodexApxmEvent::TextDelta { text: "hello" }
StreamChunk::Thought("hmm")   → CodexApxmEvent::ReasoningDelta { text: "hmm" }
StreamChunk::Done(response)    → CodexApxmEvent::ResponseCompleted {
                                     response_id: None,
                                     finish_reason: "stop",  // Display, not Debug
                                     ...
                                 }
StreamChunk::Usage(usage)      → CodexApxmEvent::UsageUpdate(CodexTokenUsage { ... })
StreamChunk::Error("fail")     → CodexApxmEvent::StreamError { message: "fail" }
```

### C4. Shadow mode function

The `compare_outputs()` function uses whitespace tokenization to compute divergence:
```
divergence = |apxm_count - legacy_count| / max(legacy_count, 1)
```
Logs a warning when divergence > 1%.

---

## Part D: TypeScript Consumer Adapter Tests (25+ tests)

### D1. Type check

```bash
cd ~/projects/agents/google/gemini-cli
npm run typecheck
```

**Expected:** No type errors in the new `packages/core/src/core/apxm/` module.

### D2. Event translator unit tests

```bash
cd packages/core
npx vitest run src/core/apxm/event-translator.test.ts
```

Runs 25+ test cases across two describe blocks:

**`toGeminiEvent` (16 tests):**
- ContentDelta → Content (with/without trace_id)
- ThinkingDelta → Thought (plain text + bold-subject parsing)
- ToolCallDelta → ToolCallRequest (valid JSON, invalid JSON → `_raw`, empty args)
- ModelInfo → ModelInfo
- Error → Error (with status, without status, with code)
- Done → Finished (stop, length/MAX_TOKENS, undefined → STOP)
- UsageInfo → null

**`mapFinishReason` (9 tests):**
- stop/end_turn → STOP
- tool_use/tool_calls → TOOL_USE
- length/max_tokens → MAX_TOKENS
- content_filter/safety → SAFETY
- Case-insensitive matching
- undefined → STOP
- Unknown → OTHER

### D3. Key behaviors to verify

```
ApxmEventType.ContentDelta  → GeminiEventType.Content
ApxmEventType.ThinkingDelta → GeminiEventType.Thought
ApxmEventType.ToolCallDelta → GeminiEventType.ToolCallRequest
ApxmEventType.Done          → GeminiEventType.Finished
ApxmEventType.Error         → GeminiEventType.Error
ApxmEventType.ModelInfo     → GeminiEventType.ModelInfo
ApxmEventType.UsageInfo     → null (consumed separately as metadata)
```

### D4. SSE client smoke test

With `apxm-server` running (Part B1):

```bash
GEMINI_CLI_USE_APXM=true npx vitest run --grep "apxm"
```

Or manually verify the SSE client connects:
```bash
# In a Node REPL or test script:
# import { ApxmServiceClient } from './packages/core/src/core/apxm/client.js'
# const client = new ApxmServiceClient({ baseUrl: 'http://localhost:9100' })
# console.log(await client.isHealthy())  // true
```

### D5. Fallback behavior

Set `GEMINI_CLI_USE_APXM=true` but with apxm-server NOT running. The content generator should:
1. Attempt APXM connection
2. Fail gracefully
3. Fall back to direct provider dispatch
4. Log a warning (not crash)

---

## Part E: Rust Consumer — E2E Workflow Testing

These tests run the actual consumer CLI end-to-end through the APXM backend,
exercising the full path: CLI → `ApxmModelClient` → `LLMRegistry` → LLM provider → streaming response.

**How it works:** When `CODEX_USE_APXM=1` is set (or `[apxm_backend] enabled = true` in config),
the Codex CLI's `try_run_sampling_request()` dispatches through `run_apxm_sampling_request()`,
which creates an `ApxmModelClient`, calls `stream_turn()`, and processes events through
`process_apxm_events()` — delivering text deltas, reasoning, usage, and errors as `EventMsg` to
the TUI/JSON-RPC clients.

### Prerequisites

1. Build apxm-server and put it on PATH (for credential auto-loading):

```bash
cd ~/projects/agents/apxm
cargo build --release -p apxm-server
export PATH="$PWD/target/release:$PATH"
```

2. Build the consumer CLI with APXM support (requires `libcap-dev`):

```bash
cd ~/projects/agents/openai/codex/codex-rs
cargo build --release --features apxm-llm -p codex-cli
export PATH="$PWD/target/release:$PATH"
```

3. Register LLM credentials (if not already done):

```bash
apxm llm add openai --provider openai --api-key $OPENAI_API_KEY
apxm llm test openai
```

### E1. Simple prompt

```bash
CODEX_USE_APXM=1 codex "Say hello in one word"
```

**Verify:**
- Response appears with streamed tokens (not all at once)
- Output is coherent (e.g., "Hello" or "Hi")
- No errors or stack traces

### E2. Tool execution

```bash
CODEX_USE_APXM=1 codex "List the files in /tmp and tell me how many there are"
```

**Verify:**
- The agent requests a tool call (e.g., `bash` with `ls /tmp`)
- Tool approval prompt appears (or auto-approved if configured)
- Tool output is incorporated into the response
- Final answer includes the file count

### E3. Multi-turn conversation

```bash
CODEX_USE_APXM=1 codex
# Then interactively:
# > What is 2 + 2?
# (wait for answer)
# > Now multiply that by 3
# (wait for answer — should say 12)
```

**Verify:**
- Context is preserved between turns
- Second answer references the first result
- Streaming works on each turn

### E4. Shadow mode (compare APXM vs legacy)

Add to `~/.codex/config.toml`:

```toml
[apxm_backend]
enabled = true
shadow_mode = true
```

```bash
codex "Explain what /etc/hosts does in one sentence"
```

**Verify:**
- Both APXM and legacy paths execute
- Check logs for `compare_outputs` divergence report
- Warning appears only if divergence > 1%

### E5. Extended thinking (Anthropic backend)

```bash
apxm llm add anthropic --provider anthropic --api-key $ANTHROPIC_API_KEY
CODEX_USE_APXM=1 codex "Think step by step: what is 17 * 23?"
```

**Verify:**
- Reasoning/thinking tokens appear before the final answer
- Final answer is correct (391)

### E6. Error recovery

```bash
# With no credentials registered for a backend
CODEX_USE_APXM=1 codex "Say hello"
```

**Verify:**
- Error is reported gracefully (not a panic or stack trace)
- If shadow mode is on, legacy path still works

---

## Part F: TypeScript Consumer — E2E Workflow Testing

**How it works:** When `GEMINI_CLI_USE_APXM=true` is set, the CLI's `ApxmContentGenerator`
spawns an `apxm-server` child process via `ApxmServiceManager`, health-checks it, and routes
requests through `ApxmServiceClient` (SSE over HTTP). The server auto-loads credentials from
`~/.apxm/credentials.toml` at startup, so any model registered via `apxm llm` is available.

### Prerequisites

1. APXM server binary on PATH (same as Part E).

2. Build the consumer CLI:

```bash
cd ~/projects/agents/google/gemini-cli
npm install
npm run build
```

3. The consumer CLI auto-starts `apxm-server` as a managed child process
   (no need to start it separately). It finds a free port, spawns the server,
   health-checks it, and connects. The server loads registered credentials
   automatically.

### F1. Simple prompt

```bash
cd ~/projects/agents/google/gemini-cli
GEMINI_CLI_USE_APXM=true node packages/cli/dist/index.js "Say hello in one word"
```

**Verify:**
- `apxm-server` starts automatically (check process list if desired)
- Response appears with streamed tokens
- Output is coherent
- Server shuts down when CLI exits

### F2. Tool execution

```bash
GEMINI_CLI_USE_APXM=true node packages/cli/dist/index.js \
  "What files are in /tmp? List them."
```

**Verify:**
- Agent requests a tool call
- Tool output is incorporated into the response
- SSE stream contains `tool_call_start` → `tool_call_delta` → `done` events

### F3. Multi-turn conversation

```bash
GEMINI_CLI_USE_APXM=true node packages/cli/dist/index.js
# Interactive session:
# > What is the capital of France?
# (wait for answer)
# > What country is it the capital of?
# (should reference Paris and France)
```

**Verify:**
- Context preserved between turns
- Streaming works on each turn

### F4. Thinking mode

```bash
GEMINI_CLI_USE_APXM=true node packages/cli/dist/index.js \
  "Think step by step: what is 17 * 23?"
```

**Verify:**
- Thought events appear before the final content
- Final answer is correct (391)

### F5. Fallback behavior (apxm-server unavailable)

Remove `apxm-server` from PATH temporarily:

```bash
PATH_BACKUP="$PATH"
export PATH=$(echo "$PATH" | tr ':' '\n' | grep -v apxm | tr '\n' ':')

GEMINI_CLI_USE_APXM=true node packages/cli/dist/index.js "Say hello"

export PATH="$PATH_BACKUP"
```

**Verify:**
- Warning logged: `Failed to start apxm-server, falling back to direct dispatch`
- Response still arrives (via direct provider dispatch)
- No crash or unhandled exception
- Requires a direct provider API key (e.g., `GEMINI_API_KEY`) for fallback

### F6. Provider switching

```bash
# OpenAI through APXM
GEMINI_CLI_USE_APXM=true APXM_LLM_BACKEND=openai \
  node packages/cli/dist/index.js "Say hello"

# Anthropic through APXM
GEMINI_CLI_USE_APXM=true APXM_LLM_BACKEND=anthropic \
  node packages/cli/dist/index.js "Say hello"
```

**Verify:**
- Different models/providers produce responses
- Model name in response metadata changes accordingly

### F7. Mid-stream failure recovery

Start `apxm-server` manually, then kill it during a streaming response:

```bash
# Terminal 1: Start server
apxm-server --port 9100 &
SERVER_PID=$!

# Terminal 2: Start a long streaming request
GEMINI_CLI_USE_APXM=true APXM_SERVER_PORT=9100 \
  node packages/cli/dist/index.js "Write a 500-word essay about the ocean"

# Terminal 1: Kill server mid-stream (after a few tokens appear)
kill $SERVER_PID
```

**Verify:**
- Warning logged about APXM service failure
- CLI falls back to direct dispatch for remainder
- No crash or data corruption

---

## Full Checklist

### APXM Substrate
| # | Test | Command | Pass? |
|---|------|---------|-------|
| A1 | apxm-events serde (42) | `cargo test -p apxm-events` | |
| A2 | StreamAssembler (8+) | `cargo test -p apxm-backends assembler` | |
| A3 | Backend streaming (101+) | `cargo test -p apxm-backends --lib` | |
| A4 | EventBus pub/sub | `cargo test -p apxm-events bus` | |
| A5 | trace_id on LLMRequest | `cargo test -p apxm-backends trace_id` | |
| A6 | Full workspace | `cargo test --workspace` | |

### apxm-server
| # | Test | Method | Pass? |
|---|------|--------|-------|
| B1 | Server starts | `cargo run -p apxm-server` | |
| B2 | Health check | `curl /health` | |
| B3 | Schema | `curl /v1/schema` | |
| B4 | Non-streaming | `curl /v1/generate` | |
| B5 | Trace-ID | `curl -H X-Trace-ID` | |
| B6 | Streaming SSE | `curl -N /v1/generate-stream` | |
| B7 | Tool calls | SSE with tools | |
| B8 | Multi-provider | OpenAI + Anthropic + Google | |
| B9 | Extended thinking | Anthropic with thinking | |
| B10 | Error handling | Invalid backend / empty msgs | |

### Rust Consumer Adapter
| # | Test | Command | Pass? |
|---|------|---------|-------|
| C1 | Feature gate isolation | `cargo check -p codex-core` (no feature) | |
| C2 | Adapter tests (53) | `cargo test -p codex-core --features apxm-llm -- apxm_adapter` | |

### TypeScript Consumer Adapter
| # | Test | Command | Pass? |
|---|------|---------|-------|
| D1 | Type check | `npm run typecheck` | |
| D2 | Event translator (25+) | `npx vitest run event-translator.test.ts` | |
| D4 | SSE client smoke | `GEMINI_CLI_USE_APXM=true` with server running | |
| D5 | Fallback | `GEMINI_CLI_USE_APXM=true` without server | |

### E2E Workflow (Rust Consumer)
| # | Test | Method | Pass? |
|---|------|--------|-------|
| E1 | Build with APXM | `cargo build --features apxm-llm -p codex-cli` | |
| E2 | Simple prompt | `CODEX_USE_APXM=1 codex "Say hello"` | |
| E3 | Tool execution | `CODEX_USE_APXM=1 codex "List files in /tmp"` | |
| E4 | Streaming output | Tokens arrive incrementally | |
| E5 | Shadow mode | `shadow_mode = true` in config, check logs | |
| E6 | Multi-turn | Interactive session with follow-up | |

### E2E Workflow (TypeScript Consumer)
| # | Test | Method | Pass? |
|---|------|--------|-------|
| F1 | Build CLI | `npm install && npm run build` | |
| F2 | Simple prompt | `GEMINI_CLI_USE_APXM=true gemini "Say hello"` | |
| F3 | Tool execution | `GEMINI_CLI_USE_APXM=true gemini "What files are in /tmp?"` | |
| F4 | Streaming output | Tokens arrive incrementally | |
| F5 | Thinking mode | Prompt requiring reasoning | |
| F6 | Fallback (no server) | Start without apxm-server on PATH | |
| F7 | Multi-turn | Interactive session with follow-up | |
