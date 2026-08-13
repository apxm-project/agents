---
status: accepted
date: 2026-07-31
owner: APXM agents
requires: accepted workspace ADR-0027 and Agents ADR-0013, ADR-0016, ADR-0018
amends: ADR-0016
---

# Builtin Capabilities own no durable scheduling or process-global wake bridge

## Status and authority

Accepted for the Agents repository. It is subordinate to accepted workspace
ADR-0027 and consumes, without redefining, Agents ADR-0018 (Agents owns
portable Program Event, reference, reducer, activation, readiness, and local
scheduling semantics; Server owns managed durable coordination), Agents
ADR-0013 (core semantics are closed and implementations enter through exact
Port Bindings), and Agents ADR-0016 (Tool authoring and handler execution are
separate boundaries).

ADR-0018 decides what the Agents runtime owns. It does not address the builtin
Capability surface, and its single mention of Capabilities is the list of
authority a stolen work item never carries. ADR-0016 fixes how a Tool is
authored and how a handler is admitted, and says nothing about durable state or
background timers inside a builtin. The pinned revision therefore carries a
durable wall-clock scheduler and a process-global wake bridge that no accepted
decision owns. This ADR closes that gap and amends ADR-0016 by adding a ceiling
on what a builtin Capability may hold.

Acceptance is an ownership decision. It does not claim the removal has
happened, that Server's replacement schedule surface is implemented, or that
any module named below is complete. Measured against Agents revision
`1af8d4f68c2aef2ba6b653837f5546dd80f00265`, the surfaces this ADR retires are
still present.

## Context

At the pinned revision the following exist in the Agents checkout.

`crates/runtime/capability/src/builtins/schedule.rs` implements a builtin
Capability whose actions create, list, get, and cancel timers expressed as
`after_secs`, `at_ms`, or `every_secs`. It spawns a background firer task, and
on fire it invokes an `OnFire` hook and wakes a parked invocation through the
`CapabilityHost` bridge. A host may use the bridge to
that hook to enqueue prompt wakeups into an existing task queue, which places
one half of a durable coordination protocol inside an Agents builtin.

`crates/runtime/capability/src/builtins/store.rs` backs it with a rusqlite
database in WAL mode holding a `ScheduleRow` with a `kind` of `once`,
`recurring`, or `cron`, a `next_fire_ms` column, and a `status` of `armed`,
`fired`, or `cancelled`. That is a durable timer store with its own lifecycle
state machine. Both modules are gated by the `sqlite` feature in
`crates/runtime/capability/src/builtins/mod.rs`, and that feature is default-on
in `crates/runtime/capability/Cargo.toml`, so the ordinary build carries them.

`crates/runtime/capability-iface/src/host.rs` defines `CapabilityHost` as the
wake interface, documented as the seam a scheduler implements over a
process-global park and wake registry. `crates/runtime/capability-iface/src/lib.rs`
describes a three-module `apxm-runtime` crate providing `scheduler`,
`executor`, and `capability`. Neither the crate nor the registry is a workspace
member. `crates/runtime/capability-iface/src/events.rs` carries the residue as
an `emit_scheduler_decision` observer keyed on a `u64` node identifier, while
`crates/machine/program/src/air.rs` identifies a semantic operation node by
`String`.

Three consequences follow from that state. A timer is durable coordination, and
ADR-0018 gives durable coordination to Server. A process-global wake registry
is ambient authority reaching across invocations, which ADR-0013 forbids for
core semantics. And a wake path that bypasses the readiness kernel is a second
way to make a node runnable, which contradicts the single canonical runtime
path ADR-0018 fixes.

## Decision

### Agents owns no durable timer

No Agents crate holds a durable timer, a wall-clock firing schedule, or a
persisted schedule lifecycle. A Program that must resume at a wall-clock time
awaits a typed Program Event under ADR-0018. The occurrence that satisfies it
is produced by a managed Server schedule and enters Agents through the durable
event Port Contract, identically to every other managed occurrence. Agents
neither stores the schedule nor decides when it fires.

`crates/runtime/capability/src/builtins/schedule.rs`,
`crates/runtime/capability/src/builtins/store.rs`, their gate in
`crates/runtime/capability/src/builtins/mod.rs`, and the `sqlite` and
`rusqlite` feature entries in `crates/runtime/capability/Cargo.toml` are
deleted. They are not re-homed inside Agents behind a different feature, a
different crate, or a non-default build.

### A builtin Capability holds no durable store

A builtin Capability performs a bounded external effect and returns a typed
result. It owns no database, no persisted lifecycle state machine, and no state
that outlives the invocation that called it. Durable state that outlives an
invocation belongs to a Server-owned store reached through an exact Port
Binding. This is the ceiling this ADR adds to ADR-0016: ADR-0016 separates
authoring from handler execution, and this decision bounds what a handler is
permitted to hold.

### A builtin Capability spawns no background task

A builtin Capability runs only inside the call that invokes it. It spawns no
firer, poller, or daemon task that continues after the call returns. Work that
must continue after the call is an effect the runtime prepares and Server
dispatches under ADR-0018's rule that an external effect ends the activation
segment before dispatch.

### Wake is readiness, not a host bridge

Making a node runnable is exclusively the readiness kernel's decision under
ADR-0018. There is no process-global park and wake registry, and no Capability
reaches sideways into another invocation's execution state.
`crates/runtime/capability-iface/src/host.rs` is deleted with the schedule
builtin it served. The stale `apxm-runtime` scheduler rationale in
`crates/runtime/capability-iface/src/lib.rs` and the `emit_scheduler_decision`
observer in `crates/runtime/capability-iface/src/events.rs` are removed, and
scheduler observation is republished by the local executor against the
`String` node identity the AIR actually uses.

### Removal is proved, not asserted

The retired paths are added to the retired-file list that
`tools/tests/test_canonical_only_reachability.py` enforces, so the absence is a
gate rather than a claim. Removal lands only after the Server-owned managed
schedule surface exists, in the `A-RM` lane of the Agents owner lane plan.

## Consequences

- A Program that needs a delay or a recurring wakeup depends on Server. Agents
  offers no local substitute, and no offline or single-process mode restores
  one.
- `crates/runtime/capability/` loses its rusqlite dependency and its default
  `sqlite` feature. Any caller relying on the default feature set to obtain the
  schedule or store builtin breaks at compile time rather than silently
  degrading.
- `crates/runtime/capability-iface/` loses `host.rs`. Adapters and Host SDK
  work that reads that module for the wake contract reads the readiness and
  activation surfaces instead.
- The Agents build no longer contains a second path to make a node runnable, so
  the readiness kernel's model tests cover the whole space of transitions into
  `runnable`.
- This ADR decides ownership only. The `A-RM` lane owns the deletion, and the
  removal is not complete at the pinned revision.

## Rejected alternatives

**Keep the schedule builtin behind a non-default feature.** A feature flag that
can be turned on is a second semantic path that must be tested, specified, and
kept consistent with Server's schedule semantics. ADR-0018 fixes one canonical
runtime path; an opt-in durable scheduler is a fork of it.

**Move the schedule builtin into the runtime as a first-party scheduler.** That
keeps durable coordination inside Agents, which ADR-0018 gives to Server, and
reintroduces the wake path that bypasses readiness.

**Keep `CapabilityHost` as a generic wake seam without the schedule builtin.**
The seam's only justification is a Capability that must wake a parked
invocation from outside its own call. Removing the durable timer removes the
justification, and an unused ambient-authority interface invites a second
consumer.

**Keep `store.rs` as a general-purpose Capability store and delete only the
timer.** The objection is durable state inside a builtin, not the timer shape.
A general store inside a Capability is durable state Server does not see, back
up, replay, or bound, and it would drift from Server's evidence and recovery
guarantees.
