# Conversational Agent — Specification-Driven Development

Status: spec draft v2 · 2026-06-15 · companion to `docs/apxm-cli-agent-vision.md`
(strategy). v2 supersedes v1's "Model A" framing: the target is now **the entire
agent in one program** (Model B). Every claim below is grounded in a verified,
adversarially-checked audit (file:line throughout).

## 0. The one principle

> The **entire** conversational agent lives in **one APXM program**: the
> conversation **loop**, each user turn, context management + compaction,
> **pre/post/workflow-start hooks**, skill/tool discovery, and sub-agent
> spawning. The host (`apxm chat` / server) is **dumb I/O transport** — deliver
> the user message in, stream tokens out. No cognition, no policy, no loop in the
> host.

This is the apxm-os boundary applied to chat: *host = transport, graph =
cognition* — a boundary apxm-os already ships (cue POST in → graph runs). The
conversational agent must reach the same shape. **Compiler and runtime changes
are in scope** where the substrate genuinely requires them.

A capability that exists in Rust but cannot be authored from the program and run
on the **server path** (`/v1/execute/stream`) is, for this spec, not delivered.

---

## 1. Verified baseline

Two audits (10 deep-readers, every blocker adversarially verified) established:

**A. The turn body is authorable today; the loop and cross-turn policy are not.**
The runtime is single-shot ("one DAG = one turn", `chat.rs:6-8`); the host
(`chat.rs`, 1199 lines) owns the loop (`chat.rs:332`), the transcript
(`Conversation`, `chat.rs:99-168`), compaction (a host post-hook making its own
`SUMMARIZE_AIR` call, `chat.rs:735-779`), grants (`chat.rs:284`), turn caps
(`chat.rs:315`). The built-in agent graph is literally one `ais.ask` over a
`conversation` param (`apxm-ais/src/chat.rs:185`).

**B. The in-graph loop half-exists but traps the turn body in Rust.**
`autonomous.rs` has `converse` mode (loops a pre-supplied turn list,
`autonomous.rs:256-300`) and `recv` mode (one turn per event, re-arms,
`autonomous.rs:370-452`). Both call the hardcoded `run_agent_turn`
(`autonomous.rs:305` = persona + growing `String` transcript + one ASK). The
author cannot inject recall/umem/hooks/compaction/delegate into the turn.

**C. `recv` busy-polls; it does not park.** `recv_loop` uses `poll_once` +
`tokio::sleep` (`autonomous.rs:425,446`), pinning a worker per conversation.

**D. The real park/wake keystone ships and is proven.** PAUSE returns
`RuntimeError::OperationParked{wait_key}` (`pause.rs:115`); the worker yields
lane+permit and releases its admission slot (`worker.rs:201-223`); `park_registry::wake`
delivers a value and resumes (`park_registry.rs:74`). Blocking waits route to a
separate 4096-slot pool so they never starve compute (`worker.rs:96-102`).
`splice_dag` grafts new nodes into a **live** execution (`splicing.rs:71`),
already used for PLAN and SWITCH.

**E. Multi-agent is multi-DAG below the frontend.** The artifact wire format is
already multi-DAG (`ArtifactEmitter.cpp:644-672`, walks all `func.func`);
`reconstruct_agents_from_artifact` registers all flows (`runtime.rs:1181`);
DELEGATE resolves by name (`delegate.rs:42`). Only the **frontend** emits a
single `func.func` (`ir.py:247`), and `_sanitize_name` eats the dot in
`Agent.flow` (`ir.py:258`).

**F. The three hook surfaces are real but program-unreachable.**
`CapabilityInterceptor` (Allow/Deny/EditArgs, `interceptor.rs:9-33`) is honored
at the capability chokepoint but **bypassed by both Python-tool dispatch sites**
(`tool_dispatch.rs:281` and `inv_tool.rs:262`). `OperationMiddleware`
(pre/post/short-circuit, `middleware.rs:50`) is host-registered only.
`ExecutionHook` (`hooks.rs:12`) fires on the server path but has **zero
registrations** and its setters have **zero callers** — dead. `PythonToolBridge`
dispatches any handler_id and runs on the server path (`python_tools/mod.rs:73`).

---

## 2. The executable spec: one self-contained program

The spec is defined **by this program** — the acceptance fixture, to live at
`examples/python/conversational/controllable_agent.py`. "Done" = it compiles to
**one artifact**, the host is a dumb pipe, and the loop + every hook + sub-agent
fires on `/v1/execute/stream`.

```python
#!/usr/bin/env python3
"""controllable_agent.py — the whole agent in one program."""

from apxm import ConversationalAgent, CompactionPolicy, hook, tool, Agent

# ---- tools & sub-agents (server-path callable via PythonToolBridge) ----------
@tool
def lookup(symbol: str) -> str:
    """Look up a ticker price."""
    ...

researcher = Agent(name="researcher", instructions="Gather supporting facts.")

# ---- hooks: pre / post / workflow-start, ALL python callables ----------------
@hook(on="workflow_start")            # gate-capable; runs once at session start
def announce(ctx):
    ctx.log(f"session start; budget={ctx.remaining_budget}")

@hook(on="pre_tool", match="lookup")  # Allow / Deny / EditArgs
def guard_lookup(ctx, call):
    if call.args["symbol"] == "FORBIDDEN":
        return ctx.deny("symbol not permitted")
    return ctx.edit_args({**call.args, "symbol": call.args["symbol"].upper()})

@hook(on="post_tool", match="*")
def redact(ctx, call, result):
    return ctx.replace_result(scrub_secrets(result))

@hook(on="pre_ask")                   # context injection (dataflow system prompt)
def inject_context(ctx):
    ctx.prepend_system(ctx.read_agents_md() + "\n" + ctx.recall_window(n=4))

@hook(on="post_turn")
def remember(ctx, reply):
    ctx.umem("session summary", ctx.summarize(reply))

# ---- the agent: declares the LOOP + the turn body, all in-program ------------
agent = ConversationalAgent(
    persona="You are APXM Assistant. Be precise and concise.",
    memory_space="stm",
    tools=[lookup],
    tool_groups=["web"],
    skills=True,                       # real search_skills discovery
    sub_agents=[researcher],           # resolved in the SAME artifact
    compaction=CompactionPolicy(keep_recent=4, compact_at_tokens=20_000),
    hooks=[announce, guard_lookup, redact, inject_context, remember],
    loop="in_graph",                   # the conversation loop lives in the .air
)

main = agent.compile()                 # → ONE self-contained multi-flow artifact
```

Properties asserted: one file/one artifact; the loop is in the graph
(`loop="in_graph"`); hooks are python callables covering the full lifecycle and
can **control flow** (deny/edit/inject), not just observe; sub-agents resolve
from the same artifact; it runs identically under the CLI host and the server,
with the host reduced to "POST once, pipe stdin to the turn-input seam, render
tokens."

---

## 3. The architecture

Three mechanisms compose into "one program."

### 3.1 The program shape — a multi-flow artifact

`ConversationalAgent.compile()` emits **one module with multiple `func.func`**:

- `main` — the **loop flow**: a re-arming turn-input wait whose body is the
  author's **turn sub-DAG** (not Rust `run_agent_turn`).
- the **turn body** — author nodes: `recv(user_msg)` → pre_ask hooks → recall
  window → `ask(tools, skills)` with pre/post_tool hooks → optional
  `delegate(researcher)` → `umem` remember → compaction subgraph → `done`.
- one flow per sub-agent (`researcher.main` / `researcher.delegate`).

The runtime/artifact layer already supports this (§1E); the work is a frontend
multi-func emitter + a name carrier that preserves the `Agent.flow` dot.

### 3.2 The loop — two realization paths

The scheduler is a fire-once topological DAG (cycles rejected, `dag.rs:204`).
Putting the loop in the graph has two paths; the spec adopts **Path B as the
target** and allows **Path A as an intermediate**.

- **Path A (intermediate, medium):** the loop stays an in-handler Rust `while`
  (extend `autonomous`/`recv`), but per turn it **dispatches the author's
  turn-flow** (a sibling `func.func`) instead of the hardcoded `run_agent_turn`,
  binding the user message as input and threading session state. Reuses
  FLOW_CALL/child-DAG dispatch (already works). Gets "author owns the turn body"
  without scheduler surgery. Cost: the loop *construct* is still Rust-parameterized.

- **Path B (target, large — the authorized scheduler work):** a **native
  re-arming loop**. `recv` is converted from poll to **real park** (return
  `OperationParked{wait_key=session_recv_key}`, reusing the PAUSE path); a server
  turn-input seam calls `park_registry::wake(session_recv_key, user_msg)`; on
  wake the runtime **re-splices a fresh turn-body sub-DAG** (`splice_dag`) and
  re-arms the wait; one long-lived execution per session holds one SSE open
  across parks. This is what makes the host truly dumb. Verified-hard blockers:
  no node re-execution / one-execution-per-turn at the server / re-arm after wake.

### 3.3 Hooks — python-handler-backed lifecycle bindings

A hook is a `python_handler_id` bound to a lifecycle event, reusing the `@tool`
plumbing that already runs server-side.

```
@hook(on=EVENT, match=GLOB, mode=observe|gate)
  └─ frontend: handler_id (same registry as @tool) + hooks.json sidecar
  └─ lowering: REGISTER_HOOK op { HOOK_EVENT, HOOK_MATCH, HOOK_MODE, PYTHON_HANDLER_ID }
       • AIR-portable: travels inside the artifact, like REGISTER_CAPABILITY
  └─ runtime: per-artifact HookRegistry on ExecutionContext (lifetime = python_tool_bridge)
       • pre_tool/post_tool → PythonHookInterceptor, wired into BOTH bridge sites
         (tool_dispatch.rs:281 AND inv_tool.rs:262 — both bypass interceptors today)
       • pre_ask/post_ask    → PythonHookMiddleware (OperationMiddleware applies_to=Ask)
       • workflow_start/graph_* → awaited async pre-step in execute_artifact_inner
         (NOT the sync, dead ExecutionHook trait — sync cannot await the bridge)
       • invocation          → PythonToolBridge.call_hook(handler_id, payload)
```

Consequences: the three disconnected surfaces (§1F) collapse into one;
`AgentHooks` (decorative) is deleted; subprocess `command=` hooks become one
backing kind of the same `@hook`; hooks fire on `/v1/execute/stream` for free
because the bridge and emitter already run there.

### 3.4 Context, skills, sub-agents — in-program

- **Context:** `system_prompt` becomes dataflow-driven (small: `resolve_system_prompt`
  accepts an input operand); a `qmem(recent=N)` recency window makes the
  transcript session memory; compaction is an in-graph subgraph
  (`count_tokens` builtin → `guard`/`switch` on the integer → `summarize` ASK).
  Note: `branch()` labels are not routed by the scheduler — use `guard`/`switch`.
- **Skills:** `skills=True` enables the real `search_skills` group; add a
  first-class `skill_search()` helper. Delete the stub `find_skill`; correct the
  "embedder-backed" docstring (it is lexical).
- **Sub-agents:** port HANDOFF's inline fallback into DELEGATE (`delegate.rs`),
  so `delegate(researcher)` resolves via `agent_info` STM written by SPAWN_AGENT
  even before the multi-func emitter lands; carry the spawn grant inside the
  artifact so it is self-contained.

---

## 4. Capability contracts

Each states requirement, mechanism, the **exact** compiler + runtime work
(verified, with effort), and acceptance. All must pass on the server path.

### 4.1 The loop in the program
- **Requirement:** the conversation loop runs inside the artifact; the host does
  not loop.
- **Compiler:** lower a turn-loop entry whose body is the author turn sub-DAG;
  AIR validation for a recv/turn-input entry + loop-back marker (medium).
- **Runtime (Path B):** `recv` → real `OperationParked` (medium); re-arm via
  `splice_dag` on wake (large, keystone); one long-lived execution per session
  over one SSE (large); per-session budget/grant ledger (large).
- **Acceptance:** a 10-turn session runs under one execution, host only pipes
  stdin + renders tokens; no worker pinned during idle waits.

### 4.2 User input into the graph (I/O seam)
- **Requirement:** each user message enters a parked in-graph node; tokens stream
  out from any depth.
- **Already works:** streaming out at any depth (`mod.rs:347` → `EmitterAdapter`
  → SSE); entry-arg injection as pre-ready tokens (`state.rs:163-208`).
- **Runtime:** server turn-input endpoint that `park_registry::wake`s the parked
  RECV with the user text as a `Value` (clone of the checkpoint-resume route,
  `checkpoints.rs:255`); accept zero-arg looping entry in `validate_args` (small).
- **Acceptance:** `recv → ask → recv → ask` drives two real turns over one
  connection.

### 4.3 Context management (in-program)
- **Compiler:** dataflow `system_prompt` operand on `ask()` (small); `qmem(recent=N)`
  surface (small); expose `switch_` runtime-discriminant regions to Python (large)
  *or* document `guard`-based compaction.
- **Runtime:** `resolve_system_prompt` accepts an input operand (small);
  `qmem` recent-window recall mode (medium); `count_tokens` registration parity
  in `register_standard_tools` (small).
- **Acceptance:** a 20-turn server-only session compacts + recalls in-graph;
  changing `keep_recent` in Python changes behavior.

### 4.4 Hooks (the keystone)
- **Compiler:** `REGISTER_HOOK` op (new op, update the op-count/tablegen parity
  guard) + graph keys `HOOK_EVENT/HOOK_MATCH/HOOK_MODE/PYTHON_HOOK_HANDLER_ID`;
  `@hook` decorator + `g.register_hook()` + hooks.json sidecar; AIR validation
  (all small–medium).
- **Runtime:** per-artifact `HookRegistry` on `ExecutionContext` (medium);
  `PythonHookInterceptor` wired into **both** bridge sites (medium);
  `PythonHookMiddleware` for pre/post_ask (medium); gate-capable `workflow_start`
  as an awaited async pre-step (small — H3 refuted the "blocker").
- **Acceptance:** the §2 fixture's `guard_lookup` deny, `redact` rewrite, and
  `inject_context` all observable under `/v1/execute/stream`.

### 4.5 Sub-agents in one artifact
- **Compiler:** multi-func `to_air` emitter (medium); dotted-name carrier so
  `Agent.flow` survives `_sanitize_name` (small); build-time `delegate()` target
  validation (small).
- **Runtime:** DELEGATE inline fallback mirroring HANDOFF (small); self-contained
  spawn-grant admission (medium).
- **Acceptance:** single-file `delegate(researcher)` runs end-to-end; the
  rewritten `chat_agent.py` works.

---

## 5. Cross-cutting invariants ("fully connected")

1. **Transport parity** — runs identically on CLI host and `/v1/execute/stream`;
   nothing depends on `requires_local_cli`.
2. **AIR-portability** — loop, hooks, compaction policy, sub-agents travel inside
   the one artifact; a deployed `.apxmobj` carries its own control logic.
3. **One handler mechanism** — tools and hooks share `python_handler_id` +
   `PythonToolBridge` + the sidecar. No second Python-invocation path.
4. **Control, not just observation** — pre-hooks return Allow/Deny/EditArgs and
   may mutate the system prompt; gate hooks can short-circuit.
5. **Named entry** — the turn/user-message binds by reserved name, not position.
6. **No dead surface** — delete `AgentHooks`; docstrings match behavior; no
   silent worker-pinning poll loops.

---

## 6. Phased delivery

Each phase ships green with an acceptance test. P0–P3 move the entire **turn**
(cognition, context, hooks, sub-agents) into the program — medium effort, **no
scheduler surgery**. P4 moves the **loop** itself in — the large, authorized
scheduler work — making the host a dumb pipe.

- **P0 — In-program turn, no new ops (small):** DELEGATE inline fallback (proves
  in-program sub-agents today); dataflow `system_prompt` (proves in-program
  context injection); `count_tokens` registration parity; fix the stub/broken
  examples; auto-admit spawn for compiled graphs; delete `AgentHooks`.
- **P1 — One artifact + `ConversationalAgent` (medium):** multi-func emitter +
  dotted-name fix; the builder emitting the loop flow + turn body + sub-agent
  flows; named turn param; `skill_search` helper.
- **P2 — Hook keystone (medium):** `REGISTER_HOOK` + `@hook` + `HookRegistry` +
  interceptor/middleware/async-pre-step drivers wired into both bridge sites.
  *Gate: §2 fixture hooks fire on the server path.*
- **P3 — Context fully in-program (medium):** `qmem(recent=N)` + in-graph
  compaction subgraph (guard/switch); host stops owning the transcript.
- **P4 — The loop in the program (large, capstone):** `recv` real-park + re-arm
  via splice + one long-lived per-session execution + turn-input wake seam +
  per-session budget/grant ledger. *Gate: host is POST-once + pipe + render.*

Until P4, `loop="host"` (thin host owns the outer loop) is the default and
everything else is in-program; `loop="in_graph"` activates after P4. The §2
fixture is the whole-spec acceptance test.

---

## 7. Open decisions (call before P2 / P4)

1. **Path A vs Path B for the loop.** Ship Path A (in-handler dispatch of the
   author turn-flow) as an early win, or go straight to Path B (native re-arm)?
   Recommendation: **P1–P3 deliver the turn under `loop="host"`; do Path B
   directly at P4** (Path A's parameterized-Rust-loop is throwaway).
2. **`REGISTER_HOOK` op vs entry-func attr block.** Recommendation: **new op**
   (consistency with REGISTER_CAPABILITY, inspectable).
3. **In-graph branching surface.** Expose `switch_` regions to Python (large) or
   ship `guard`-based compaction now (small)? Recommendation: **guard now,
   switch later.**
4. **Discovery embedder.** Correct docs to "lexical" now; embedder as a separate
   enhancement.
5. **Per-session budget/grant ledger location** (P4): runtime keyed by
   `session_id` vs a session service. Recommendation: **runtime ledger** (keeps
   the host dumb).

---

## 8. Traceability — verified blockers → contract

| Blocker (verified, file:line) | Sev | Contract / Phase |
|---|---|---|
| Scheduler fire-once, no node re-execution (`dag.rs:204`) | hard | 4.1 / P4 |
| `recv` polls, never parks (`autonomous.rs:446`) | hard | 4.1 / P4 |
| One execution = one turn at server | hard | 4.1 / P4 |
| Per-turn cognition host-resident (`chat.rs:99-168`) | hard | 4.1+4.3 / P3+P4 |
| `system_prompt` static attr only (`llm/mod.rs:114`) | hard | 4.3 / P0 |
| `switch_` unreachable from Python @compile | hard | 4.3 / P3 |
| pre/post_tool hooks bypass both bridge sites (`tool_dispatch.rs:281`,`inv_tool.rs:262`) | hard | 4.4 / P2 |
| `ExecutionHook` dead, host-global, not program-scoped | soft | 4.4 / P2 |
| Frontend emits one `func.func` (`ir.py:247`) | hard | 4.5 / P1 |
| DELEGATE has no inline fallback (`delegate.rs:48`) | hard | 4.5 / P0 |
| `_sanitize_name` mangles `Agent.flow` (`ir.py:258`) | soft | 4.5 / P1 |
| Spawn grant external, not self-contained | soft | 4.5 / P0 |
| `branch()` labels never routed by scheduler | soft | 4.3 (use guard/switch) |
| `count_tokens` not in `register_standard_tools` | soft | 4.3 / P0 |

Verification corrected two over-stated claims: the multi-func limit is confined
to the **frontend** text emitter (runtime/artifact are already multi-DAG), and a
gate-capable `workflow_start` is a **small** async-pre-step addition (the sync
`ExecutionHook` is not the right vehicle), not scheduler surgery.
```
