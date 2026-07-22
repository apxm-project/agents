---
status: superseded by ADR-0014
date: 2026-07-15
superseded_by: ADR-0014
---

# Gao specializes the Conversational Agent construct

This decision is preserved as historical rationale. ADR-0014 replaces its
package-level `ConversationalAgent` and named Gao target with repository
examples built only from generic Agent Program APIs.

Gao will be implemented as a named TypeScript specialization of APXM's
standard Conversational Agent frontend construct. Gao may compose prompts,
Skills Discovery Capabilities, admitted executable Capabilities, Agent Facade
Hooks, loop-body logic, `program.new`, `program.invoke`, and explicit context
updates, but it will not own a loop primitive, Hook system, context model,
model/tool loop, compiler path, session engine, or runtime type.

The stable agent id `gao` identifies an installed Agent Program. It does not
select privileged compiler lowering or runtime behavior. Gao must compile and
run through the same FrontendGraph v1, AIR v1, executable artifact v1,
Program Instance, Agent Facade, admission, and evidence contracts as an
equivalent user-authored Conversational Agent.

## Context

The current Gao package is already an APXM TypeScript Agent Program with a
manifest, prompts, Skills, Capabilities, permission policies, and lifecycle
callbacks
([manifest](../../examples/agents/gao/agent.toml),
[Hook handlers](../../examples/agents/gao/capabilities/handlers/hooks.ts)).
Its entry currently creates `GraphBuilder` directly and builds a re-arming
`autonomous` loop itself
([current entry](../../examples/agents/gao/capabilities/handlers/main.ts)).

That implementation predates the accepted first-class Conversational Agent
contract. Keeping it would make the most visible APXM agent a second authoring
pattern and would allow its loop, context, and callbacks to drift from the
Python and TypeScript abstraction intended for every agent author.

Studio currently treats Gao as an installed agent id, validates its
server-projected record, and delivers Turns through APXM OS ingress
([Studio chat dispatch](../../../studio/apxm-studio/crates/studio/src/chat.rs)).
That transport boundary is compatible with this decision: Studio supplies
input and an allowlisted structured snapshot, while Gao owns agent behavior.

## Composition contract

Gao will compose the standard Conversational Agent construct rather than
inherit runtime behavior or wrap a hidden service. Its specialization may
define:

- Gao identity and static definition metadata;
- workflow-authoring prompts and Skill Discovery Capabilities;
- admitted planning, validation, composition, and execution Capabilities;
- typed Agent, loop, Node, Model, and Capability Hook bindings over the Agent Facade;
- an authored loop body that reasons about and proposes APXM workflows;
- explicit `agent.context` updates and persistence Capabilities;
- typed workflow-draft outputs and ordinary lifecycle evidence.

Gao must not define:

- a Gao-only input loop primitive, `recv` mode, rearm rule, or session state machine;
- a Gao-only Agent Facade, callback result, event name, or dispatch branch;
- a mutable Gao context bag or prompt assembly path outside Program Context;
- a Gao-specific model/tool loop or Capability enforcement path;
- a compiler or runtime check keyed on `agent_id == "gao"`;
- a direct Studio-to-Gao execution channel that bypasses APXM admission;
- authority inferred from Studio context, Skills, prompts, or the Gao id.

Studio or another authorized product surface may supply allowlisted page, canvas,
selection, run-evidence, and capability-inventory data as typed input values.
Gao may propose a typed workflow draft. Product-side application,
publication, approval, and customer authority remain outside this Agent
Program and must use their owning contracts.

## Dependency

This decision depends on P6 of the
[Agent Program composition and AIR full-replacement plan](../agents/agent-program-composition-and-air-full-replacement-plan.md).
The public TypeScript Conversational Agent construct must exist and pass
cross-language, compiler, artifact, local-runtime, and server-runtime
conformance before target Gao can be admitted. The current direct `GraphBuilder` entry
may be inspected as read-only implementation evidence until that gate passes;
it is not part of the target API or a supported fallback.

## Considered options

### Keep Gao's direct `GraphBuilder.autonomous` loop

Rejected. It preserves current behavior with low short-term effort but makes
Gao a competing reference architecture and forces authors to learn semantics
that the standard Conversational Agent should own.

### Implement Gao as a Studio service or UI-side orchestrator

Rejected. Studio is a product surface. Moving agent behavior into it would
duplicate APXM execution, make mobile and embedded clients diverge, and break
the rule that the same Agent Program runs independently of its host UI.

### Add a Gao runtime type or special AIS operation

Rejected. Gao is domain behavior, not a new abstract-machine primitive. A
special runtime type would bypass the reusable frontend abstraction and create
an agent-id-dependent execution path.

### Use the standard Conversational Agent by composition

Accepted. It makes Gao a demanding reference fixture for the public API while
keeping its workflow-authoring Skills, Capabilities, prompts, and Hooks
replaceable and inspectable as ordinary Agent Program components.

## Consequences

- Gao becomes the release-blocking TypeScript reference specialization for the
  Conversational Agent contract.
- The TypeScript frontend must be ergonomic enough to express Gao without
  reaching for a lower-level loop builder.
- Gao-specific context injection migrates to explicit `agent.context` updates
  and typed Model Context projections.
- Gao Hooks migrate to the portable Agent Facade; current `HookContext`, broad
  decision union, and unversioned callback types are removed at the common
  migration gate.
- Studio and other authorized clients remain transport and product surfaces;
  they do not own Gao cognition or runtime state.
- No current code is declared conformant by this ADR. Delivery is governed by
  the [Agent Program composition and AIR full-replacement plan](../agents/agent-program-composition-and-air-full-replacement-plan.md).
