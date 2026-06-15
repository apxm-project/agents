# Implementation Plan: The Whole Conversational Agent in One Program

**Phase:** plan (next: tasks) · **Spec:** `./spec.md` · **Evidence base:**
`./research.md` (verified, file:line) · **Clarifications:** finish-it-all, full
scope incl. in-program loop (spec.md §Clarifications 2026-06-15).

## Summary

Make the entire conversational agent authorable as one self-contained APXM
program — the conversation loop, each turn, context management + compaction,
pre/post/session-start hooks, skill/tool discovery, and sub-agents — so the host
(`apxm chat` or the HTTP server) is a dumb pipe that only delivers user input and
renders streamed output. The chosen approach reuses the runtime substrate that
already ships (no-poll park/wake, live-DAG `splice_dag`, `PythonToolBridge`,
multi-DAG artifact format) and adds: a native re-arming in-graph loop, a
program-authored lifecycle-hook mechanism, dataflow-driven context controls, and
a frontend `ConversationalAgent` that emits one multi-flow artifact. The loop —
the only large item — lands as a native re-arm (park + splice) rather than a
throwaway intermediate, per the finish-it-all clarification.

## Technical Context

- **Language/Version:** Rust (workspace edition as repo) for compiler/runtime/
  server; Python 3.12 for the frontend authoring package.
- **Primary Dependencies:** tokio (scheduler/workers), the APXM executor +
  scheduler (`crates/runtime/apxm-runtime`), MLIR via `apxm-compiler`
  (ArtifactEmitter, wire v3), `reqwest` (server I/O), serde; the subprocess
  Python tool bridge (`tool_worker.py` + `python_tools/`).
- **Storage:** existing STM memory store (session-scoped via `memory_scope()`);
  SQLite for durable park/checkpoint state; no new store.
- **Testing:** `cargo test` per crate (runtime/server/compiler suites must stay
  green — they encode park/wake, op-invariants, server security); Python
  `--validate` AIR check; an end-to-end fixture run on both hosts.
- **Target Platform:** Linux; two reference hosts — interactive CLI
  (`apxm chat`) and HTTP server (`/v1/execute/stream`).
- **Project Type:** multi-crate Rust workspace + Python frontend package +
  examples.
- **Performance Goals:** per-turn work independent of completed-turn count
  (SC-007); idle waits MUST NOT pin a compute/LLM permit (route to the blocking
  pool, park rather than poll).
- **Constraints:** every capability MUST run on `/v1/execute/stream` (not
  CLI-only); all control logic MUST travel inside one artifact (AIR-portable);
  reuse `PythonToolBridge` for both tools and hooks; respect the op-count /
  tablegen parity guard when adding an op; keep the existing host-driven agent
  working during development.
- **Scale/Scope:** one feature spanning frontend (Python), compiler (MLIR
  emitter + AIR validation), runtime (scheduler, executor, handlers, hooks), and
  server (turn-input seam, admission). ~5 subsystems.

No open `NEEDS CLARIFICATION`: the two deferred clarify items are resolved here
with safe defaults — lifecycle-rule failure is **surface-and-fail-closed** for
`gate` hooks (a failed gate denies and reports) and **surface-and-continue** for
`observe` hooks; sub-agent default execution is **in-process** (inline/flow
dispatch), with ACP-profile sub-agents opt-in.

## Constitution Check

Gated against `.spec/memory/constitution.md` v1.0.0 (principles cited by number).

| Constitution principle | This plan |
|---|---|
| 1. Transport parity | PASS — hooks/loop/context all run via runtime + bridge that already serve the server path |
| 2. Program owns cognition, host is a pipe | PASS — CLI shrinks to POST-once + pipe + render (T030) |
| 3. AIR-portability | PASS — multi-flow emitter + `REGISTER_HOOK` op + policy attrs travel in one artifact |
| 4. One Python-handler mechanism | PASS — `HookRegistry` reuses `PythonToolBridge` |
| 5. Control not just observation | PASS — `CapabilityInterceptor` Allow/Deny/EditArgs + `OperationMiddleware` |
| 6. Named entry | PASS — `ConversationalAgent` declares the reserved turn param |
| 7. No dead surface | PASS — delete `AgentHooks`; docstrings corrected |
| 8. Reuse proven substrate | PASS — park/wake, `splice_dag`, bridge, multi-DAG reused |
| 9. Waits MUST NOT pin compute | PASS — `recv` converted poll→park (T021) |
| 10. Green invariant suites | PASS — op-parity guard updated with `REGISTER_HOOK` (T070); suites kept green (T072) |

**Justified complexity (Complexity Tracking):** the native re-arming loop
requires scheduler work (re-arm a parked node by splicing a fresh turn sub-DAG).
Justified because the finish-it-all clarification puts the in-program loop in
scope and the intermediate (in-handler Rust loop) is throwaway; simpler
alternative (keep the loop in the host) rejected because it violates FR-002.

## Project Structure

Concrete files, grouped by subsystem (paths verified in `research.md`).

```
crates/compiler/apxm-frontend/python/apxm/
  agent.py            # delete AgentHooks; ConversationalAgent builder
  hooks.py            # NEW: @hook decorator, HookFn, register_python_hook
  proxy.py            # g.register_hook(); dataflow system_prompt operand on ask();
                      #   qmem(recent=N); skill_search() helper; multi-flow capture
  ir.py               # multi-func to_air emitter; preserve Agent.flow dot
  decorators.py       # multi-graph capture for ConversationalAgent
  constants.py / _generated/  # HOOK_EVENT/HOOK_MATCH/HOOK_MODE/OP_REGISTER_HOOK keys

crates/compiler/apxm-compiler/
  mlir/.../Artifact/ArtifactEmitter.cpp   # already multi-DAG (verify dot carrier)
  src/.../ais ops + validation            # REGISTER_HOOK op + AIR validation

crates/core/apxm-ais/
  src/chat.rs         # the canonical in-program turn graph (replaces chat_air spine)

crates/runtime/apxm-runtime/src/
  executor/handlers/autonomous.rs   # recv: poll→park; turn body = author flow, not run_agent_turn
  executor/handlers/delegate.rs     # inline fallback mirroring HANDOFF
  executor/handlers/llm/tool_dispatch.rs  # PythonHookInterceptor wired here (bridge site 1)
  executor/handlers/inv_tool.rs           # PythonHookInterceptor wired here (bridge site 2)
  executor/handlers/llm/mod.rs            # resolve_system_prompt accepts dataflow operand
  executor/hooks/ (NEW)             # HookRegistry, PythonHookInterceptor/Middleware, async pre-step
  executor/context.rs              # thread HookRegistry; per-session budget/grant ledger
  scheduler/state.rs, splicing.rs  # re-arm a parked node via splice (the loop keystone)
  capability/builtins/count_tokens.rs  # register in register_standard_tools (parity)
  memory qmem.rs                   # recent-window recall mode

crates/tools/apxm-server/src/
  execute.rs          # self-contained spawn-grant admission; one long-lived session execution
  routes.rs           # NEW turn-input endpoint that park_registry::wake(session_key, msg)

crates/tools/apxm-cli/src/commands/chat.rs   # shrink to dumb pipe (POST-once + pipe + render)

examples/python/conversational/
  controllable_agent.py   # NEW: the spec acceptance fixture (spec.md §2 equivalent)
  conversational_agent.py, chat_agent.py  # fix stub/broken examples
```

## Companion docs

- `./research.md` — the verified architecture + every blocker with file:line and
  effort (the evidence base; already present).
- `./data-model.md` — entities and state transitions (turn, hook, session memory,
  park wait-key, multi-flow artifact).
- `./contracts/` — external interfaces this feature exposes: the Python authoring
  API, the new AIR ops/attrs, and the server turn-input endpoint.
- `./quickstart.md` — how to build, run on both hosts, and validate the fixture.

## Boundary

Planning ends here. The ordered task list is `spec-tasks` → `tasks.md`.
