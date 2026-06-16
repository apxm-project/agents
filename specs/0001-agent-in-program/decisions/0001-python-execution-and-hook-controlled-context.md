# ADR 0001 — Production execution model for python handlers + hook-controlled context

Status: accepted (investigation-backed) · 2026-06-15 · feature 0001-agent-in-program
Basis: 5-dimension ultracode investigation, every claim adversarially verified
against code (file:line). Supersedes the AskUserQuestion option "allow python on
the server".

## Context

A conversational agent's python `@tool`/`@hook` handlers run via a
`PythonToolBridge` that spawns `python -m apxm.tool_worker`. Three execution
paths exist with different trust postures:

- **raw `/v1/execute` (server):** REJECTS python — two guards (python_tools
  section at execute.rs:801; any `PYTHON_HANDLER_ID` node at execute.rs:817) AND
  `workflow_source.rs:53` strips the sidecar. Structurally python-free.
- **CLI/driver in-process:** allows python (`run_graph_with_python_tools_sidecar`,
  apxm-cli execute.rs:323).
- **trusted artifact:** `python_tool_bridge_from_artifact` (runtime.rs:1124) —
  exists but has **zero callers** (dead/unwired).

Verified production posture: **single-tenant, single-principal, loopback-only.**
`DEFAULT_ADDR=127.0.0.1` (main.rs:92; `bind_addr` overridable with no loopback
enforcement, startup.rs); auth opt-in **default-off**, single shared host bearer
when on (auth.rs); `owner` is a self-declared request string scoping credential
lookup only, not an authenticated principal (credentials.rs:43-44).

Verified isolation gap: the python worker spawns with **no OS isolation and full
inherited environment** (worker.rs:122 — no `env_clear`). The bubblewrap sandbox
backend exists and wraps bash/capabilities/ACP (`wrap_command`, backend.rs:117,
sandbox_linux.rs:200) but is **never applied to the python worker**. A handler
can read the host's KEK / LLM keys / auth bearer. `supports_resource_limits =
false` (no cpu/mem caps from bwrap alone).

## Decision

### Where python-handler agents run

Two independent layers — **provenance** (whose code may run) + **isolation**
(what it can do if it misbehaves):

1. **Keep raw `/v1/execute` python-free.** It is the untrusted-AIR entry point;
   its rejection + security suite stay.
2. **Run author python only via a trusted, provenance-verified compiled-artifact
   path** (signed `.apxmobj` installed through a deploy step — the skills model;
   how apxm-os already deploys). Wire the dead `python_tool_bridge_from_artifact`
   path to the skill/deploy surface; relax the guards ONLY under that path.
3. **Isolate the worker.** Step 1: `env_clear()` + `SAFE_PASSTHROUGH` allowlist
   (DONE in this feature). Step 2: route the worker spawn through bubblewrap
   `wrap_command` (default-deny net, per-exec cwd) + a timeout/cgroup for cpu/mem.
4. **CLI/driver remains the safe home** for python agents in the current
   single-user-local reality.
5. **Multi-tenant only later:** real per-principal auth (bind bearer→owner) + a
   separate per-tenant executor service. Inversion trigger: `bind_addr` leaves
   loopback OR the host is shared.

### Hook-controlled context / compaction / budgets

Mandated by the constitution (#2 program owns context policy, #3 compaction
travels in the artifact, #5 hooks inject context, #7 no dead surface):

- **Hooks own policy; the trusted runtime owns budget enforcement**
  (`charge_tokens`, `SessionLedger` — already fail-closed).
- **Drop the hardcoded compaction subgraph (T051) — it was never built**
  (`CompactionPolicy` is metadata-only; `_build_turn_flow` is recall→ask→done).
- **Real blocker:** `_HookCtx.recall_window/umem/summarize` are dead no-ops
  (tool_worker.py:270-279) because the bridge is **unidirectional** (no
  worker→runtime callback); `post_turn` gets only `{reply}` and its return is
  discarded (hook_driver.rs:243-250) — so a hook **cannot compact today**.
- **Fix:** add a worker→runtime callback channel (make the memory/budget/
  count_tokens helpers real), enrich `pre_turn`/`post_turn`/`pre_ask` payloads
  (remaining_budget, token count, transcript window), add a **transcript-rewrite
  outcome** to `post_turn`/`pre_ask`, ship an author-overridable default
  compaction hook. `count_tokens` builtin already exists.

## Consequences

- Raw server stays safe; python is a trusted-deploy capability, not a client RPC.
- The "host is a dumb pipe" goal is preserved: cognition (incl. compaction) lives
  in the program via hooks, enforcement in the runtime.
- New work, sequenced: (1) worker env hardening ✅; (2) bubblewrap-wrap the
  worker; (3) wire trusted-artifact python path + guarded relax; (4) hook
  callback channel + transcript-rewrite + default compaction hook.

## Changes shipped in this feature (verified)

1. **Worker env hardening** — `worker.rs` now `env_clear()`s the python worker
   and passes only the sandbox `SAFE_PASSTHROUGH` allowlist (+ PYTHONPATH/
   PYTHONUNBUFFERED + trusted caller env), closing the host-secret-leak gap.
2. **Sidecar strip** — server (`workflow_source.rs`) + CLI (`compile.rs`) strip
   any `; __apxm_*` sidecar comment so `__apxm_hooks__` no longer breaks MLIR.
3. **Flow-call message wiring + recv re-arm attrs** — fixed the in-graph loop so
   the user message reaches the turn flow and the loop re-arms; PROVEN live
   multi-turn over the AMD backend ("2+2"→"Four", "10×10"→"100").
4. **Hook-controlled context/compaction (the unidirectional design, NOT a
   callback channel)** — `post_turn` and `pre_ask` hook payloads are enriched
   with `remaining_budget` + a recent conversation `window` pre-loaded from
   session STM; `_HookCtx.recall_window` reads it; `ctx.umem(...)` accumulates
   writes the runtime applies to STM after the hook returns
   (`apply_hook_writes`). This makes the previously-dead `recall_window`/`umem`
   helpers real and lets a post_turn hook compact/remember — author-controlled,
   no bidirectional bridge needed.
5. **Python worker portability fix (CRITICAL)** — `worker.rs` `PYTHON_BIN` was
   hardcoded to `"python"`; modern hosts ship only `python3`, so the python
   tool/hook worker could NEVER spawn (ENOENT). Now `resolve_python_bin()`
   prefers `python3`. Without this, NO python @tool/@hook ran on such hosts.
6. **Turn-hook transport parity** — `ConversationMemoryMiddleware` (which fires
   `pre_turn`/`post_turn`/`post_ask`) was registered only on the server path;
   now also registered on the driver/CLI path (constitution #1).
7. **`register_hook(fn=...)` sidecar wiring** — registers the hook handler into
   the python-tools sidecar so hooks resolve in raw `@compile` graphs, not only
   `ConversationalAgent`.

**LIVE-PROVEN (T047):** `session_start` and `post_turn` hooks fire end-to-end
over the real AMD backend via `apxm execute` (markers written from inside the
hook handlers). pre_tool/post_tool/pre_ask drivers are wired into both bridge
sites + the ask middleware. Full suite green after all changes: runtime 72,
server 19, 0 failed.

8. **Bubblewrap worker isolation (#2) — DONE + LIVE-VERIFIED.** Installed
   bubblewrap 0.9.0 + the `/etc/apparmor.d/bwrap` userns profile (Ubuntu 24.04).
   The python tool/hook worker spawn now routes through the sandbox backend's
   `wrap_command` (RO root, ephemeral `/tmp`, `--unshare-net`, manifest written
   into a temp dir bound writable as the worker cwd). Threaded via
   `PythonToolBridge::with_sandbox` + `Runtime::python_worker_sandbox`, gated on
   the `APXM_SANDBOX_PYTHON` opt-in (default off = unchanged behavior). VERIFIED:
   with the flag, `apxm execute` logs `Sandboxing python tool worker
   backend=apxm-bwrap` and the agent runs end-to-end over AMD (reply "SANDBOX-OK",
   hooks registered) inside the sandbox; without it, the default path is
   untouched. (Note: bwrap gives fs+net isolation, not cpu/mem — `supports_resource_limits=false`;
   a cgroup/timeout is the follow-up for untrusted multi-tenant.)

9. **#3-lite trusted+sandboxed server-python path — DONE + LIVE-VERIFIED.** An
   operator trust gate `python_artifacts_trusted()` (requires BOTH
   `APXM_TRUST_PYTHON_ARTIFACTS` and `APXM_SANDBOX_PYTHON` — never unsandboxed
   author python on the server) now: captures the python-tools sidecar in
   `prepare_request`, re-attaches it to the compiled artifact as a `python_tools`
   section (`attach_trusted_python_section`), and relaxes the server admission
   guards under that gate — python sections, python-handler nodes, in-artifact
   INV_TOOL capabilities, and ASK tool-exposure are admitted for capabilities the
   artifact provides itself (read from the section manifest). VERIFIED LIVE: the
   FULL `controllable_agent` fixture (in-graph loop + python @tool + @hooks +
   sub-agent + skills) runs end-to-end on the server via `/v1/execute/stream` +
   `/v1/conversations/{id}/message` — recv parks, the turn-input endpoint wakes
   it (`woken=1`), the reply streams ("2+2 equals 4."), the python worker runs
   `Sandboxing python tool worker backend=apxm-bwrap`, and session_start/
   post_tool/post_turn hooks register. This is T047-full + T032 (CLI ✓ + server ✓).

## Pre-commit review findings (ultracode, adversarially verified)

A 5-dimension review of the committed diff found hygiene CLEAN (no secrets, no
identifiers, no cruft) and these issues:

- **FIXED — fail-open sandbox (was high, security):** `python_worker_sandbox()`
  swallowed `select(OsLevel) Err → None`, so on a trusted host without bwrap the
  worker spawned UNSANDBOXED while the trust gate claimed confinement. Now
  **fail-closed**: `sandbox_required` (set by `APXM_SANDBOX_PYTHON`) is threaded
  to `spawn_with_env`, which returns an error rather than running author python
  unsandboxed when no OS-isolating backend is available.
- **BOUNDED (CONV-1) — re-arm memory growth (was high):** each in-graph turn
  spliced a FLOW_CALL + recv node with no upper bound. Now the re-arm is capped
  by the recv node's `MAX_ITERATIONS` (default 100): `RearmSpec` carries
  `session_id` + `max_turns`, `SchedulerState.next_rearm_turn(session_id)`
  counts per-session re-arms, and `ParkWaker::fire()` stops re-arming once the
  cap is reached. Residual follow-up: completed nodes/tokens are still not
  reclaimed (`condense_subdag` has no caller on the wake path), so growth within
  the cap is monotonic — full GC of completed re-arm nodes remains future work.
  Committed `7307384c`.
- **RESOLVED (CONV-2) — turn hooks fired per Ask:** `ConversationMemoryMiddleware`
  applied to every Ask, so sub-agent asks (sharing the session `memory_scope` via
  inherited `session_id`) inflated `conversation:turn_count`, polluted the recall
  window, and re-fired pre_turn/post_turn/post_ask hooks. Now gated on a
  `conversational_turn` marker the frontend stamps on the top-level turn ask
  (`ConversationalAgent._build_turn_flow`); the program declares the turn
  (constitution #2). Committed `b3c69558`.
- **RESOLVED (CONV-4) — `__system` positional binding is safe:** the concern was
  that `dataflow_system_prompt` resolves `__system` by its position in
  `input_names` and indexes `inputs` by it, which would misbind if control edges
  interleaved value operands. Verified this cannot happen: control/effect edges
  are scheduling-only and contribute NO value token to a node's operand list
  (a compiled ask with a control edge still emits exactly one operand +
  `input_names = ["history"]`). `input_names` is therefore strictly parallel to
  `inputs` — a parity `render_named` enforces on the same node
  (`input_names.len() == inputs.len()`, else error), and the scheduler's
  `collect_inputs` builds a full-width operand vector (every input token ready or
  the node does not run). So position-in-the-parallel-array IS name binding; no
  code change. (The legacy `engine.rs` path skips missing operands and is not the
  production scheduler path.)
- **MAINTAINER SIGN-OFF — inference concurrency 2→16 (medium, WH-1):** the
  concurrent webhook change raised the default 8× deployment-wide; confirm vs
  rate-limited backends (overridable via `[server.inference]`).

## Remaining (sequenced, per this ADR)

- **#3 FULL multi-tenant provenance:** cryptographic artifact signing (apxm-auth
  sign + server verify) to replace the operator-trust env gate for untrusted
  multi-tenant callers. The single-tenant operator-trust path (#3-lite) is done.
- **TIMEOUT ENFORCED (was observe-only):** the per-call deadline is now a real
  resource limit. The worker wrapped the handler in `asyncio.shield`, so a
  timed-out (or externally cancelled) coroutine kept running to completion in
  the background — the deadline reported but did not reclaim compute. Removed
  the shield (`wait_for(coro, …)`) so async handlers are actually cancelled, and
  the Rust bridge now sends a best-effort `Cancel` on timeout instead of merely
  abandoning the response. Covered by `tests/test_tool_worker_deadline.py`
  (red/green-verified against the shield).
- **REMAINING — cpu/mem hard limits:** bwrap gives fs+net isolation, not
  cpu/mem, and a runaway *sync* handler runs in an executor thread that Python
  cannot cancel cooperatively. True per-call cpu/mem reclamation needs
  process-level limits (a process-per-call worker or cgroup-v2 delegation), a
  design change rather than a flag — not bolted on as a process-wide
  `setrlimit`, which would destabilize the persistent multi-handler worker.
  After that lands, promote `APXM_SANDBOX_PYTHON` toward a policy default.
- T053/SC scale validations (50-turn compaction; skill-by-description 8/10).

## Hook-driven LLM compaction (added 2026-06-15, proven live)

Hooks were previously LLM-blind (worker subprocess, one-way bridge), so
`ctx.summarize` could only truncate. Added a bidirectional host-call channel:
a hook raises an `llm.ask` host call that the runtime services with the hook's
OWN `ExecutionContext` (same backend, budget, cancellation), replying with a
`host_result`. To keep the context borrow-clean the worker's `pending` map
became an mpsc so an awaiting call services interleaved host calls inline via a
handler closure built from `ctx`; the `llm.ask` handler runs an ephemeral ASK
through the normal LLM path (boxed to break the `pre_ask → llm → pre_ask`
recursion). `ctx.ask`/`ctx.summarize` (Python) emit the host call from the hook
thread and block on a queue until the async loop delivers the result; stdout is
unified under one thread-safe lock. Compaction is then a `post_turn` hook that
folds (prior summary + full transcript window) → a rolling `conversation:summary`
that `recent_scoped` surfaces ahead of the recent window.

**Proven live over AMD:** a user-stated turn-1 fact is summarized by the real
backend, rolled forward, and recalled at turn 6 after leaving `keep_recent=4`
(SC-003), with the worker sandboxed under bwrap. The optional in-graph
summarize/fold subgraph (host-independent, no python) is future work, not a
blocker.
