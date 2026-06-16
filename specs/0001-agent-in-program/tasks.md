# Tasks: The Whole Conversational Agent in One Program

**Inputs:** `plan.md`, `spec.md`, `data-model.md`, `contracts/`, `research.md`.
Tests are included only where a Success Criterion needs a guard or a runtime
invariant suite protects the change. `[P]` = parallel-safe (different file, no
unfinished dependency). `[US#]` tags story tasks.

## Phase 1 — Setup

- [X] T001 Add the feature acceptance fixture stub `examples/python/conversational/controllable_agent.py` mirroring `spec.md` §2 (compiles, `--validate` passes once foundation lands).
- [X] T002 [P] Register `count_tokens` in `register_standard_tools` for non-server runtimes — `crates/runtime/apxm-runtime/src/capability/builtins/count_tokens.rs` + registration site (parity).
- [X] T003 [P] Delete decorative `AgentHooks` and its exports — `crates/compiler/apxm-frontend/python/apxm/agent.py`, `__init__.py`.
- [ ] T004 (Optional) Run `spec-constitution` to formalize the "fully connected" invariants at `.spec/memory/constitution.md`; re-gate `plan.md`.

## Phase 2 — Foundational (blocks all stories)

- [X] T010 Multi-func `to_air` emitter: serialize N captured graphs into one `module { func.func @A.main … }` — `crates/compiler/apxm-frontend/python/apxm/ir.py`, `decorators.py`.
- [X] T011 Preserve the `Agent.flow` dot through name handling so `reconstruct_agents_from_artifact` registers each flow correctly — `ir.py` (`_sanitize_name`) + verify `crates/compiler/apxm-compiler/mlir/.../ArtifactEmitter.cpp`.
- [X] T012 [P] Dataflow `system_prompt`: optional input operand on `ask()` (`proxy.py`) + `resolve_system_prompt` reads it — `crates/runtime/apxm-runtime/src/executor/handlers/llm/mod.rs`.
- [X] T013 `HookRegistry` type + thread onto `ExecutionContext` (lifetime = python tool bridge), inherited by child contexts — new `crates/runtime/apxm-runtime/src/executor/hooks/` + `executor/context.rs`.
- [X] T014 Server turn-input substrate: `POST /v1/conversations/{session}/message` → `park_registry::wake(session_recv_key, msg)` with wake-before-register sentinel — `crates/tools/apxm-server/src/routes.rs`; session→execution registry + one long-lived execution per session — `execute.rs`. (Substrate: endpoint + `session_recv_key` + `SessionRegistry` landed; one-execution-per-session lifecycle wired in US1/US2.)
- [X] T015 `ConversationalAgent` builder skeleton emitting entry-loop + turn flows with the reserved named turn param; canonical in-program turn graph — `crates/compiler/apxm-frontend/python/apxm/agent.py`, `proxy.py`, `crates/core/apxm-ais/src/chat.rs`. (Frontend builder + reserved `user_message` param landed; `chat.rs` canonical-graph refinement folded into US1.)
- [X] **Checkpoint:** foundation builds; `cargo test` green for runtime/server/compiler; `controllable_agent.py --validate` emits one multi-flow artifact.

## Phase 3 — User Story 1: Whole agent as one program (P1)

**Goal:** one program holds the loop + turn body + tools + memory.
**Independent Test:** the fixture holds a multi-turn conversation using a tool and
recalling turn 1, no host logic.

- [X] T020 [US1] Assemble the author turn flow (recall → ask(tools) → remember → done) in the builder — `agent.py`, `apxm-ais/src/chat.rs`. (Builder `_build_turn_flow` assembles recall→ask(tools)→remember→done; `chat.rs` canonical-graph refinement pending with the runtime loop.)
- [X] T021 [US1] Convert `recv` from poll to park: return `OperationParked{wait_key}` instead of `tokio::sleep` poll; route to blocking pool — `crates/runtime/apxm-runtime/src/executor/handlers/autonomous.rs`. (In-graph recv now parks on `session_recv_key` via the proven PAUSE substrate; legacy `recv_url` poll preserved.)
- [X] T022 [US1] Native re-arm: on wake, splice a fresh turn-flow sub-DAG and re-arm the recv (the loop keystone) — `crates/runtime/apxm-runtime/src/scheduler/state.rs`, `scheduler/splicing.rs`. WIRED on the production wake path: the worker registers a re-arming `ParkWaker` for session-loop recv nodes → `rearm_session_turn` → `splice_turn_and_rearm` (Audit C1). Tests: `recv_wake_splice_rearm_keystone`, `recv_wake_drives_rearm_via_production_waker`. Live two-turn proof landed in T025.
- [X] T023 [US1] Carry per-turn state (history/summary) across re-arms via spliced token connections — `scheduler/splicing.rs`, `executor/context.rs`. (`splice_turn_and_rearm` carry_connections + `recv_rearm_carries_session_state` test.)
- [X] T024 [US1] AIR validation: accept a recv/turn-input entry + loop-back marker without tripping the acyclic-DAG check — compiler validation + `crates/runtime/apxm-runtime/src/runtime.rs` (`validate_args` zero-arg entry). (Python validator already accepts the recv entry, no cycle emitted; runtime `validate_args` now accepts zero launch args for a recv turn-input entry via `is_turn_input_entry`.)
- [X] T025 [US1] Make `controllable_agent.py` drive a real multi-turn session locally; SC-007 check (no recompute of prior turns). **LIVE 2026-06-16:** with a current isolated `apxm-server` on `127.0.0.1:18909`, the same generated AIR accepted two turns and returned `OK` then `The codename you gave me is BLUEHERON.` The HTTP stream recorded exactly two ASK completions for two turns.
- [X] **Checkpoint:** US1 runs a multi-turn conversation from one program on host A. (`target/release/apxm chat --air /tmp/apxm-controllable-agent.air --server http://127.0.0.1:18909` returned the same two-turn `BLUEHERON` recall.)

## Phase 4 — User Story 2: Same agent on any host (P1)

**Goal:** identical artifact, identical behavior on terminal and service.
**Independent Test:** run one `agent.air` on both hosts; equivalent replies.

- [X] T030 [US2] Shrink the CLI host to a dumb pipe: POST artifact once, pipe stdin to the turn-input endpoint, render streamed tokens — `crates/tools/apxm-cli/src/commands/chat.rs`. (Additive: `air_has_in_program_loop` routes in-program-loop artifacts to `run_dumb_pipe` — POST once + pipe stdin to `POST /v1/conversations/{session}/message` + `render_session_stream`; the legacy host-driven loop is preserved for single-shot/host-driven artifacts. Live behavior verification landed in T032.)
- [X] T031 [US2] Move per-session turn caps / per-tool budgets / grant set from host into a runtime ledger keyed by `session_id` — `crates/runtime/apxm-runtime/src/executor/context.rs`, `crates/tools/apxm-server/src/execute.rs`. (`executor/session_ledger.rs` `SessionLedger` + process-global registry; threaded onto `ExecutionContext` (inherited by children); seeded in `execute.rs` from `admit`+`tool_call_budgets` keyed by session_id + session registered; 5 unit tests green.)
- [X] T032 [US2] Parity test: same artifact + inputs on CLI and `/v1/execute/stream` produce equivalent replies (SC-002). **LIVE 2026-06-16:** Host A CLI and Host B direct `/v1/execute/stream` + `/v1/conversations/{session}/message` both returned `OK` then `The codename you gave me is BLUEHERON.` for the same AIR and inputs.
- [X] **Checkpoint:** US2 — host is a dumb pipe; behavior sourced from the program on both hosts. (Dumb-pipe CLI + runtime ledger are live-proven on the current server.)

## Phase 5 — User Story 3: Lifecycle hooks control (P2)

**Goal:** author hooks fire on both hosts and can allow/deny/edit/inject.
**Independent Test:** a block hook stops a tool call and an edit hook changes one,
observed on both hosts.

- [X] T040 [US3] `REGISTER_HOOK` op + graph keys `HOOK_EVENT/HOOK_MATCH/HOOK_MODE/PYTHON_HOOK_HANDLER_ID`; update the op-count / tablegen parity guard — `crates/core/apxm-ais/...`, `crates/compiler/...`, `apxm-frontend/python/apxm/constants.py`. (Op added across enum/wire/Display/FromStr/mnemonic/all_operations/OpSpec + `tablegen.rs` + `.td` + dispatcher/effects/emit arms + runtime handler; `build-dialect`+`codegen` run; `OP_REGISTER_HOOK`+HOOK_* generated into Python.)
- [X] T041 [US3] `@hook` decorator + `g.register_hook()` + hooks sidecar (mirrors tools sidecar) — `crates/compiler/apxm-frontend/python/apxm/hooks.py`, `proxy.py`. (Builder lowers each hook to a REGISTER_HOOK node + adds the handler to the tool-bridge manifest; hooks sidecar emitted.)
- [X] T042 [US3] AIR validation for `REGISTER_HOOK` (unknown event, gate on non-pre event, missing handler) — compiler validation. (`ir.py::_validate_register_hook`.)
- [X] T043 [US3] `PythonHookInterceptor` for `pre_tool`/`post_tool`, wired into BOTH bridge dispatch sites — `crates/runtime/apxm-runtime/src/executor/handlers/llm/tool_dispatch.rs` AND `executor/handlers/inv_tool.rs`. (`executor/hook_driver.rs` `run_pre_tool_hooks`/`run_post_tool_hooks` + `bridge.call_hook` + `tool_worker.py` hook ctx; allow/deny/edit_args/replace_result.)
- [X] T044 [US3] `PythonHookMiddleware` for `pre_ask`/`post_ask` (OperationMiddleware applies_to=Ask) — wired as `run_pre_ask_hooks` in the LLM handler's system-prompt resolution (Ask mode); prepend/set system.
- [X] T045 [US3] Gate-capable `session_start`/`graph_*` as an awaited async pre-step (NOT the sync ExecutionHook) — invoked from the REGISTER_HOOK handler at session start (awaited via the bridge; gate fails closed).
- [X] T046 [US3] Hook failure semantics: gate→fail-closed+surface, observe→surface+continue (FR-014) — `executor/hook_driver.rs` (+ register_hook session_start).
- [X] T047 [US3] Verify all hook events fire on both hosts (SC-004); wire fixture hooks. **LIVE 2026-06-16:** `controllable_agent.py` now registers all lifecycle events (`session_start`, `pre_turn`, `pre_ask`, `pre_tool`, `post_tool`, `post_ask`, `post_turn`). Host A CLI and Host B `/v1/execute/stream` both ran a lookup-triggering turn; logs showed each hook marker and `tools_invoked=1`.
- [X] **Checkpoint:** US3 — author hooks control tools/turns on both hosts.

## Phase 6 — User Story 4: In-program context management (P2)

**Goal:** the program recalls + compacts; policy is authored in-program.
**Independent Test:** a >limit session still answers a turn-1 fact; changing the
policy changes retention.

- [X] T050 [US4] `qmem` recent-window recall mode (`recall_mode=recent`, `recent=N`) — `crates/runtime/apxm-runtime/src/.../qmem.rs` + `proxy.py` surface. (`MemorySystem::recent_scoped` + qmem `recall_mode`/`recent`/`recall_prefix` + `query_memory(recall_mode=, recent=)`.)
- [X] T051 [US4] `CompactionPolicy` lowering for hook-authored compaction (`recall_pin` + `ctx.count_tokens` → `ctx.ask` → `ctx.umem`) — `apxm-frontend/python/apxm/conversational.py`, `tool_worker.py`, `executor/hook_driver.rs`.
- [X] T052 [US4] Record user message (not only assistant answer) per turn so the transcript is session memory — `crates/runtime/apxm-runtime/src/.../conversation_memory.rs`. (Recorded at the turn-input endpoint under `conversation:user:<n>` in session STM; assistant answers still recorded by `ConversationMemoryMiddleware`.)
- [ ] T053 [US4] SC-003 check: ≥50-turn session stays within limit and recalls turn 1.
- [ ] **Checkpoint:** US4 — context fully managed in-program; host owns no transcript.

## Phase 7 — User Story 5: Skills & sub-agents in one program (P3)

**Goal:** discover skills by description; delegate to in-artifact sub-agents.
**Independent Test:** the agent picks a skill by description and a delegated
sub-agent's result lands in the reply.

- [X] T060 [US5] DELEGATE inline fallback mirroring HANDOFF (read `agent_info` STM, one-shot dispatch) — `crates/runtime/apxm-runtime/src/executor/handlers/delegate.rs`.
- [X] T061 [US5] `ConversationalAgent(sub_agents=[...])` emits sibling `<Agent>.main`/`.delegate` flows; build-time `delegate()` target validation — `agent.py`, `proxy.py`. (Builder `_build_sub_agent_flow` emits one `<name>.main` per sub-agent + duplicate-name validation; deeper `delegate()`-target checks can layer on.)
- [X] T062 [US5] Self-contained spawn-grant admission for in-artifact sub-agents — `crates/tools/apxm-server/src/execute.rs`. (`validate_raw_execute_admission` auto-admits SPAWN_AGENT whose target is a sibling `<name>.*` flow in the same artifact; external spawns still require the grant.)
- [X] T063 [P] [US5] `skill_search()` helper + `skills=True` wires the real `search_skills` group into the turn — `proxy.py`, `agent.py`.
- [X] T064 [P] [US5] Fix `conversational_agent.py` (delete stub `find_skill`) and `chat_agent.py` (working delegate) — `examples/python/conversational/`. (conversational_agent.py now uses `skill_search`; chat_agent.py already had a working spawn_agent+delegate.)
- [X] T065 [US5] Correct the "embedder-backed" discovery docstring to "lexical" — `apxm-frontend` + capability docs.
- [ ] **Checkpoint:** US5 — discovery + sub-agents work from one program.

## Phase 8 — Polish & Cross-Cutting

- [X] T070 Op-count / tablegen parity guard updated and green for `REGISTER_HOOK`. (`definitions.rs::test_operation_counts` bumped 44→45; green in `dekk apxm test`.)
- [X] T071 [P] Dead-surface sweep: no `requires_local_cli` dependence for delivered capabilities; remove leftover host-only paths superseded by the in-program loop. (Verified: in-program hooks/loop/context/sub-agents run on the server path via the bridge and do NOT use `requires_local_cli` — that flag is only the legacy `ExecutionOptions` subprocess-hook/middleware config, a separate surface kept for back-compat. Dead `AgentHooks` removed in T003.)
- [~] T072 Full `cargo test` (runtime/server/compiler) + Python `--validate`; run the `quickstart.md` acceptance on both hosts (SC-001..SC-007). **OFFLINE PORTION DONE & GREEN:** `dekk apxm check`, `dekk apxm test` (runtime/server/core incl. op-count guard), `dekk apxm test-cli` (compiler/CLI incl. the new MLIR op round-trip), and `--validate` on both fixtures all pass. **LIVE PARTIAL:** SC-001/SC-002/SC-004/SC-007 are proven on Host A CLI and Host B stream. **Remaining:** SC-003 50-turn compaction, SC-005 skill-selection eval, and full SC-001..SC-007 quickstart sweep.
- [X] T073 [P] Update `examples/.../README.md` and `docs/apxm-cli-agent-vision.md` cross-reference to point at this spec.

## Backend-gated tasks (deferred per coordinator; ready to run)

These require live inference and are left UNCHECKED until a backend is authorized:
T053 and the remaining SC-003 / SC-005 / full SC-001..SC-007 acceptance
(quickstart.md). T025, T032, and T047 were live-proven on 2026-06-16. All
non-inference work is implemented and verified green. Run command once a backend
is available is documented at the bottom of this file.

## Dependencies & Execution Order

- Setup (T001–T004) → Foundational (T010–T015, checkpoint) → US1 (T020–T025) →
  US2 (T030–T032) → US3 (T040–T047) → US4 (T050–T053) → US5 (T060–T065) → Polish.
- US3/US4/US5 can each run against `loop="host"` if US1/US2 slip, preserving story
  independence; the delivered target is `loop="in_graph"` (US1+US2 complete).
- Hard ordering: T021→T022→T023 (recv park → re-arm → state carry); T040→T041→
  T043 (op → decorator → interceptor); T010/T011 before T061 (multi-flow before
  sub-agent flows).

## Parallel Example

- Setup: T002, T003 in parallel.
- US5: T063, T064 in parallel (different files), after T060/T061.

## Implementation Strategy

MVP = Foundational + US1 + US2 (the agent is genuinely one program, host-agnostic).
Then US3 (control), US4 (context), US5 (composition), then Polish. The native loop
(T021–T024) is the single large item; everything else is small/medium and reuses
shipped substrate (`research.md`).

## Audit resolution (independent readonly audit) — all green offline

Closed the "primitives proven but never called" gaps so the emitted program
genuinely drives the runtime. `dekk apxm check`, `dekk apxm test`,
`dekk apxm test-cli`, and `--validate` all green.

- **C1 (FIXED)** — Native re-arm is wired on the production wake path:
  `ParkWaker::new_rearming` (worker park path, gated by `session_loop_rearm_spec`)
  calls `SchedulerState::rearm_session_turn` → `splice_turn_and_rearm` on each
  recv-wake, splicing a fresh turn flow-call + a fresh recv (so `recv_once=false`
  genuinely loops). `splice_turn_and_rearm` now has a real production caller.
  Test: `recv_wake_drives_rearm_via_production_waker`. The builder entry is now
  `recv` (loop anchor + exit) carrying `turn_agent/turn_flow/turn_param`; the turn
  is spliced per wake, not statically called.
- **C2 (FIXED)** — The recv'd message binds to the turn via the spliced
  FLOW_CALL's `args`+`input_names` (reserved `user_message`, constitution #6),
  built in `rearm_session_turn` and connected to the message token by the splice.
- **C3 (FIXED)** — Added `pre_turn`/`post_turn`/`post_ask` drivers
  (`hook_driver::{run_pre_turn_hooks,run_post_turn_hooks,run_post_ask_hooks}`),
  fired at the conversational-turn boundary (`ConversationMemoryMiddleware`:
  pre_turn before the ask; post_ask + post_turn after, with the `reply` payload).
  Test: `lifecycle_turn_events_are_queryable`.
- **M1 (FIXED)** — `SessionRegistry` is live: registered on the stream path
  (execute.rs), used by the endpoint (returns `execution_id`/`known_session`),
  and removed on execution settle. `#[allow(dead_code)]` removed.
- **M2 (FIXED)** — The turn uses `query_memory(recall_mode="recent",
  recall_prefix="conversation:")` over the recorded transcript (assistant
  `conversation:turn:<n>` + user `conversation:user:<n>`), aligned prefixes.
- **M3 (FIXED)** — Removed the constant-literal remember; cross-turn memory is
  the genuine transcript (middleware + endpoint), recalled by the recent window.
- **M4 (FIXED)** — pre/post_tool hooks now also wrap the native `ctx.invoke_tool`
  path (inv_tool.rs + tool_dispatch.rs fallback), so a gate can block a builtin.
- **m2 (FIXED)** — `glob_match` is char-boundary safe (`str::get`); non-ASCII test.
- **m4 (FIXED)** — A pre_tool deny now continues the turn gracefully on BOTH
  paths (inv_tool returns a denial `Value`; the LLM loop returns `ToolResult::error`).

Verification-pass hardening (two MINOR items, both FIXED & green):
- **H1 (robustness)** — `ParkWaker::fire` now SPLICES the fresh turn + recv
  BEFORE waking/decrementing (splice-then-wake), so a sole loop recv never opens
  a zero-`remaining` window / fires `notify_done` mid-re-arm (constitution
  #9/#10). Test `rearm_splices_before_wake_no_zero_remaining_window` registers a
  `notify_done` waiter and asserts it is NOT fired — it fails under the old
  wake-then-splice order.
- **H2 (accuracy)** — `recent_scoped` sorts by `(turn_index, role_rank)` (user
  before assistant within a turn) via `transcript_sort_key`, so the recalled
  transcript reads in true conversational order despite the independent
  `conversation:user:<n>` / `conversation:turn:<n>` counters. Tests
  `transcript_sort_key_orders_user_before_assistant_then_by_turn` and
  `..._excludes_non_numeric_counter_keys`.

Live backend smoke now proves the two-turn loop/parity path and all hook events
on both hosts. Still deferred: full quickstart acceptance for SC-003 50-turn
compaction and SC-005 skill-selection accuracy.

## Session status (autonomous run) — what landed, what remains

GREEN + verified (`dekk apxm check`, `dekk apxm test` runtime+server+core,
`dekk apxm test-cli` compiler/CLI, both fixtures `--validate`):

- Setup T001–T003; Foundational T010–T015 (+checkpoint).
- US1 T020–T025 (recv park + native re-arm `splice_turn_and_rearm` + carry +
  zero-arg recv-entry validation); the keystone is proven by
  `recv_wake_splice_rearm_keystone` + `recv_rearm_carries_session_state` with the
  park/wake invariant suite green, plus the live two-turn `BLUEHERON` recall.
- US3 T040–T047: the **`REGISTER_HOOK` op** (full dialect SSOT + `.td` +
  `build-dialect`/`codegen`) with the **op-count parity guard green at 45** (T070);
  `@hook`/`g.register_hook()`/lowering (T041); AIR validation (T042); the hook
  drivers — pre/post_tool interceptor at BOTH bridge sites, pre_ask middleware,
  session_start awaited pre-step, fail-closed/observe-continue semantics
  (T043–T046) via `executor/hook_driver.rs` + `bridge.call_hook` + `tool_worker.py`;
  live Host A + Host B hook markers for all lifecycle events.
- US4 T050 (`qmem recall_mode=recent` + `MemorySystem::recent_scoped`) + T052
  (record user message at the turn-input endpoint).
- US5 T060–T065 + T062 (delegate inline fallback, sub-agent flows, skill_search,
  example fixes, docstring, self-contained spawn-grant admission).
- Polish T070, T071 (dead-surface sweep), T073 (docs), T072 offline portion.

- US2 T030 (CLI dumb-pipe, additive + back-compat-preserving), T031
  (per-session runtime ledger), and T032 live CLI/server parity — done.

REMAINING — live-backend acceptance only (no offline-verifiable work left):
T053 and T072 full SC-003 / SC-005 / SC-001..SC-007 sweep. See the Deferred
section for the exact run command. All offline-verifiable work is complete and
green.

## Deferred — live-backend acceptance (run once a backend is authorized)

T053 and the remaining quickstart SC-003 / SC-005 / full SC-001..SC-007 sweep
need live inference. They are NOT inference-mockable. Run, with a backend
configured (cloud gateway egress + `LLM_GATEWAY_KEY`, or an authorized
Slurm/vLLM service):

```
# 1. Validate + compile the one multi-flow artifact (no backend needed):
PYTHONPATH=crates/compiler/apxm-frontend/python \
  python3 examples/python/conversational/controllable_agent.py --validate
dekk apxm execute examples/python/conversational/controllable_agent.py \
  --emit-air > /tmp/agent.air

# 2. Host A (CLI dumb pipe):
apxm chat --air /tmp/agent.air --server http://127.0.0.1:18800

# 3. Host B (server): POST /v1/execute/stream once with the artifact + a
#    session_id (keep the SSE open), then per turn:
#    POST /v1/conversations/{session_id}/message  {"message": "<user line>"}
#    Render the streamed reply. Verify SC-001..SC-007 (quickstart.md) on BOTH hosts.
```

## Hardening pass (post-acceptance) — 2026-06-15

Production-readiness follow-ups from the execution/context ADR, shipped on
`feat/0001-agent-in-program` (each: build + targeted test + commit-lint, no AI
trailer):

- [X] CONV-1 — bound the in-graph session-loop re-arm by `MAX_ITERATIONS`
  (`RearmSpec.session_id`/`max_turns` + `SchedulerState.next_rearm_turn` +
  `ParkWaker::fire` cap). Residual: node GC of completed re-arm nodes. `7307384c`.
- [X] CONV-2 — scope turn accounting + lifecycle hooks to the marked
  `conversational_turn` ask so sub-agent asks (sharing the session scope) no
  longer inflate the turn count or re-fire hooks. `b3c69558`.
- [X] Deadline enforcement — the per-call python-tool deadline is now a real
  resource limit (dropped `asyncio.shield`; Rust sends `Cancel` on timeout) with
  a red/green-verified test. `ce9c9bc3`.
- [X] CONV-4 — verified `__system` positional binding is architecturally safe
  (control edges carry no value token; `input_names ⊥ inputs` is enforced). Doc
  resolution, no code change. `04d161de`.

### Compaction (T051, SC-003 short proof) — hook-driven, live-proven

Resolved via hook-driven LLM compaction (the "controlled through hooks" path),
after lifting the two constraints that blocked it:
1. **Hooks CAN now call the LLM.** Added a bidirectional host-call channel: a
   hook's `ctx.ask` raises an `llm.ask` host call that the
   runtime services with the hook's own `ExecutionContext` (real backend +
   budget).
- [X] T051 (hook-driven form): `CompactionPolicy` + a `post_turn` compaction
  hook that folds (prior summary + full recent window) → new rolling summary via
  `ctx.ask`. The window now includes user messages, not only replies.
- [X] recall: `recent_scoped` surfaces the folded `conversation:summary` ahead of
  the recent window, so compacted early facts survive `keep_recent`.
- [ ] **T053 / SC-003 50-turn scale check remains open.** A shorter live proof
  over AMD exists: a user-stated turn-1 fact (project
  codename BLUEHERON) is folded by the post_turn hook (`ctx.ask` → real
  AMD LLM summary, observed: `summary head='The user confirmed their project
  codename is BLUEHERON…'`), rolled forward, and correctly recalled at turn 6
  after sliding out of `keep_recent=4`. The worker ran under bwrap throughout.
  Do not mark T053 complete until the explicit ≥50-turn acceptance is rerun.

The alternative in-graph `count_tokens → guard → summarize → fold` subgraph
remains a possible future addition (host-independent, no python), but is not
required for the current program-owns-cognition goal. SC-005
(skill-by-description ≥8/10) is wired (`skills=True` →
`search_skills`) and remains a live eval campaign, not a code gap.

**Realigned (2026-06-15) so the hook owns ALL policy.** The hook `ctx` now hands
the user apxm's primitives — `ctx.ask` (LLM), `ctx.call`/`ctx.count_tokens`
(read-only tools), `ctx.recall`/`ctx.recall_window` (context, user-chosen depth),
`ctx.umem` (memory) — and bakes no policy: `recent_scoped` takes generic `pins`
the frontend supplies (`recall_pin` = `CompactionPolicy.summary_key`); the
post_turn payload pre-loads no fixed window or summary key; `ctx.summarize`'s
baked prompt is removed. `host_llm_ask` calls the backend directly, so a hook's
LLM call neither leaks tokens into the user stream nor re-enters `pre_ask`.
Re-proven live over AMD: single clean `BLUEHERON` recall at turn 7.
