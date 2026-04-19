# Guardrails, Handoffs, and Retries

Design document for three production-framework features bridging OpenAI Agents SDK
patterns into the APXM compiler/runtime.

---

## A. Guardrails

OpenAI defines four guardrail kinds: input/output at both agent and tool level
(`src/agents/guardrail.py:72-185`, `src/agents/tool_guardrails.py:151-206`).
Each guardrail is a callable that returns a `GuardrailResult` with a
`tripwire_triggered` boolean and optional metadata.

### Mapping to APXM

APXM already has a **GUARD** operation (`crates/core/apxm-ais/src/operations/definitions.rs:1443-1465`)
with a runtime handler (`crates/runtime/apxm-runtime/src/executor/handlers/guard.rs`)
that evaluates simple condition expressions (`> 0.8`, `not_null`, `== approved`)
and either halts or skips on failure. However, GUARD is expression-based; it
cannot invoke arbitrary user-defined Python functions.

**Recommendation: add one new AIS operation, `GUARDRAIL_CHECK`.**

Rationale for a new op rather than reusing INV_TOOL with a sentinel attribute:

1. Guardrails have distinct semantics from tool calls -- they are gating checks,
   not capability invocations. Mixing them into INV_TOOL muddies the DAG
   semantics and makes compiler passes (e.g., dead-context-elimination) harder
   to reason about.
2. The operation needs a `mode` attribute (`terminate` vs `recover`) that has
   no INV_TOOL analogue.
3. Tracing and observability benefit from a dedicated op type -- guardrail
   evaluations should appear as their own span category in traces.

**GUARDRAIL_CHECK attributes:**

| Attribute | Required | Values |
|-----------|----------|--------|
| `guardrail_kind` | yes | `input_agent`, `output_agent`, `input_tool`, `output_tool` |
| `handler_id` | yes | Python handler identifier (same pattern as `python_handler_id` on REGISTER_CAPABILITY) |
| `mode` | no | `terminate` (default) or `recover` |

The existing `guardrail_kind` attribute constant is already defined at
`crates/core/apxm-ais/src/attrs.rs:77`.

**Tripwire semantics:**

- `mode=terminate`: handler returns `tripwire_triggered=true` -> runtime emits a
  `RuntimeError::GuardrailTripped` error. This flows into the existing TRYCATCH
  mechanism (`crates/runtime/apxm-runtime/src/executor/handlers/try_catch.rs`)
  if the graph wraps the guarded region.
- `mode=recover`: handler returns a recovery value that replaces the original
  input/output, and execution continues.

**Lowering:** `Agent(input_guardrails=[g1, g2])` desugars during graph capture to
GUARDRAIL_CHECK nodes inserted before the first ASK/LLM node in the agent's
subgraph. Output guardrails are inserted after the final LLM node.
Tool-level guardrails (`@apxm.tool(input_guardrails=[...])`) lower to
GUARDRAIL_CHECK nodes immediately before/after the corresponding INV_TOOL node.

**Interaction with GUARD:** The existing GUARD operation remains useful for
simple expression-based preconditions (threshold checks, null checks). It does
not need modification. GUARDRAIL_CHECK is for user-defined callable checks.

**New ops required: 1** (`GUARDRAIL_CHECK`).

---

## B. Handoffs

OpenAI's `Handoff` dataclass (`src/agents/handoffs/__init__.py:94-181`) models
delegation from one agent to another, exposed to the LLM as a tool so the
model can decide when to hand off.

### Mapping to APXM

APXM already has the two primitives needed:

- **SPAWN_AGENT** (`crates/runtime/apxm-runtime/src/executor/handlers/spawn_agent.rs`):
  creates a new agent in the process table or flow registry, supports ACP
  subprocess spawning with profile, mode, model, and cwd attributes.
- **COMMUNICATE** (`crates/runtime/apxm-runtime/src/executor/handlers/communicate.rs`):
  dispatches messages to agents via local sub-flow, HTTP, ACP, or broadcast
  protocols.

**Static handoffs** (user wires the delegation explicitly in the graph) require
no new ops. The pattern is: SPAWN_AGENT(target) -> COMMUNICATE(target, protocol=acp).
The `input_filter` from OpenAI's Handoff maps to COMMUNICATE's existing ability
to inject filtered message content into STM for the sub-flow.

**Authoring surface:**

```python
apxm.handoff(
    agent,
    tool_name_override="transfer_to_billing",
    on_handoff=callback,
    input_filter=filter_fn,
)
```

This lowers to: (1) a REGISTER_CAPABILITY node that exposes the handoff as a
tool the LLM can call, (2) an INV_TOOL node that triggers SPAWN_AGENT +
COMMUNICATE when the LLM selects it. The `on_handoff` callback executes as a
GUARDRAIL_CHECK(kind=input_tool) before the spawn.

**LLM-driven selection** ("give the model a list of agents and let it pick")
requires the model to see handoff targets as tool definitions. This is the
TOOL_DISPATCH pattern identified in the OpenAI agents bridge investigation
(`/home/raherrer/.claude/projects/-home-raherrer-projects-agents-apxm/memory/openai_agents_bridge.md`).

**Recommendation: defer SELECT_AGENT / TOOL_DISPATCH to a later phase.**
LLM-driven handoffs can be implemented today by registering each handoff as a
regular tool via REGISTER_CAPABILITY + INV_TOOL. The LLM sees N tools
(`transfer_to_billing`, `transfer_to_support`, etc.) and calls whichever one
it chooses. This is exactly how OpenAI's SDK implements it -- each Handoff
becomes a tool. A dedicated SELECT_AGENT op would only add value if we need
compiler-level optimization of agent selection (e.g., pruning unreachable
agents), which is a post-MVP concern.

**New ops required: 0.** Handoffs compose from existing SPAWN_AGENT,
COMMUNICATE, REGISTER_CAPABILITY, and INV_TOOL.

---

## C. Retries and Failure Behavior

### Existing Infrastructure

APXM's retry infrastructure is rated **Excellent** in the resilience assessment
(`/home/raherrer/.claude/projects/-home-raherrer-projects-agents-apxm/memory/error-handling-resilience-report.md`):

- `RetryConfig` with exponential backoff (500ms-60s), jitter, 3 max retries
  (`crates/runtime/apxm-backends/src/llm/retry/mod.rs`)
- `ErrorClass` enum: `Retryable` (timeout, 5xx, 429), `Permanent` (401, 403),
  `Unknown` (retry once)
- Per-backend health monitoring with Healthy/Degraded/Unhealthy states

However, this retry logic currently lives **only in the LLM backend layer**.
Tool invocations (INV_TOOL) and guardrails have no retry mechanism.

### Design

**Per-tool retries:**

```python
@apxm.tool(retries=3, backoff="exponential", failure_behavior="return_to_llm")
def search(query: str) -> str: ...
```

These attributes stamp onto the INV_TOOL node as AIS attributes: `retries`,
`backoff`, `failure_behavior`. The INV_TOOL handler
(`crates/runtime/apxm-runtime/src/executor/handlers/inv_tool.rs`) already has
timeout enforcement (line 129); retry logic wraps the same
`invoke_with_timeout` call using `RetryStrategy` from `apxm-backends`.

`failure_behavior` values:
- `raise` (default): propagate error up the DAG
- `return_to_llm`: return the error as a string value so the LLM can decide
  what to do next (matches OpenAI's tool error handling pattern)
- `recover`: call a user-defined recovery handler (via GUARDRAIL_CHECK with
  `mode=recover`)

**Per-agent failure policy:**

```python
Agent(failure_policy=FailurePolicy(max_retries=2, on_failure="escalate"))
```

This stamps onto the SPAWN_AGENT node. The runtime's executor engine
(`crates/runtime/apxm-runtime/src/executor/engine.rs`) already tracks
`NodeStatus` with `retries` and `last_error` fields. The enhancement is to
check `failure_policy` attributes before marking a node as permanently failed,
re-dispatching if retries remain.

**Reuse of existing infrastructure:**

- `RetryStrategy` and `RetryConfig` from `apxm-backends/src/llm/retry/mod.rs`
  should be promoted to `apxm-core` so they are available to all handlers,
  not just LLM backends.
- `ErrorClass` classification should be extended to cover tool errors (timeout
  -> Retryable, validation failure -> Permanent, unknown -> Unknown).
- The TRYCATCH handler (`crates/runtime/apxm-runtime/src/executor/handlers/try_catch.rs`)
  must be fixed first -- it currently passes through without catching (line 6-7).
  Retry-with-recovery depends on TRYCATCH actually working.

**New ops required: 0.** Retries are runtime behavior on existing INV_TOOL and
SPAWN_AGENT ops, configured via new AIS attributes (`retries`, `backoff`,
`failure_behavior`, `failure_policy`). The `RetryStrategy` type already exists.

---

## Summary: New AIS Operations

| Feature | New Op? | Rationale |
|---------|---------|-----------|
| Guardrails | **Yes: `GUARDRAIL_CHECK`** | Distinct semantics from GUARD (callable vs expression); needs `mode`, `handler_id`, `guardrail_kind` |
| Handoffs (static) | No | Composes from SPAWN_AGENT + COMMUNICATE |
| Handoffs (LLM-driven) | Deferred | Use REGISTER_CAPABILITY + INV_TOOL for MVP; dedicated SELECT_AGENT is post-MVP |
| Per-tool retries | No | New attributes on INV_TOOL; reuse `RetryStrategy` from apxm-backends |
| Per-agent failure | No | New attributes on SPAWN_AGENT; runtime engine checks before marking failed |

**Total new operations: 1** (`GUARDRAIL_CHECK`).

**Prerequisites:**
1. Fix TRYCATCH pass-through (`crates/runtime/apxm-runtime/src/executor/handlers/try_catch.rs:5-7`)
2. Promote `RetryStrategy`/`RetryConfig`/`ErrorClass` from `apxm-backends` to `apxm-core`
3. Add `python_handler_id` attribute support (Task #9, in progress)
