---
status: accepted
date: 2026-09-01
owner: APXM agents
requires: ADR-0013, ADR-0016, ADR-0018, ADR-0022, ADR-0023
amends: ADR-0016, ADR-0022
---

# Host-fulfilled capabilities are requested by the runtime and settled by the host

## Status and authority

Accepted for the Agents repository. It amends ADR-0016 (tool authoring and
handler execution are separate) by adding a second execution carrier for a
non-builtin Capability, and ADR-0022 (capability references resolve against a
catalogue) by widening the minted set with a package-declared, manifest-closed
`host:` namespace. It changes no builtin path and removes no shipped-handler
path.

## Context

A non-builtin Capability could reach an implementation in exactly one way: a
handler shipped in the package snapshot, executed by a worker process that
`PackageHandlerWorker::spawn` refuses to start without an OS-level sandbox
backend. Nothing in the shipped `apxm-runtime-service` binary registers such a
backend, so the shipped runtime could not execute any external capability at
all. Building that backend is a large, platform-specific piece of work whose
outcome — running foreign code inside APXM — is not what an embedding control
plane wants. What such a host wants is the opposite: APXM asks, the host acts,
the host answers. That is the activity-executed-by-a-worker shape.

## Decision

A package may declare the host capabilities it needs. The runtime never
executes them; it requests them and parks.

1. **Declaration.** `agent.toml` gains `[[capabilities.host]]` with `id`,
   `effect` (`read` | `write`), `input_schema` and `output_schema` (inline JSON
   Schema objects). `contracts/schemas/apxm.host-capability.v1.json` is the
   published closed schema for a declaration set, the request the runtime
   emits, and the outcome the host returns.

2. **Reference and minting.** Authored source refers to a declared host
   capability as `Capability("host:<id>")`. The minted set is
   `builtins ∪ {"host:" + id for each declared id}`. Both frontends accept the
   `host:` arm; the compiler closes the set against the manifest and rejects an
   undeclared `host:` reference as a source diagnostic carrying the exact
   `line:column` of the reference in the submitted source.

3. **Request, not execution.** On `capability.invoke` of a `host:` reference the
   driver does not call the capability port. It mints a deterministic
   `capability_request_id` from the invocation and node-execution coordinates,
   emits a `capability_requested` observation carrying
   `{capability_request_id, capability_ref, effect, authored_permission,
   input}`, and awaits the answer through the same `EventPort` that
   `await.event` uses. Under `suspend_on_park` the invocation parks on a
   durable continuation exactly as an `await.event` node does.

4. **Settlement.** Runtime/1 gains `capability_fulfill{request_id, owner_claim,
   capability_request_id, outcome: ok|denied|failed|unknown, output,
   receipt_ref}` and `capability_cancel{request_id, owner_claim,
   capability_request_id}`. Both are authorized by the instance owner claim,
   the way `program_invocation_cancel` is. A fulfilment records a durable
   application and wakes the parked continuation through the existing
   `event_fulfill` wake authority; the node then settles as `Completed` for
   `ok`, as a typed failure for `denied`/`failed`, and as `OutcomeUnknown` for
   `unknown`. Every settlement emits a `capability_settled` observation.

5. **Cancellation.** `program_invocation_cancel` cancels every outstanding host
   capability request of that invocation, and `capability_cancel` cancels one.
   A cancelled request settles its node as a typed failure and emits
   `capability_settled` with outcome `cancelled`. A fulfilment that arrives for
   a cancelled request is refused, not applied.

6. **Permission.** Permission for a `host:` reference is the host's decision.
   APXM records the authored `Allow | Ask | Deny` in AIR
   (`capability_permission_requests`), carries it verbatim in the
   `capability_requested` observation, and does not broker it: the approval
   broker resolves `Ask` for builtin and shipped-handler references only.
   Builtins keep APXM's own broker unchanged.

## Deviations from the design as stated

- **`capability_fulfill` carries `request_id` and `owner_claim`.** Every
  mutating Runtime/1 method is correlated by `request_id` and authorized by an
  exact possession claim. A method without them would be the only unauthorized
  mutation on the wire.
- **Protocol/1's request union grows.** The union was documented as frozen.
  Adding a method is additive for a peer that never sends it, we own both
  sides, and the alternative — a fourth separately negotiated envelope — would
  make the host open a second channel to answer a request it received on the
  first. The doc comment is amended rather than worked around.
- **"Parks the node and continues other branches" parks the invocation.** The
  canonical driver is a single deterministic schedule walk (`build_schedule`);
  it has no concurrent branch scheduler, and `await.event` already parks the
  whole walk. A host capability parks the same way. Two host requests in one
  program are therefore issued and settled in schedule order, not concurrently.
  Making the walk concurrent is a separate decision about AIS semantics, not
  about this carrier.
- **`input` travels inline in the observation as canonical JSON text.** The
  observation contract carries payloads by reference elsewhere, but a host that
  must execute the request needs the arguments, and the canonical argument
  bytes are already bounded by the Capability request contract.

## Consequences

- The shipped runtime binary can serve a program that uses an external
  capability, with no sandbox backend and no foreign code inside APXM.
- A host that answers no request leaves the invocation parked; that is a
  durable, inspectable state, not a hang, and `capability_cancel` ends it.
- `apxm.execution-observation.v1` gains two observation kinds and one optional
  object. `apxm.execution-read.v1` refers to the observation schema rather than
  inlining it, so `EXECUTION_READ_SCHEMA_DIGEST` is unchanged.
- The `host:` namespace is reserved. A builtin id may never begin with `host:`,
  and a shipped handler directory named `host:...` is not representable.
