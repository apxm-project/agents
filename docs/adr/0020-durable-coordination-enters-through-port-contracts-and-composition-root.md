---
status: accepted
date: 2026-08-13
owner: APXM agents
requires: accepted workspace ADR-0027 and Agents ADR-0013, ADR-0016, ADR-0018, ADR-0019
amends: ADR-0018, ADR-0019
---

# Durable coordination enters through Port Contracts and the Composition Root, not a named product

## Status and authority

Accepted for the Agents repository. It is subordinate to accepted workspace
ADR-0027 and to Agents ADR-0013 (core semantics are closed and
implementations enter through exact Port Bindings). It amends Agents ADR-0018
and ADR-0019 by restating their managed-durability boundary; it does not
reopen either ADR's Agents-owned semantics, and it does not change the
deletion decided in ADR-0019.

## Context

ADR-0018's "Server owns managed durability" section and ADR-0019's removal
gate both name a specific downstream product, "Server", as the counterparty
that owns external occurrence acceptance, durable delivery, activation
leases, retry/dead-letter state, and reconciliation scheduling. ADR-0019
additionally makes that same product's existence a precondition for deleting
the schedule builtin: "Removal lands only after the Server-owned managed
schedule surface exists."

That product has been removed from scope. Recon across code, ADRs, and
`.agents/project.md` found no live "Server" counterparty anywhere in this
repository or its adjacent workspace documentation. Naming it in an accepted
ADR is no longer a description of a real boundary; it is dead product residue
of the kind `.agents/project.md §1` forbids Agents from depending on. Worse,
ADR-0019's precondition as written is now permanently unsatisfiable: it gates
a deletion on the existence of a surface owned by a product that will never
exist, which would block that lane's removal forever if read literally.

Both ADR-0018 and ADR-0019 carry `amends`/`requires` provenance chains to
ADRs 0003, 0004, 0007, 0008, 0011, 0013, 0016, and 0027. Editing their
Server-ownership language in place would break that chain's own convention:
an accepted ADR's decision text is amended by a later ADR, not silently
rewritten. This ADR is that later amendment.

Nothing here reopens ADR-0018's Agents-owned portable event, readiness,
activation-runner, or local-executor semantics, and nothing here reopens
ADR-0019's decision that Agents owns no durable timer, no durable Capability
store, and no background task inside a builtin. Both deletions and both
ownership boundaries stand exactly as decided. Only the name of, and the
entry point to, the durability counterparty changes.

## Decision

### The counterparty is whatever the Composition Root binds, not a named product

Wherever ADR-0018 says "Server owns managed durability" and wherever ADR-0019
says "the Server-owned managed schedule surface," read instead: managed
occurrence acceptance, durable delivery/target application, activation
records, managed effect work, leases, retry schedules, dead-letter/redrive
state, and reconciliation scheduling are owned by whatever implementation the
Composition Root binds behind the durable-event Port Contract
(`contracts/port-contracts/apxm.durable-event.port-contract.json`), under
an exact Runtime Profile, per ADR-0013. Agents names no product on the other
side of that Port Contract. It may be satisfied by a managed service, a
standalone embedding adapter, or any other implementation an accepted Runtime
Profile admits; Agents' semantics do not vary with which one is bound.

A program that must resume at a wall-clock time awaits a typed Program Event,
and the occurrence that satisfies it arrives through the durable-event Port
Contract — which survives this amendment unchanged — from whatever the
Composition Root binds. Agents neither stores the schedule nor decides when
it fires, exactly as ADR-0019 already decided; only the label on the far side
of that boundary changes, from a named product to the Port Contract's exact
counterparty.

### ADR-0019's removal gate is restated, not weakened

ADR-0019's precondition is restated from "the Server-owned managed schedule
surface exists" to: an accepted Runtime Profile binds an implementation of
the durable-event Port Contract capable of producing wall-clock-scheduled
occurrences. The deletion of
`crates/runtime/capability/src/builtins/schedule.rs`,
`crates/runtime/capability/src/builtins/store.rs`, their gate, and the
`sqlite`/`rusqlite` feature entries stays exactly as ADR-0019 decided; this
ADR changes only what must be true on the other side of the Port Contract for
that deletion lane to proceed, so the gate is satisfiable by any conformant
implementation rather than by one specific, now-nonexistent product.

## Consequences

- ADR-0018 and ADR-0019 remain the accepted decisions of record for what
  Agents owns; this ADR is the current authority for what the docs mean by
  "Server" in both, and future readers should resolve that name through this
  ADR rather than treating it as a still-live product reference.
- No Agents code, doc, or fixture may introduce a new Server- or Studio-named
  identifier as the counterparty for managed durability going forward; the
  durable-event Port Contract is the only sanctioned seam.
- The `A-RM` removal lane referenced by ADR-0019 is gated on Port Contract
  conformance, not on a specific product's existence, and is therefore
  satisfiable.

## Rejected alternatives

**Edit ADR-0018 and ADR-0019 in place to remove the product name.** Rejected.
Both are accepted decisions with recorded `amends`/`requires` provenance;
silently rewriting their decision text breaks the repository's own
ADR-amendment convention and erases the historical record of what was
decided when.

**Leave the product name in place as harmless documentation residue.**
Rejected. `.agents/project.md §1` forbids Agents from depending on a
downstream product's identifiers, and an accepted ADR is normative text, not
incidental commentary; leaving it also leaves ADR-0019's removal gate
permanently unsatisfiable.

**Drop the managed-durability owner entirely instead of naming the Port
Contract.** Rejected. ADR-0013 already requires that every implementation
enter through an exact Port Binding under a signed Runtime Profile; naming
that existing seam is the product-neutral restatement, not a new boundary.
