---
status: accepted
date: 2026-07-16
decision: D-001F
owner: APXM agents / apxm-core
amends: ADR-0007
---

# AIR has five public semantic operations

## Context

The current AIS catalogue at `agents@9e26a62adebb` contains 38 operations. It
mixes five different kinds of concern:

1. portable runtime effects;
2. frontend cognition conveniences;
3. compiler control-flow instructions;
4. artifact declarations and observability annotations; and
5. platform/process/storage mechanisms.

That mixture produces semantic overlap, incomplete Python/TypeScript parity,
runtime assumptions about agent behavior, and placement-specific AIR. A public
operation is justified only when its semantics must survive both frontends and
execute identically in every admitted runtime profile.

## Decision

The replacement AIR exposes exactly five public semantic operations:

| AIR operation | Meaning | Typed result |
| --- | --- | --- |
| `model.call` | Invoke an admitted model with an explicit request and options | Declared `ModelOutput<T>` value; operational evidence is separate |
| `capability.invoke` | Invoke one admitted Capability through the unified authority/effect chokepoint | Declared capability result `T` |
| `program.new` | Create an attached stateful Program Instance from a typed exact Program reference | `ProgramInstanceRef<I, O, C>` |
| `program.invoke` | Invoke an exact Program reference one-shot or a Program Instance statefully | Declared program output `T` |
| `await.event` | Suspend on a typed durable external event/approval/signal | Declared event value `T` |

No other public semantic operation is part of the target AIR.

`program.invoke` has a typed receiver union, not a string target:

- `ProgramRef<I, O, C>` means isolated one-shot invocation; and
- `ProgramInstanceRef<I, O, C>` means invocation against committed instance
  state.

The verifier and runtime retain that distinction. Frontend
`instance.invoke(input)` lowers to `program.invoke(instance_ref, input)`.

### Structural compiler IR

The compiler may use structural operations required to represent ordinary
program semantics:

- constants and typed values;
- functions and same-program calls;
- blocks, regions, and block arguments;
- branch, conditional branch, and switch;
- structured loop with loop-carried values plus `continue`, `break`, and
  region `yield`;
- structured parallel regions and typed joins;
- `try`, `throw`, and `catch`; and
- program `return` and resumable program `yield` terminators.

These are compiler/runtime IR, not a public agent operation API. Frontends use
normal Python/TypeScript control flow. The compiler owns lowering and the
single canonical printer. A user cannot bypass the frontend type system by
calling a generic raw-op builder.

Program `yield` ends one stateful Program Invocation occurrence, returns its
typed value, and creates the generic commit boundary for explicitly updated
Program Context plus compiler-owned continuation state. The next
`instance.invoke(input: I)` supplies the resume block argument. Program
`return` instead completes the instance. In one-shot invocation, the first
return/yield produces the result and the ephemeral continuation is discarded.
Neither terminator names a Turn. Region `yield` is a different typed terminator
used inside structured IR. `await.event` suspends inside the same invocation
and is not an output/commit terminator.

### Automatic runtime mechanics

The following remain runtime mechanics and evidence, never authored AIR
operations:

- admission, idempotency, scheduling, retry attempts, cancellation, and
  deadlines;
- checkpoint placement, storage backend, TTL, compaction, replay, and recovery;
- NodeExecution ids, region-occurrence ids, timestamps, cost, usage,
  provenance, source mapping, Session Output layout, and tracing;
- child placement, worker/process selection, transport, endpoint, filesystem,
  and current working directory; and
- result/outcome delivery to an external client.

The runtime automatically checkpoints generic state at admitted durable
boundaries. There is no public `checkpoint` operation because program-authored
storage policy would make execution profile-dependent. There is no public
Turn, outcome-commit, conversation, memory-tier, or autonomous-loop operation.

### FrontendGraph and artifact ownership

The full replacement introduces canonical `apxm.frontend-graph`,
`apxm.air`, and `apxm.executable-artifact`. They replace the
pre-canonical implementation contracts; those prototypes are not a supported
version line.

`FrontendGraph` records:

- typed Agent Program definitions, entrypoints, signatures, imports, and exact
  Program references;
- ordinary control-flow and data dependencies;
- the five public semantic operations;
- explicit context/state flow and loop-carried values;
- static Hook bindings for agent, node, model, and Capability scopes; and
- non-executable source/semantic annotations used by diagnostics and Studio.

The artifact contains digest-bound definitions, handler bindings, model and
Capability requirements, typed program references, source maps, compiler and
source digests, and compatibility metadata. It contains no grants,
credentials, server URLs, runtime paths, mutable context, dynamic registration
operations, or alternate behavior manifest.

## Disposition of every current operation

The current wire ids are retired permanently and are not reused for new
semantics.

| Current operation | Target disposition |
| --- | --- |
| `AGENT` | Delete executable op. Agent Program descriptors are artifact metadata; authenticated Agent Identity is supplied and verified by admission, never self-asserted by the artifact. |
| `QMEM` | Delete. Context/local state is explicit data; external durable knowledge is a Capability. |
| `UMEM` | Delete. Local updates are values; durable writes use an admitted Capability/effect. |
| `ASK` | Replace with `model.call`. |
| `THINK` | Frontend helper over `model.call` options and semantic-role metadata. |
| `REASON` | Frontend pattern over `model.call` plus an explicit context/state update. No implicit belief/goal mutation. |
| `PLAN` | Frontend/model pattern. It cannot mutate runtime graph topology. |
| `REFLECT` | Frontend/model pattern over explicitly supplied evidence. |
| `VERIFY` | Frontend/model pattern or a verifying Capability. |
| `INV_CAP` | Replace with `capability.invoke`, the only executable Capability chokepoint. |
| `EXC` | Delete privileged op. Code/process execution is an admitted sandbox Capability. |
| `PRINT` | Delete. Program output uses return/yield; diagnostics use an admitted sink Capability or observer. |
| `JUMP` | Internal structural branch. |
| `BRANCH_ON_VALUE` | Internal structural conditional branch. |
| `RETURN` | Internal typed program/function terminator. |
| `SWITCH` | Internal structured switch. |
| `FLOW_CALL` | Delete. Same-program reuse is an internal function call; cross-program composition is `program.invoke`. |
| `WORKFLOW_SPAWN` | Delete. A workflow is an Agent Program reference invoked through `program.invoke`; runtime paths/artifacts are forbidden inputs. |
| `MERGE` | Delete. Typed block arguments and structured joins merge values explicitly. |
| `FENCE` | Delete. Data/effect dependencies order work; the QMEM/UMEM memory barrier disappears. |
| `WAIT_ALL` | Internal structured parallel join/await; no agent semantic op. |
| `TRY_CATCH` | Internal structured error control flow. |
| `ERR` | Delete. Recovery is ordinary authored try/catch or a Capability. |
| `COMMUNICATE` | Delete. APXM program-to-program work uses `program.invoke`; provider/human messaging is a Capability. |
| `HANDOFF` | Delete. Invoke-and-return is ordinary composition; APXM defines no implicit session/context/authority transfer. |
| `UPDATE_GOAL` | Delete. Goals are explicit program state/context; an external goal service is a Capability. |
| `PAUSE` | Replace with `await.event` over a typed durable signal. No polling URL, notification, or storage attribute enters AIR. |
| `RESUME` | Delete. An authorized external command satisfies an awaited event; it is not a graph node. |
| `DELEGATE` | Frontend pattern over `program.invoke`; delete op. |
| `NOP` | Compiler-transient only and never serialized. |
| `IDENTITY` | Delete op. Identity/source/observability annotation is metadata. |
| `SPAWN_AGENT` | Replace Agent Program use with `program.new`; external process lifecycle belongs to an adapter, not AIR. |
| `REGISTER_CAPABILITY` | Delete op. Capability requirements and bindings are static artifact records admitted before execution. |
| `REGISTER_HOOK` | Delete op. Hook callbacks are static artifact bindings; compiler inserts callback calls around their scopes. |
| `AUTONOMOUS` | Delete. Autonomy and conversation are frontend-authored structured loops. |
| `CHECKPOINT` | Delete public op. Runtime inserts provider-neutral checkpoints automatically at durable boundaries. |
| `CONST_STR` | Internal generic constant. |
| `YIELD` | Retire the current wire id. Use typed structured-region or program-yield terminators with distinct new identities. |

## Context, Hooks, and model/tool flow

- Program Context is a typed local variable carried as ordinary values/region
  arguments. There are no `ContextDelta`, `ContextMerge`, `QMEM`, or `UMEM`
  public operations.
- Model and Capability results do not enter context automatically. Agent
  Program source or a Hook explicitly assigns the next `agent.context` value.
- A model may request a Tool, but the authored Conversational Agent loop
  decides whether and how to call the Capability and whether to call the model
  again. `model.call` never starts a hidden agent loop.
- Hooks are callback functions statically bound to source scopes. They are not
  dynamic registry operations or an alternative runtime engine.
- Skills are never injected into an invocation or Turn. Skill discovery and
  reads occur through admitted Capabilities; the program explicitly decides
  what returned data enters model context.

## Observability and Studio projection

Every execution of a static program node creates a distinct NodeExecution.
Runtime retry creates an attempt under the same NodeExecution; program-authored
retry or a loop revisit creates a new NodeExecution.

The artifact maps static nodes and structured loop regions to source and
semantic annotations. Runtime evidence records generic region occurrences.
Studio joins them to display “Turn N”, model request/output, Capability calls,
Hook execution, Program Context before/after, attempts, cost, output files,
and failure. AIR and runtime do not contain a Turn entity.

A model node may show only provider-returned reasoning summary/content that the
provider contract makes available. APXM never claims access to hidden chain of
thought.

## Admission and replacement

- Artifacts containing any retired operation fail admission under the target
  Compatibility Set.
- There is no old-to-new runtime translator, alias opcode, fallback builder,
  dual parser, mixed artifact, or feature-selected operation set.
- One-time offline tooling may extract fixtures, but it is not linked into the
  target compiler/runtime and is deleted or archived after cutover.
- The old operation ids are reserved as retired and can never acquire new
  meanings.
- Canonical v1 structural operations use new schema identities even when an IR
  concept is also called branch, return, switch, try/catch, constant, or yield;
  no pre-canonical wire id, builder, validator, or handler survives behind the shared
  English word.

## Alternatives considered

### Preserve cognition aliases as runtime operations

Rejected. `THINK`, `REASON`, `PLAN`, `REFLECT`, and `VERIFY` differ in author
intent and presentation, not in a portable runtime effect. Semantic-role
metadata preserves Studio meaning without multiplying handlers.

### Preserve separate flow/workflow/agent coordination operations

Rejected. They encode placement and implicit state transfer. `program.new` and
`program.invoke` provide one typed composition contract.

### Add public Turn, checkpoint, memory, and output-commit operations

Rejected. They would make the runtime assume a conversational lifecycle or
storage policy. Generic program invocation, yield, region occurrence, and
runtime-controlled persistence are sufficient.

### Keep a raw operation builder for advanced authors

Rejected. It would recreate cross-frontend drift and make retired operations
reachable. Rust owns the operation schema, verifier, lowering, and printer.

## Consequences

- `apxm-core` owns a much smaller stable operation constitution.
- Python and TypeScript expose equivalent high-level APIs instead of unequal
  per-op builders.
- Existing artifacts and first-party programs require a breaking, coordinated
  migration.
- Conformance must prove the five operations, structural control flow,
  stateful yield/resume, callbacks, and absence of all retired handlers.
- The reference implementation sequence is the
  [Agent Program composition and AIR contract](../agents/agent-program-composition-and-air-contract.md).
