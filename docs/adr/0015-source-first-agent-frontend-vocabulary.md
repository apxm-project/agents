---
status: accepted
date: 2026-07-23
owner: APXM agents
supersedes: none
amends: ADR-0006, ADR-0010, ADR-0014
---

# Source-first Agent frontend vocabulary and representation stack

The Capability-reference position and the frozen declaration matrix below have
been restated by
[ADR-0022](0022-capability-references-resolve-against-a-catalogue-and-permissions-are-declared-requests.md):
a Capability reference resolves against a generated catalogue rather than being
an opaque exact string, `contracts/vectors/apxm.frontend-surface.json` rather
than §4's table is where the cross-language surface is now decided, and §4's
two bundled-handler rows are one row. Read §4 and §7 of this ADR through that
amendment.

## Context

ADR-0006 fixed `apxm.frontend-graph` as the versioned interchange contract
and made Python/TypeScript pure authoring libraries over explicit compiler
bridges. ADR-0010 and ADR-0014 assigned loop, Hook, Context, and composition
behavior to source, made `ConversationalAgent`/Gao examples, and split AIS into
two closed families.

Those decisions still leave the public authoring surface as an imperative
recorder. At the baseline, authors construct `AgentProgram`, hand-record
node/region identities, import raw operation constants (`OP_MODEL_CALL`,
`SEMANTIC_*`, `STRUCTURAL_*`, literal `"ais.loop"`), and the frontend copies
those records field-for-field into `apxm.frontend-graph`, whose `operands`
are an untyped object bag and whose structural `kind` enum admits the raw AIR
spelling `ais.loop`. Rust then copies FrontendGraph to AIR field-for-field, and
the MLIR emitter writes unregistered `apxm.*` operations as `() -> ()` under an
`allowUnregisteredDialects(true)` escape hatch.

The recorder surface is baseline evidence, not the target author experience. It
exposes compiler bookkeeping, lets source choose AIS spellings, and prevents the
compiler from proving that a program was captured statically rather than by
running one path.

This decision freezes the target public vocabulary, the representation stack,
and the ownership boundary between source-typed intent and Rust-owned AIS
selection, so the phased implementation in the source-first Agent frontend
master plan can proceed. It adds no operation and no runtime behavior.

## Decision

### 1. The everyday public surface is five concepts

Installable Python and TypeScript frontends expose one source-first authoring
surface. An everyday author needs exactly five concepts:

```text
Agent     Context     Tool     Model     ordinary language control flow
```

| Public name | Author meaning | Contract/lowering identity |
| --- | --- | --- |
| `Agent` | Declares one typed Agent Program; returns a definition with `.new(...)` and `.invoke(...)` | `AgentProgram<I,O,C>`, `ProgramRef`, `program.new`, `program.invoke` |
| `agent` | Inferred callback parameter exposing `.context`, `.yield_(...)`, and callsite-valid views | The `AgentFacade<I,O,C>` contract; no public `AgentFacade` import |
| `Context` | Declares the typed initial and persistent Program Context schema | Explicit `C`, context value flow, loop-carried state, commit at yield/return |
| `Tool` | Declares a model-callable action reference or decorates a bundled typed handler | Tool schema over one Capability definition; lowers only to `capability.invoke` |
| `Model` | Declares one exact typed model target; calling it records one Model invocation | `ModelTargetRef` requirement; Rust selects `model.call` |

The callback parameter is named `agent` and inferred. No ordinary example
imports `AgentFacade`.

### 2. The advanced surface is four additional names

| Public name | Use |
| --- | --- |
| `Capability` | A typed executable action that is not a model-callable Tool |
| `Event` | A typed durable event reference whose `.wait(...)` records `await.event` |
| `Hook` | Static before/after binding when definition-scoped markers are insufficient |
| `TaskGroup` | Language-neutral structured concurrency with mandatory join and no detach |

Grants, credentials, Runtime Profiles, endpoints, graph node ids, AIR text, and
deployment bindings never appear in author source.

### 3. Names that are not public authoring concepts

- `AgentProgram` is the semantic contract and internal definition type, not the
  beginner constructor.
- `AgentFacade` is the precise callback-view contract, not a required import.
- `FrontendGraph`, node ids, region ids, context edges, and source spans are
  generated compiler inputs and inspection results, not authored values.
- `model.call`, `capability.invoke`, `program.new`, `program.invoke`,
  `await.event`, `ais.loop`, and every other `ais.*` spelling are lowering
  identities, not raw frontend builders. No authoring package exports them.
- `AgentInstance` is not a second lifecycle. `Agent.new(...)` returns the
  existing typed `ProgramInstanceRef`.

The full-replacement release removes the old public builder exports outright. It
keeps no `AgentProgram`, `AgentFacade`, recorder, or raw operation constant as a
compatibility alias. Compiler conformance may use a private recorder fixture.

### 4. Cross-language declaration matrix is frozen

Python and TypeScript expose the same declarations and semantics using the most
readable native form. Consistency means semantic and type equivalence, not
identical punctuation. Neither language may add a public construct, hidden
default, or lowering behavior the other cannot express equivalently.

| Concept | Python projection | TypeScript projection | Bound semantic node |
| --- | --- | --- | --- |
| Agent definition | `@Agent(...)` on one `async def` | `Agent<I, O, C>({...})` | `AgentDecl` |
| Context schema/default | `@Context` on one typed class | `Context<C>(initial)` | `ContextDecl` |
| Exact Model binding | `Model[I, O](ref)` | `Model<I, O>(ref)` | `ModelBinding` |
| Imported Tool binding | `Tool[I, O](capability_ref)` | `Tool<I, O>(capabilityRef)` | `ToolBinding` |
| Bundled Tool handler | `@Tool` on one typed `async def` | `Tool<I, O>({ async run(...) })` | `ToolDecl` + handler metadata |
| Imported Capability | `Capability[I, O](ref)` | `Capability<I, O>(ref)` | `CapabilityBinding` |
| Bundled Capability handler | `@Capability` on one typed `async def` | `Capability<I, O>({ async run(...) })` | `CapabilityDecl` + handler metadata |
| Static Hook | `@Hook.before(...)` / `@Hook.after(...)` | `Hook.before({...})` / `Hook.after({...})` | `HookDecl` |
| Durable Event value | `Event[T]`, `await event.wait(...)` | `Event<T>`, `await event.wait(...)` | `EventType` / `EventWait` |
| Structured task scope | `async with TaskGroup()` | `TaskGroup.run(...)` | `TaskScope` with mandatory join |

These are compile-time markers with closed behavior: the frontend resolves them
by imported symbol identity and type, never by coincidental local spelling; they
declare schemas/bindings/handlers/Hook relationships and never execute the
decorated body to discover the graph; their arguments are statically resolvable
types, exact references, closed options, or literals, never credentials, grants,
endpoints, runtime objects, or computed values; compiled declarations are
immutable after binding; and diagnostics point to the offending marker,
declaration, argument, or callsite. `Model` is a binding factory (it binds an
exact external target and has no local body). `Event` is primarily a typed value
returned by an admitted effect. `TaskGroup` is a lexical structured-control
scope.

### 5. The representation stack is frozen

The frontend is a compiler pipeline, not a decorator recorder. Each
representation has one abstraction level and one owner:

```text
native Python/TypeScript AST + symbols/types   (language-owned, parser evidence)
        -> bind, type/effect check
BoundAgentTree                                  (frontend-internal, immutable, typed, lexical, source-mapped)
        -> one deterministic traversal
apxm.frontend-graph                          (only cross-language serialized IR; typed source intents)
        -> Rust verification, CFG/SSA, AIS selection
AIR + registered ais.* / MLIR                   (Rust-owned closed semantics)
        -> verified, digest-bound
APXM artifact                                   (immutable release boundary)
```

`BoundAgentTree` is the frozen name for the frontend-internal high-level
representation. Its invariants are required:

- it retains lexical constructs, resolved APXM declarations, inferred types,
  helper-function bodies, source spans, and diagnostics;
- it replaces surface sugar with a small closed set of semantic nodes and
  introduces no AIR/AIS operation names;
- it is immutable after construction, so analysis and lowering cannot depend on
  decorator execution order or mutable recorder state;
- it is frontend-internal and ephemeral: never serialized, versioned as a public
  contract, sent to Server, or accepted as executable input; and
- Python and TypeScript may use language-native host types but implement the
  same semantic-node taxonomy and pass the same conformance vectors.

Stable node and region identities derive from canonical module/export identity
plus normalized lexical preorder — never author strings, mutable recorder
counters, or hash-map iteration order. Identities are assigned during BoundTree
construction and preserved through traversal.

### 6. FrontendGraph owns typed source intents; Rust alone owns AIS selection

`apxm.frontend-graph` is replaced (not extended with a second IR) by a typed
shape containing: typed Agent/Context/Model/Tool/Capability/imported-Agent/Event/
Hook declarations and bindings; typed functions, parameters, values, blocks,
regions, results, data edges, and explicit context/state edges; discriminated
semantic **intents** for Model invocation, Tool/Capability invocation, Agent
creation/invocation, and Event wait; language-neutral conditional, loop,
task-scope, try/catch, yield, and return intents; and exact requirements,
digests, annotations, and spans.

FrontendGraph contains no arbitrary operation strings and no `ais.*` kind. A
Tool invocation and an advanced Capability invocation are distinct typed intents
in FrontendGraph; Rust converges both to `capability.invoke` while retaining
enough typed metadata to validate the Tool schema and requirement.

Rust owns FrontendGraph verification, CFG/SSA construction, selection of the
five semantic operations, construction of structural AIS, Hook-region expansion,
AIR verification/serialization, registered AIS MLIR emission, and artifact
binding. The registered MLIR names are `ais.model_call`, `ais.capability_invoke`,
`ais.program_new`, `ais.program_invoke`, `ais.await_event`; the serialized AIR
spellings remain `model.call`, `capability.invoke`, `program.new`,
`program.invoke`, `await.event`. Registered `ais.*` operations carry complete
typed operands/results, attributes, nested regions, and block arguments, and
canonical verification succeeds with unregistered dialects disabled.

### 7. One machine-readable surface manifest

D0 freezes one machine-readable frontend-surface manifest
(`contracts/schemas/apxm.frontend-surface.json`) enumerating every public
declaration/marker, its accepted arguments, inferred types, bound semantic node,
and the diagnostics it can raise. Public-export scans, documentation-alignment
checks, and generated reference docs derive from this one manifest. A guide,
README, or generated doc that imports a name absent from the manifest fails the
alignment check.

### 8. Source-to-evidence correlation is preserved

The frozen correlation identity joins source span, static graph node, static
loop region, dynamic Program Invocation, loop occurrence, NodeExecution,
attempt, effect/request identity, admitted model binding, Context transition,
usage, output, and failure. Legal optimization must preserve types, effects,
authority, ordering, durability, source lineage, and observable results, and
must emit inspectable optimization provenance. The compiler may emit
backend-neutral analysis/scheduling metadata; an admitted adapter may translate
supported metadata into backend hints only while preserving the same request,
result, cancellation, usage,
failure, and evidence semantics, and only while remaining visible in backend
evidence. Providers are inference implementations behind exact model ports, not
Agent runtimes or frontend concepts.

## Consequences

- The source-first Agent frontend master plan becomes implementation authority
  for phases P1–P6 under this ADR and the amended parent contract.
- The public-spelling portions of ADR-0010 and ADR-0014 (imperative
  `AgentProgram` recorder as the author surface) are superseded by the
  five-concept source-first surface; their semantic boundaries are unchanged.
  ADR-0006's canonical-interchange and explicit-bridge decision is retained; the
  interchange shape is replaced under §6, not the bridge boundary.
- The parent Agent Program composition and AIR contract is amended so §3 Source
  API shows the source-first surface, §7 FrontendGraph lists typed intents
  rather than raw operation records, and §8/§11 fix the AIR-to-registered-AIS
  signatures with complete typed operands/results.
- `CONTEXT.md` is amended to distinguish friendly frontend names (`Agent`,
  `Context`, `Tool`, `Model`, `Capability`, `Event`, `Hook`, `TaskGroup`) from
  contract types (`AgentProgram`, `AgentFacade`, `FrontendGraph`, AIR ops).
- The FrontendGraph and AIR JSON schemas, their Rust types, the AIS operation
  signatures in `crates/machine/ais/src/operations/definitions.rs`, the
  generated TableGen, the canonical MLIR emitter, and the native bridges are all
  in scope for replacement under P1/P2. No new operation is added.
- This is a breaking full replacement. There is no dual constructor, alias
  period, old graph reader, or compatibility translator.

## Considered options

### Extend the recorder surface with typed helpers

Rejected. It keeps compiler bookkeeping (node/region ids, raw operation
constants) in the author surface and cannot prove static capture.

### Add a second serialized IR between FrontendGraph and AIR

Rejected. Two serialized levels create competing authorities. FrontendGraph is
replaced in one compatibility-set cutover, not paired with a translator.

### Let FrontendGraph carry `ais.*` kinds and typed operands

Rejected. That makes source choose AIS spellings and duplicates AIS selection in
the frontend. FrontendGraph carries typed source intent; Rust alone selects AIS.

### Keep `allowUnregisteredDialects(true)` for `apxm.*` output

Rejected. Canonical lowering must verify with unregistered dialects disabled so
no success depends on the escape hatch.
