# Tracing, Streaming, MCP, and HITL Design

Production-framework features beyond MVP native Python tools.

---

## A. Tracing

### Current state

APXM already has a three-layer event system:

- **Core taxonomy** (`apxm-core/src/events/kind.rs`): 30+ typed `EventKind` constants across Stream, Lifecycle, Error, Observability, and UserAction categories.
- **Payload structs** (`apxm-core/src/events/payload.rs`): concrete payloads like `OperationStartPayload`, `ToolStartPayload`, `TokenUsagePayload`, each implementing `EventPayload` trait.
- **Emission pipeline** (`apxm-core/src/events/emitter.rs` + `sinks.rs`): `EventEmitter` trait with `NoOpEmitter`, `ChannelEmitter`, and `FanOutEmitter` sinks. The runtime bridges legacy `ExecutionEventEmitter` to core payloads via `EmitterAdapter` (`apxm-runtime/src/executor/emitter_adapter.rs`).

Each `ApxmEvent` envelope (`apxm-core/src/events/event.rs`) already carries `trace_id`, `seq`, `timestamp`, and `EventSource` — the same shape as an OpenAI span's `trace_id` + `span_id` + `started_at`/`ended_at`.

### Mapping to OpenAI span schema

The OpenAI SDK defines span types in `tracing/span_data.py`: `AgentSpanData`, `FunctionSpanData`, `GenerationSpanData`, `HandoffSpanData`, `GuardrailSpanData`, `MCPListToolsSpanData`, plus `TaskSpanData` and `TurnSpanData`. The mapping:

| OpenAI span type | APXM event(s) | Notes |
|---|---|---|
| `agent_span` | `SESSION_START` / `SESSION_END` | Per-agent scope; add `agent_name` field to `SessionStartPayload` |
| `generation_span` | `OPERATION_START` + `TOKEN` stream + `LLM_DONE` + `USAGE` | Correlate by `node_id`; `LlmDonePayload` already carries `model`, `usage`, `finish_reason` |
| `function_span` | `TOOL_START` / `TOOL_END` | 1:1 match; `ToolStartPayload.name` = function name |
| `handoff_span` | `MODEL_REROUTED` or new `HANDOFF` event kind | Need new `EventKind::HANDOFF` with `from_agent`, `to_agent` fields |
| `guardrail_span` | New `GUARDRAIL_CHECK` event kind | Attrs: `guardrail_name`, `triggered: bool`, `action: String` |
| `mcp_list_tools` | New `MCP_DISCOVERY` event kind | Phase 2; see section C |
| `turn_span` | `TURN_BOUNDARY` | Already exists with `turn_number` and `direction` |

### Span tree construction

OpenAI uses `contextvars` to track parent-child span nesting (`SpanImpl.__enter__` sets `Scope.set_current_span`; `spans.py:316-334`). APXM should build the tree server-side in the SSE consumer, not in the emitter. Every `ApxmEvent` already carries `trace_id`; add an optional `parent_span_id: Option<String>` field to `EventMeta` (in `apxm-core/src/events/event.rs`). The `EmitterAdapter` can set parent IDs using the execution stack — the `executor/handlers/` dispatch loop knows which operation is running at the point of emission.

### Python-side API

```
apxm.add_trace_processor(processor)
```

Mirrors `agents.add_trace_processor()` (`tracing/processors.py:32`). Processors receive `ApxmEvent` objects converted to Python dicts. Implementation: register callbacks in the Python frontend that receive events from the Rust runtime's `ChannelEmitter` sink, forwarded over the NDJSON tool-worker protocol as `{type: "trace_event", ...}` frames.

### Studio integration

`apxm-gui/frontend/src/types/events.ts` already defines `EventPayload` and `AgentStreamEvent` types that align closely with the core kinds. The existing `pass-pipeline-view.tsx` renders a compiler-pass waterfall; a new `trace-waterfall-view.tsx` component should render runtime spans as a nested timeline, consuming SSE events from `POST /v1/execute/stream` (`apxm-server/src/main.rs:604`). Group by `trace_id`, nest by `parent_span_id`, color by `EventCategory`.

### Phase 2: ingest endpoint

Add `POST /v1/traces/ingest` to `apxm-server` accepting the OpenAI span batch format (`{data: [{object: "trace.span", ...}]}`). This lets external OpenAI-native tracers push spans into Studio for side-by-side comparison of native vs. APXM-compiled runs (per `openai_agents_bridge.md`).

---

## B. Streaming tool output

### Current state

The tool-worker NDJSON protocol (see `native_python_tools.md`) supports a `v` (value) field in responses. The protocol is synchronous per request: one `invoke` frame, one `result` frame.

### Design

Extend the worker protocol with incremental `chunk` frames:

```
{"type": "chunk", "req_id": "...", "value": "partial output..."}
```

Sent zero or more times between the `invoke` request and the final `result` response. The runtime reads chunks as they arrive and emits them as a new event kind.

**New event kind:** Add `TOOL_CHUNK` to `apxm-core/src/events/kind.rs`:

```
pub const TOOL_CHUNK: EventKind = EventKind::new("tool_chunk", EventCategory::Stream, false);
```

With a `ToolChunkPayload { name: String, chunk: String }` in `payload.rs`. The runtime handler (`executor/handlers/inv_tool.rs`) emits `TOOL_CHUNK` for each chunk frame, then `TOOL_END` on the final result.

**AIS approach:** Add an `streaming: bool` attribute to `INV_TOOL` rather than a separate `STREAM_TOOL` op. This keeps tool invocation unified and avoids handler duplication. The attribute is set at compile time from `@apxm.tool(streaming=True)`.

**SSE forwarding:** `apxm-server` already forwards `StreamChunk::Token` events over SSE (`main.rs:1757`). `TOOL_CHUNK` events follow the same path — they get serialized as `ApxmEvent` envelopes and sent as SSE data frames. Studio renders them as live-updating tool output panels.

---

## C. MCP integration

### OpenAI approach

`mcp/server.py` defines `MCPServer` (abstract), `MCPServerStdio` (subprocess), `MCPServerSse` (HTTP+SSE), and `MCPServerStreamableHttp` (streamable HTTP). The core abstraction: `connect()` → `list_tools()` → `call_tool(name, args)` → `cleanup()`. Tool lists are optionally cached (`cache_tools_list`). Approval policies control which tools require human approval (`RequireApprovalSetting`).

### APXM design

**MVP (Phase 1): compile-time snapshot.** Provide `apxm.MCPServer(url_or_params)` in the Python frontend. At graph-capture time, the decorator calls `list_tools()` on the MCP server and generates one `@apxm.tool` wrapper per discovered tool. Each wrapper's `python_handler_id` points to a closure that calls `MCPServer.call_tool()` through the existing tool-worker subprocess bridge. The result: MCP tools are statically bound into the AIR graph, benefit from compiler validation (`tool-binding-check` pass), and execute through the standard `INV_TOOL` handler at runtime.

The Python tool worker (`apxm/tool_worker.py`) spawns the MCP client connection in-process. Tool calls arrive over NDJSON, the worker dispatches to `MCPServer.call_tool()`, and returns results. No new Rust code for MCP in Phase 1.

**Phase 2: runtime refresh.** Add an `MCP_REFRESH` AIS op that re-fetches the tool list from a running MCP server mid-execution. This handles servers whose tool lists change over time (e.g., dynamic plugin registries). The op emits `MCP_DISCOVERY` events for the trace waterfall.

**Approval integration:** MCP tools with `require_approval="always"` map directly to the HITL mechanism below — they set `approval_required=True` on the generated tool wrapper.

---

## D. HITL + RunState resumability

### OpenAI approach

`RunState` (`run_state.py:182`) is a `@dataclass` snapshot of an in-flight agent run: `_current_turn`, `_current_agent`, `_original_input`, plus serialized model responses and approval state. It enables pause/resume for human-in-the-loop flows, particularly MCP tool approvals (`mcp_approval_requested` / `mcp_approval_response` stream events in `stream_events.py:39-40`).

### APXM design

**RunState.** APXM already has `.apxmobj` artifacts (compiled graphs) but no in-flight state capture. Propose `apxm.RunState` that snapshots the execution DAG at every `COMMUNICATE` or `ASK` boundary — the points where the runtime blocks for external input. The snapshot includes: current node positions, accumulated context values, turn counter, and pending tool results.

**AIS op: `CHECKPOINT`.** Attributes: `scope` (node-local or graph-wide), `persist_to` (memory or disk). The runtime handler (`executor/handlers/checkpoint.rs`, already exists as a stub emitting `CheckpointSavedPayload`) gets extended to actually serialize the `ExecutionContext` to a `RunState` blob. The server exposes this through the existing `POST /v1/checkpoints` and `POST /v1/checkpoints/{id}/resume` endpoints (`apxm-server/src/main.rs`).

**Tool approval.** `@apxm.tool(approval_required=True)` compiles to `INV_TOOL` with an `approval_required: bool` AIS attribute. At runtime, the handler:

1. Emits `PAUSE_FOR_APPROVAL` event (new `EventKind`, category `UserAction`).
2. Creates a checkpoint with the tool call details.
3. Blocks on a `tokio::sync::oneshot` receiver.
4. An external `POST /v1/checkpoints/{id}/resume` call with `{"decision": "approved"}` triggers the oneshot, unblocking execution.

Studio renders `PAUSE_FOR_APPROVAL` events as an interactive approval card with Approve/Reject buttons. Rejection cancels the tool call and emits an `ErrorPayload` with `recoverable: true`.

**Python API:**

```python
state = await apxm.run(graph, input="...", return_state=True)
# ... human reviews output ...
result = await apxm.run_resume(state, human_input="approved")
```

Internally, `run_resume` loads the checkpoint from the server and sends the resume signal.

---

## Priority ordering (post-MVP tools)

1. **Tracing (A)** -- build first. Lowest implementation cost: extend `EventMeta` with `parent_span_id`, add two new event kinds (`HANDOFF`, `GUARDRAIL_CHECK`), build the Studio waterfall view. Tracing is prerequisite for debugging everything else and immediately useful for the side-by-side comparison demo.

2. **HITL + RunState (D)** -- build second. The checkpoint server endpoints and `PAUSE_FOR_APPROVAL` event kind are needed before MCP approval can work. The `checkpoint.rs` handler stub already exists. This unlocks multi-turn agent workflows and interactive demos.

3. **Streaming tool output (B)** -- build third. Requires only the `chunk` frame type in the NDJSON protocol plus `TOOL_CHUNK` event kind. Small surface area but depends on the tool-worker bridge being stable (MVP tools must land first).

4. **MCP integration (C)** -- build last. Phase 1 (compile-time snapshot) depends on the tool-worker bridge and benefits from HITL for approval flows. Phase 2 (runtime refresh) is a nice-to-have that can wait for real user demand.
