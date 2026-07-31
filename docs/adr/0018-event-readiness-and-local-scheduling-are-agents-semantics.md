---
status: accepted
date: 2026-07-30
owner: APXM agents
requires: accepted workspace ADR-0027
amends: ADR-0003, ADR-0004, ADR-0007, ADR-0008, ADR-0011, ADR-0013
---

# Event readiness and local scheduling are Agents semantics

## Status and authority

This ADR is accepted owner authority together with workspace ADR-0027. It
authorizes the Agents-owned contract and runtime implementation described
below, but does not claim that implementation or production activation is
complete. The shipped baseline still delegates durable external event delivery
through a replaceable port and walks a derived structural schedule
sequentially; the relevant implementation is
`crates/runtime/execution/src/{driver,structural,ports,resume}.rs` at Agents
revision `3e5a78ef3ae8f645fd24390244b039f8bacaa64c`.

## Context

The current owner contract already makes graph scheduling, NodeExecution
identity, `EventRef<T>`, Program Invocation lifecycle, and Execution Commit
meaning concrete Agents semantics. It also describes a replaceable durable
event port. The current driver does not yet implement dependency-driven
parallel execution: `build_schedule` materializes a flat
`Vec<ScheduleStep>`, and `drive_from` awaits each step in one ordered loop. The
workspace declares `crossbeam-deque = "0.8"`, but no Agents source uses it at
the pinned baseline.

Research into DARTS, ARTS, OCR, and SWARM shows several useful but limited
patterns: DARTS, ARTS, and OCR make readiness explicit; ARTS and OCR use
opposite-end owner/thief deques; SWARM separates speculative execution from
irrevocable commit; and placement hints do not define readiness. None provides
APXM's durable occurrence acceptance, Program Instance authority, external-
effect fencing, or per-activation process-crash recovery.

The target requires a precise owner boundary. Moving all scheduling into one
module would conflate three different mechanisms:

1. graph dependency and control readiness;
2. local CPU worker placement and work stealing; and
3. durable activation persistence, leasing, retry, and recovery.

## Decision

### Agents owns portable event and readiness semantics

Agents owns the portable meaning of:

- the internal immutable `EventTypeRef` plus payload-schema digest;
- the public typed `Event<T>` reference value backed by an unforgeable
  `EventRef<T>`, with no second public definition or string constructor;
- `EventOccurrence<T>` and the closed portable `EventProvenance` structural
  variants and fields;
- EventRef reservation, exact wait binding, fulfillment association,
  consumption, abandonment, expiry, cancellation, and deterministic restore;
- root-activation and awaited-fulfillment target semantics;
- immutable NodeOccurrence and activation identities;
- dependency publication and the blocked-to-runnable transition;
- structured joins, cancellation propagation, and continuation checkpoints;
- NodeExecution, attempt, effect, and Program Invocation state transitions;
- the PXM transition reducers that define EventRef, activation,
  prepared-effect, checkpoint, Execution Commit, and evidence consequences;
- the local activation runner and local worker scheduling policy; and
- the one fenced Execution Commit that makes Program state and evidence true.

These semantics are portable library behavior under an exact admitted runtime
composition. They contain no HTTP, broker, SQL, provider, host, or deployment
selection logic.

### Accepted I/O has provenance, not another event state machine

An input from HTTP, a provider integration, a Host, or a physical device
becomes a typed `EventOccurrence<T>` only after the owning ingress
boundary has authenticated, normalized, and admitted it. Its source, source
sequence, timestamps, content digest, clock quality, and transport facts are
represented by a closed common provenance envelope plus an exact content-
addressed source descriptor. A new webhook, user-message channel, broker,
device, or future transport does not add a runtime event variant. A schedule
fire is instead an internally materialized occurrence with schedule
provenance. These sources do not create an `IoEvent` lifecycle parallel to the
EventRef lifecycle. Raw samples, occurrence candidates, local I/O completions,
outbound effects, delivery acknowledgements, and telemetry remain separate
closed types.

Frequency is not part of event meaning. An exact source contract may admit each
item or explicitly reduce/window/sample before admission. Exhausted capacity
produces typed backpressure, rejection, or gap semantics; it never silently
drops or coalesces an accepted occurrence.

Agents does not own `SourceContract` registration or
`SourceReducerDescriptor`. Those are Server-owned pre-admission contracts;
their protocol-specific descriptor contents and deterministic implementations
belong to the relevant Adapter or Host SDK owner. Contracts only indexes and
generates the owner publications. An Agents transition reducer changes PXM
state after an exact semantic input; a source reducer classifies or reduces
stream input before occurrence acceptance. The two are never interchangeable.

Internal dependency release, local wake tokens, executor notifications,
telemetry observations, transport acknowledgements, and delivery attempts are
not Program Events. They use separate concrete types so none can fulfill an
`EventRef<T>` by accident.

### Readiness and placement are separate

Agents replaces the flat execution walk with an immutable admitted execution
plan and a per-activation `ReadinessKernel`.

For every dynamic node occurrence, the kernel owns typed operand slots, a
remaining-predecessor count, and one closed lifecycle in which cancellation or
failure may win from blocked, runnable, or running state:

```text
blocked -> runnable -> running -> completed
   |          |          |
   +----------+----------+-> cancelled
   +----------+----------+-> failed
```

The exact edge slot is claimed before its winner writes the operand; operand
publication happens-before the one predecessor decrement and readiness
publication. For a non-zero count, only the transition from one to zero may
attempt `blocked -> runnable`; zero-predecessor nodes use an explicit admission
transition. Cancellation racing the last predecessor is a legal closed race:
the winning lifecycle transition decides whether a queued item executes or is
discarded lazily. Duplicate, late, type-mismatched, or stale-claim publication
fails closed. Compiler-produced control, context, Hook, structured-concurrency,
and effect edges determine correctness; queue choice never does.

The `LocalExecutor` decides where a `RunnableNode` runs. V1 requires owner-local
work distribution, a bounded externally injected work path, locality-aware
victim selection, bounded spinning, and a lost-wakeup-safe park/unpark
protocol. The deque algorithm/library remains an acceptance decision: every
candidate is pinned to an exact source/checksum and reviewed for the Rust
memory model, weak-memory behavior, reclamation, cache-line layout, overload,
and model-test coverage before selection. The currently declared
`crossbeam-deque` 0.8.6 is not accepted merely because it is present; its source
explicitly acknowledges volatile concurrent access that is technically a data
race. Correctness does not depend on a hint, queue length estimate, victim
choice, or NUMA topology.

### Work stealing moves computation, never authority

A thief may move only an immutable local `RunnableNode` that is already
semantically runnable under the same leased activation. A steal never moves or
recreates:

- Invocation Admission or Program Instance single-flight authority;
- a durable activation lease or its fence epoch;
- a Capability Grant, approval, secret, or credential lease;
- an event binding or fulfillment right;
- an uncommitted external effect; or
- an Execution Commit right.

The stolen value retains the exact Program revision, activation id, dynamic
node-occurrence id, cancellation epoch, and commit fence. Non-stealable work is
kept out of stealable deques rather than marked with a best-effort hint.

### Server owns managed durability

Server owns external occurrence acceptance, durable delivery/target
application, activation records, managed effect work, eligibility times,
priorities, ordering lanes, leases, retry schedules, dead-letter/redrive state,
delivery/effect attempts, operational backpressure, reconciliation scheduling,
and crash recovery. It persists exact Agents-defined activation, EventRef, and
effect identities, but it does not decide graph readiness, construct local
NodeOccurrences, or choose a worker deque.

An `ActivationRunner` in Agents consumes one exact storage-neutral
`ActivationClaim`; the managed Composition Root adapts a Server lease while a
standalone implementation supplies an equivalent claim. The runner executes
its local runnable graph and returns a typed committed, parked, cancelled,
retryable-infrastructure, failed, commit-unknown, or claim-lost result. Durable
effect workers persist effect-outcome-unknown state; a later activation can
consume only a committed or reconciled effect result. Agents owns the effect
transition meaning and next-activation admissibility, Server owns managed
storage and reconciliation scheduling, and the Adapter owns provider/device
send, status, idempotency, cancellation, and reconciliation protocol. The
durability implementation marks completion only from a matching
claim, Program revision, Instance owner, EventRef revision where consumed, and
commit proof. Lease expiry never authorizes blind repetition of an external
effect.

Cross-process redistribution is lease expiry, revocation, or explicit transfer
under Server authority. It is not work stealing.

### External effects end the activation segment before dispatch

The live activation prepares a stable effect identity, exact request digest,
binding, authority decision, provider idempotency/reconciliation contract, and
continuation. A winning Execution Commit persists that prepared effect and
ends or parks the activation while publishing durable `Dispatchable` work.
Only the managed Server effect worker, or a conformant standalone durability
adapter, may claim it, record `DispatchStarted` before the first external byte,
invoke the exact Adapter, store the result or `OutcomeUnknown`, reconcile when
needed, and publish the exact next activation. A compute worker never holds an
activation or local queue slot through network/storage I/O, and an expired
local claim cannot authorize a direct send.

### Performance policy is explicit and testable

The local scheduler must provide:

- local LIFO execution for cache locality and opposite-end stealing when the
  selected sound deque supports it;
- batch steals where the chosen deque implementation supports them;
- cache-line separation for contended worker and wake counters;
- NUMA-local victims before remote-domain victims;
- randomized or permuted bounded victim selection without herd behavior;
- bounded spin, yield, and jittered park phases;
- no-lost-wakeup publication-before-notification ordering;
- admitted bounds on active activations and runnable node occurrences;
- priority service with aging so low-priority work has a starvation bound;
- cancellation and stale-claim checks before local dispatch and commit;
- an activity-credit/generation protocol that transfers activity to every
  successor before releasing the predecessor and linearizes local quiescence;
  and
- quiescence accounting that is explicitly non-authoritative for durable
  no-work truth.

Every optimization is removable without changing Program results. Performance
hints never affect correctness or authority.

### Physical AI remains outside the hard-real-time boundary

The Agents scheduler may process admitted sensor-derived occurrences and issue
fenced actuation Capabilities, but it is not a hard-real-time control loop.
Physical-source frequency never determines Program Event status: an exact
source may admit each sample, or classify it as a stream element kept behind a
physical-edge port until an admitted filter, window, threshold, or inference
produces a typed occurrence. Agents compute,
blocking-I/O, accelerator, Host-streaming, and device-controller real-time
resources remain isolated; work stealing never borrows controller cores,
priorities, callbacks, or reserved memory budgets. Emergency stop, collision
avoidance, servo control, and other bounded-deadline safety behavior remain
device-local. Physical provenance has mandatory clock/boot epoch, sequence,
uncertainty, frame, calibration, and freshness fields under its exact variant.

### The canonical runtime has one path

Agents exposes one typed EventRef/readiness/activation-runner/local-executor
architecture. No `legacy`, `compat`, `v0`, `try_old`, dual-read,
first-available, sequential production driver, or fallback scheduler is part of
the canonical runtime. Release-transition work belongs to the workspace
full-replacement plan rather than this target semantic decision.

## Consequences

- Agents gains an explicit execution-plan, readiness, activation-runner, and
  local-executor module family under `crates/runtime/`.
- `EventRef<T>` carries only Agents-owned portable semantic meaning.
- Server becomes the sole managed occurrence/delivery, activation-lease, and
  durable effect-work/reconciliation owner.
- Runtime evidence gains typed semantic readiness, NodeExecution, effect,
  activation-fence, and commit facts. Enqueue, victim choice, steal, NUMA move,
  park, and wake are always observer telemetry and cannot drive recovery.
- A complete benchmark and concurrency-verification suite is required before
  enabling parallel execution in production.

## Alternatives considered

### Put all event queues and durable scheduling in Agents

Rejected. It would put database, lease, retry, DLQ, ingress, and multi-tenant
operational policy into the portable abstract-machine library.

### Keep a separate durable scheduler plane

Rejected. Durable occurrence and activation coordination composes with Server's
root admission and Program lifecycle; a separate semantic plane creates
another ownership and transaction boundary.

### Use transport messages as Program Events

Rejected. Broker redelivery, HTTP acknowledgement, local wakeups, and Program
fulfillment have different identities and success conditions.

### Use Tokio task scheduling as the PXM scheduler

Rejected as a semantic model. An injected async executor may use Tokio, but
Tokio does not own AIR dependency readiness, NodeOccurrence identity,
continuations, effect fencing, or Execution Commit truth.

### Select a deque by familiarity alone

Rejected. A library or new implementation must first pass the exact source,
soundness, weak-memory, layout, overload, reclamation, model-test, and benchmark
gate. Familiarity and use by another runtime are insufficient evidence.
