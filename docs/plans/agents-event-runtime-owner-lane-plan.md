# Agents event-runtime owner lane plan

Status: owner lane plan for the Agents share of the accepted workspace ADR-0027
event-runtime replacement. It is subordinate to Agents ADR-0018 (portable
event/readiness/activation/commit semantics), Agents ADR-0019 (builtin
Capabilities own no durable scheduling), and Agents ADR-0013 (closed semantics
behind exact Port Bindings).

Date: 2026-07-31

Measured baseline: Agents revision `1af8d4f68c2aef2ba6b653837f5546dd80f00265`.
Every path, test file, and gate named below is verified present at that
revision. Line-level facts cited as current behaviour were read at that
revision.

## What this plan is and is not

The detailed engineering content for this work already exists in the owner
contract and the parallel replacement plan:

- [Event-driven runtime and scheduler contract](../agents/event-driven-runtime-and-scheduler-contract.md)
  — the semantic contract.
- [Event-driven runtime full-replacement plan](../agents/event-driven-runtime-full-replacement-plan.md)
  — phases 0–8, the cross-plane DAG, worktree serialization, join gates, the
  cutover, and the definition of done.

This document does not restate them. It is the Agents lane register: for each
lane coordinate the master plan assigns to this repository, it names the
file-level surfaces the lane writes, the repository-local gates that prove the
lane, and the exact joins that consume it. A coordinate absent from this table
blocks `J1`, so the table is complete by construction rather than
representative.

Lane identifiers are plan coordinates only. They never become schema ids,
routes, feature flags, metric names, or branch names.

## Repository disposition

Implement portable typed event/readiness/activation/commit semantics and local
execution; remove the sequential driver walk, the polling-shaped event port,
string-constructed event references, and the capability-owned scheduler.

## Measured starting state

These are facts about the pinned revision, not target claims.

- `crates/runtime/execution/src/structural.rs` builds a flat
  `Vec<ScheduleStep>` in `build_schedule`, with loops emitted as `EnterLoop` /
  `LoopBackEdge` markers around one linear pass and branch arms flattened
  unconditionally.
- `crates/runtime/execution/src/driver.rs` walks that vector in one ordered
  `for` loop inside the private `drive_from`, and rebuilds the schedule on every
  call including every resume.
- `crates/runtime/execution/src/resume.rs` persists `Continuation` with a bare
  `next_schedule_position: usize` index into an unpersisted schedule.
- `crates/runtime/execution/src/ports.rs` defines `EventRef` as a
  `#[serde(transparent)]` newtype over `String` with a public
  `EventRef::new(impl Into<String>)` constructor, and `EventPort::await_event`
  returns `EventOutcome::Parked` as a polled outcome.
- `crates/compiler/frontend/python/apxm_program/_markers.py` exposes
  `Event = _EventFactory()` whose `__call__(target_ref: str)` accepts a string
  and defaults `type_ref` to the literal `"Event"`;
  `crates/compiler/frontend/typescript/src/markers.ts` has the equivalent
  `Event<T>(targetRef, typeRef = "Event")`.
- `crates/runtime/capability/src/builtins/schedule.rs` and `store.rs`
  implement a durable one-shot/recurring timer over a rusqlite table with a
  background firer, reached through the `CapabilityHost` wake bridge in
  `crates/runtime/capability-iface/src/host.rs`. The `sqlite` feature that
  gates them is default-on in `crates/runtime/capability/Cargo.toml`.
- `crates/runtime/capability-iface/src/lib.rs` and `host.rs` document a
  three-module `apxm-runtime` crate with a `scheduler` module and a
  `park_registry`. Neither exists in the workspace member list.
- `crossbeam-deque = "0.8"` is declared at workspace root `Cargo.toml:97` with
  no consuming crate and no source use.
- No `loom` or `proptest` dependency exists anywhere. `criterion` exists only
  in `crates/runtime/backends/Cargo.toml` for the `vllm_hints` bench.
- `crates/runtime/execution/Cargo.toml` and `crates/runtime/kernel/Cargo.toml`
  carry `tokio` as their only dev-dependency.

## Lane register

### `A-FE` — unforgeable typed Event flow

Surfaces:

- `crates/compiler/frontend/python/apxm_program/_markers.py` (the `Event`
  factory and `EventType`), `_capture.py` (the `event_type` declaration and
  `event_wait` intent), `_bound_tree.py`, `_emit.py`, `__init__.py` exports.
- `crates/compiler/frontend/typescript/src/markers.ts`, `capture.ts`,
  `index.ts`, `contract.ts`.
- `crates/compiler/frontend/native/python/src/lib.rs` and
  `crates/compiler/frontend/native/typescript/src/lib.rs`.
- `crates/machine/program/src/frontend_graph.rs` (`DeclKind::EventType`,
  `IntentKind::EventWait`, their validation), `lower.rs` (the
  `IntentKind::EventWait -> SemanticOpKind::AwaitEvent` mapping and the
  `event_ref` operand emission), `air.rs`, `artifact.rs`,
  `runtime_evidence.rs`.
- `crates/runtime/execution/src/ports.rs` — the `EventRef` newtype and its
  public string constructor.
- `contracts/schemas/apxm.frontend-graph.v2.json`,
  `apxm.frontend-surface.v1.json`, `apxm.air.v2.json`,
  `apxm.executable-artifact.v1.json`, `apxm.runtime-evidence.v1.json` and their
  `contracts/vectors/` counterparts.
- `examples/agents/conversational/src/parity-agent.ts`, which currently
  constructs `Event<unknown>("parity.event.v1")` and must move to the typed
  form with the rest of the surface.

Local gates: `dekk agents check-frontend-parity`
(`tools/scripts/check_frontend_parity.py`), `dekk agents
check-frontend-surface` (`tools/scripts/check_frontend_surface.py`), `dekk
agents test-python-frontend` over
`crates/compiler/frontend/python/tests_program/`, `dekk agents
test-typescript-frontend` over `crates/compiler/frontend/typescript/test/`,
`dekk agents test-program` over `crates/machine/program/tests/`, `dekk agents
test-frontend-examples`, `dekk agents owner-descriptor`.

Joins: `J1` for landing, then `V-AG`, `V-E2`, `J2`, `J3`.

### `A-PL` — immutable execution plan and deterministic reference

Surfaces: `crates/runtime/execution/src/structural.rs` (the flat
`ScheduleStep`/`build_schedule` pair it replaces),
`crates/runtime/execution/src/driver.rs` (`drive_from` and the ordered walk),
`crates/runtime/execution/src/lib.rs` (the new plan/readiness/runner module
roots), `crates/runtime/execution/src/resume.rs` (the schedule-index
continuation), `crates/machine/program/src/air.rs` as the plan's admitted
input.

Local gates: `dekk agents test-execution`, `dekk agents test-runtime-seams`,
`dekk agents test-program`, the migrated
`crates/runtime/execution/tests/{end_to_end,loop_evidence,resume_and_ports}.rs`,
`dekk agents clippy`.

Joins: `J1` and `A-FE` for landing; consumed by `A-RD`, `A-AR`, `A-RM`, then
`V-AG`, `V-E2`.

### `A-RD` — readiness kernel

Surfaces: new readiness modules under `crates/runtime/execution/src/`
registered in `crates/runtime/execution/src/lib.rs`; operand and predecessor
state consumed from the `A-PL` plan; `crates/machine/program/src/runtime_evidence.rs`
for the semantic readiness and `NodeExecutionRecorded` facts; model tests
declared through new dev-dependencies in `crates/runtime/execution/Cargo.toml`,
which today carries `tokio` alone.

Local gates: `dekk agents test-execution`, `dekk agents test-runtime-seams`, and
the loom/model suite this lane introduces. `crates/runtime/execution/tests/`
gains the cancellation-race, duplicate-writer, and quiescence cases; the
existing inline `#[cfg(test)]` module in `structural.rs` that covers loop
back-edge ordering moves with the plan.

Joins: `A-PL` for landing; consumed by `A-LX`, `A-AR`, `V-AG`.

### `A-EC` — commit and prepared-effect reducers

Surfaces: `crates/runtime/kernel/src/commit.rs` (`ExecutionCommitPort`,
`ExecutionCommitRequest`, `AtomicWriteSet`, `ExecutionCommitResult`, and the
defaulted `load_continuation`), `crates/machine/program/src/execution_commit.rs`,
`crates/runtime/execution/src/bundle.rs` for the admitted port set,
`contracts/port-contracts/apxm.execution-commit.port-contract.v1.json`,
`contracts/vectors/apxm.execution-commit.v1.json`.

Local gates: `dekk agents test-kernel` over
`crates/runtime/kernel/tests/{commit_projection,lifecycle,bundle}.rs`, `dekk
agents test-program`, `dekk agents owner-descriptor`.

Joins: `J1` for landing; consumed by `A-AR`, Server `S-EW`/`S-RI`, Adapters
`D-EF`/`D-ST`, then `V-AG`, `V-E2`.

### `A-AR` — storage-neutral ActivationRunner

Surfaces: a new runner module under `crates/runtime/execution/src/` exported
from `crates/runtime/execution/src/lib.rs`; `crates/runtime/execution/src/resume.rs`
for checkpoint and end-of-activation meaning;
`crates/runtime/execution/src/bundle.rs` for claim-time port admission;
`crates/runtime/kernel/src/commit.rs` for the fence the runner enforces;
`contracts/port-contracts/apxm.durable-event.port-contract.v1.json` for the
claim-side event boundary.

Local gates: `dekk agents test-execution`, `dekk agents test-kernel`, `dekk
agents test-runtime-seams`, deterministic-parity cases in
`crates/runtime/execution/tests/resume_and_ports.rs`.

Joins: `A-PL`, `A-RD`, `A-EC` for landing; consumed by Server `S-RI`, Adapters
`D-ST`, then `V-AG`, `V-E2`, `J2`, `J3`.

### `A-QP` — audited queue decision and harness

Surfaces: workspace root `Cargo.toml` (`crossbeam-deque = "0.8"` at line 97,
declared and unconsumed; it is a candidate, not an accepted selection),
`crates/runtime/execution/Cargo.toml` for the loom/criterion/stress
dev-dependencies and the bench target this lane adds, and the audit record that
lands with the decision. The only existing bench in the workspace is
`crates/runtime/backends/benches/vllm_hints.rs`; the scheduler harness is new
and does not reuse it.

Local gates: the pinned-source and soundness audit, the loom/model harness
running green on a stub queue, and `dekk agents clippy`.

Joins: `A-C1` and an accepted soundness decision for landing; consumed by
`A-LX`, `A-PF`, `V-PF`.

### `A-LX` — bounded local work-stealing executor

Surfaces: new executor, injector, parker, and observation modules under
`crates/runtime/execution/src/` registered in
`crates/runtime/execution/src/lib.rs`; the readiness output from `A-RD` as its
sole semantic input; `crates/runtime/capability-iface/src/events.rs`, whose
`emit_scheduler_decision(&self, _node_id: u64, _delay: Duration, _reason: &str)`
observer hook keys on a `u64` node id while the plan and driver identify nodes
by `String`, so the observation seam is rewritten here rather than extended.

Local gates: `dekk agents test-execution`, the loom/model park-wake and
worker-loss suites, randomized-schedule parity against the `A-PL` deterministic
reference, `dekk agents clippy`.

Joins: `A-RD` and the accepted `A-QP` queue decision for landing; consumed by
`A-PF`, `A-RM`, `V-AG`, `V-PF`.

### `A-PF` — digest-bound performance admission

Surfaces: the checked-in threshold policy naming corpus and profile digests,
supported topology classes, and the deterministic and non-stealing baselines;
the bench targets declared in `crates/runtime/execution/Cargo.toml`. Benchmark
output is written only under `.apxm/benchmarks/results/` per the repository
artifact-placement rule and is never committed.

Local gates: the scaling, NUMA, overload, and reclamation matrices run against
the checked-in policy; a topology class with no passing policy result is not
admitted and does not select a fallback scheduler.

Joins: `A-LX` for landing; consumed by `V-PF`, `J4`.

### `A-C1` — owner schemas, vectors, and generator

Surfaces: `contracts/descriptors/apxm.agents-owner-descriptor.v1.json` and its
`.sha256` sidecar, the 23 files under `contracts/schemas/`, the 25 under
`contracts/vectors/`, the 6 under `contracts/port-contracts/`,
`contracts/tools/validate_owner_descriptor.py`, and the owner-local generator
entry point `tools/scripts/generate_contract_bindings.py` with its checked-in
outputs `crates/compiler/frontend/python/apxm_program/_generated/runtime_evidence.py`,
`crates/compiler/frontend/typescript/src/generated/runtime-evidence.ts`, and
`crates/tools/cli/generated/`.

Authority: this lane needs no new ADR. Agents ADR-0013 fixes closed semantics
and exact Port Contracts, Agents ADR-0018 fixes the portable event/readiness
semantics the schemas encode, and workspace ADR-0010 fixes digest identity for
owner descriptors. The lane publishes bytes under existing accepted authority.

Local gates: `dekk agents owner-descriptor-sync` then `dekk agents
owner-descriptor`; `dekk agents check-contract-codegen`; `dekk agents
check-frontend-codegen`; `dekk agents check`.

Joins: `P1` for landing; consumed by `S-C1`, `D-C1`, `C-G1`, `J1`, `V-CT`.

### `A-CLI` — CLI over generated clients

Surfaces: `crates/tools/cli/src/client/{mod,events,execute}.rs`, whose
`events.rs` currently parses SSE frames by reading a `"kind"` string and
matching the literal `"approval_request"` locally;
`crates/tools/cli/generated/{python/event_v1.py,typescript/event-v1.ts,typescript/core-event-kinds.ts,typescript/generated.ts}`;
`crates/tools/cli/src/commands/{canonical_execute,canonical_air,compile_service_canonical,session,watch,sse_permissions,rollout}.rs`;
`crates/tools/cli/src/commands/cli.rs` for the command enum. Agents ADR-0004
already fixes the rule this lane implements — remote CLI commands are
generated-client consumers, local commands consume public compiler/runtime APIs
— so the lane adds no decision.

Local gates: `dekk agents test-cli`, `dekk agents check-frontend-codegen`,
`dekk agents check`.

Joins: `J1` and a frozen Server query shape for landing; consumed by `S-OQ`,
`V-CU`, `V-E2`, `J3`.

### `A-RM` — removal of the replaced paths

Surfaces, each with a named replacement before deletion:

- the flat walk in `crates/runtime/execution/src/structural.rs` and the ordered
  loop in `crates/runtime/execution/src/driver.rs`, replaced by `A-PL`/`A-RD`;
- the polling-shaped `EventPort`/`EventOutcome::Parked` boundary and the
  `EventRef::new(impl Into<String>)` constructor in
  `crates/runtime/execution/src/ports.rs`, replaced by `A-FE`;
- the string event constructors in
  `crates/compiler/frontend/python/apxm_program/_markers.py` and
  `crates/compiler/frontend/typescript/src/markers.ts`, and their use in
  `examples/agents/conversational/src/parity-agent.ts`, replaced by `A-FE`;
- `crates/runtime/capability/src/builtins/schedule.rs`,
  `crates/runtime/capability/src/builtins/store.rs`, their `#[cfg(feature =
  "sqlite")]` module and re-export lines in
  `crates/runtime/capability/src/builtins/mod.rs`, the `sqlite`/`rusqlite`
  feature entries in `crates/runtime/capability/Cargo.toml`, and
  `crates/runtime/capability-iface/src/host.rs`, all deleted under Agents
  ADR-0019 with Server owning schedules;
- the stale `apxm-runtime` scheduler and `park_registry` rationale in
  `crates/runtime/capability-iface/src/lib.rs` and the
  `emit_scheduler_decision` observer in
  `crates/runtime/capability-iface/src/events.rs`;
- `crossbeam-deque` at workspace root `Cargo.toml:97` if the `A-QP` audit does
  not accept it;
- superseded cases in `crates/runtime/execution/tests/`,
  `crates/runtime/kernel/tests/`, and `crates/machine/program/tests/`.

Local gates: `dekk agents test-canonical-only`
(`tools/tests/test_canonical_only_reachability.py`, whose retired-directory and
retired-file lists this lane extends), `dekk agents test-owner-gates`
(`tools/tests/test_owner_gate_commands.py`, which refuses a lane gate cited
here that `dekk agents` does not declare), `dekk agents test`, `dekk agents
check`, `dekk agents clippy`, and a `git grep` absence scan for the removed
constructors and module paths.

Joins: `A-AR`, `A-LX`, Server `S-RI` for landing; consumed by `X-DL`, `V-LA`,
`J4`. The target branch remains unreachable from production composition until
`R4`.

### `V-AG` — Agents verification join

Consumes the exact green revisions of `A-FE` through `A-LX`. It runs frontend
parity, deterministic-executor parity, the loom and model suites, randomized
parallel parity, cancellation and quiescence, worker loss, and evidence tests
across `crates/runtime/execution/tests/`, `crates/runtime/kernel/tests/`,
`crates/machine/program/tests/`,
`crates/compiler/frontend/python/tests_program/`, and
`crates/compiler/frontend/typescript/test/`. Repository entry point: `dekk
agents test` plus the focused `test-execution`, `test-kernel`, `test-program`,
`test-python-frontend`, and `test-typescript-frontend` commands. It gates `J2`.

### `V-PF` — performance join

Consumes `A-PF` green together with Server capacity policy. It joins the local
scheduler throughput, p50/p95/p99 ready-to-start, fairness, NUMA, overload,
memory, and no-busy-polling evidence against the checked-in digest-bound
threshold policy, and admits only the topology classes with a passing result.
Benchmark output stays under `.apxm/benchmarks/results/`. It gates `J4`.

## Coordinate-to-join summary

| Lane | Landing dependency | Consuming joins |
| --- | --- | --- |
| `A-C1` | `P1` | `C-G1`, `J1`, `V-CT` |
| `A-FE` | `J1` | `V-AG`, `V-E2`, `J2`, `J3` |
| `A-PL` | `J1`, `A-FE` | `V-AG`, `V-E2`, `J2` |
| `A-RD` | `A-PL` | `V-AG`, `J2` |
| `A-EC` | `J1` | `V-AG`, `V-E2`, `J2`, `J3` |
| `A-AR` | `A-PL`, `A-RD`, `A-EC` | `V-AG`, `V-E2`, `J2`, `J3` |
| `A-QP` | `A-C1` plus an accepted soundness decision | `V-PF` |
| `A-LX` | `A-RD`, accepted queue decision | `V-AG`, `V-PF`, `J2` |
| `A-PF` | `A-LX` | `V-PF`, `J4` |
| `A-CLI` | `S-OQ`, generated clients | `V-CU`, `V-E2`, `J3` |
| `A-RM` | `A-AR`, `A-LX`, `S-RI` | `X-DL`, `V-LA`, `J4` |
| `V-AG` | `A-FE`–`A-LX` green | `J2` |
| `V-PF` | `A-PF` green | `J4` |

## Scope closure

Every surface family named above carries a stable `P2/agents/...` coordinate in
`scope/fragments/agents.scope-fragment.json` at the coordinator, pinned to
revision `1af8d4f68c2aef2ba6b653837f5546dd80f00265`. A surface discovered later
joins its owning lane here and its coordinate there; it is never deferred,
optional, or out of scope.
