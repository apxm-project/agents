---
status: accepted
date: 2026-07-22
owner: APXM agents
supersedes: ADR-0002
amends: ADR-0010
---

# Conversational Agent and Gao are examples over generic Agent Program APIs

## Context

ADR-0002 made Gao a named specialization of an APXM-provided
`ConversationalAgent` frontend construct. ADR-0010 correctly assigned loop,
Hook, Context, model/Capability sequencing, and composition behavior to source,
but still treated `ConversationalAgent`, conversational source-map annotations,
and Studio `Turn` projection as canonical product concepts.

That boundary gives one example shape significance across packages, compiler,
evidence, Server, and Studio. It also leaves structural control flow outside
the closed AIS-owned operation model even though artifacts serialize and
runtimes execute it.

## Decision

APXM installable Python and TypeScript frontends expose only generic
Agent Program authoring APIs: `AgentProgram`, Hook, Context, typed composition,
structured control flow, source maps, and explicit compiler bridges.

`ConversationalAgent` and Gao are repository examples, not APXM package APIs.
A conversational example may define an example-local `ConversationalAgent`
class or helper using only public generic APIs. The Gao example may import and
specialize that example-local construct. Neither name may appear as a core
frontend export, compiler or runtime branch, contract discriminant, Server
route, Studio product feature, or admission identity with privileged meaning.

### AIS owns two closed operation families

AIS owns two separate closed families:

1. the five effect/composition operations fixed by ADR-0009:
   `model.call`, `capability.invoke`, `program.new`, `program.invoke`, and
   `await.event`; and
2. structural operations required to encode authored control flow, including
   functions, regions, branches, loops, task scopes, try/catch, yield, return,
   and their typed value/block forms.

`ais.loop` is a first-class compiler-emitted structural AIS operation. It is
not a sixth effect/composition operation and is not directly authored through
a raw operation API. Both families are AIS-owned, generated, exhaustive for an
exact contract digest, and fail closed on unknown kinds. Python and TypeScript
author ordinary structured source; Rust alone verifies and lowers it.

### Loop evidence is generic

Source maps identify structural loop regions without a
`conversational_loop` annotation.

When one loop body and its back-edge commit atomically, runtime emits one
generic `LoopIterationCompleted` fact in `apxm.runtime-evidence.v1`. The fact
identifies the static loop, dynamic loop occurrence, zero-based iteration
index, and causal Program Invocation and NodeExecution identities. A failed,
cancelled, or rolled-back body emits no completion fact.

The completion fact is part of the same authoritative execution commit as the
state/continuation and evidence it describes. Replay reproduces the same fact
identity and ordering; telemetry or a UI projection cannot manufacture it.

Studio projects generic loop iterations. Example documentation may call a
conversational example's completed iteration a “turn,” but `Turn` is not a
core contract type, runtime entity, Server API, or Studio product model.

## Consequences

- ADR-0002 is superseded and retained only as historical rationale.
- ADR-0010 remains authoritative for source-owned behavior, explicit Context,
  Agent Facade Hooks, discovery-only Skills, and generic loops; its
  `ConversationalAgent`, Gao, conversational source-map, and `Turn` provisions
  are replaced by this decision.
- The five ADR-0009 operations remain the closed effect/composition family.
  Structural operations form a second closed AIS-owned family.
- Owner schemas and descriptors must replace `conversational_loop` and
  region-only Turn projection with generic loop-region identity and
  `LoopIterationCompleted`, then regenerate C0b before consumers change.
- Frontend packages must remove `ConversationalAgent`, Gao, `TurnSpec`,
  `SpecialistComposition`, and equivalent subpath exports outright. No alias,
  compatibility re-export, legacy reader, translator, or fallback is allowed.
- Conversational Agent and Gao move under repository examples with explicit
  build, compile, and run manifests and may use only installed generic APIs.
- Compiler, runtime, Server, and Studio conformance consumes artifacts built
  from those examples without branching on their names.

## Considered options

### Keep `ConversationalAgent` as a standard-library frontend export

Rejected. It gives one application pattern canonical package ownership and
encourages downstream contracts and products to depend on its name.

### Keep structural control flow outside AIS

Rejected. Serialized executable structure needs the same closed ownership,
generation, validation, and unknown-kind behavior as effect/composition
operations.

### Infer completed iterations from region starts or traces

Rejected. Inference cannot distinguish a committed back-edge from a failed or
rolled-back body and cannot provide replay-stable authoritative evidence.

### Preserve old exports and Turn contracts as compatibility aliases

Rejected. Canonical v1 is a full replacement and admits no mixed semantic
generation.
