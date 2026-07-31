# Source-first Agent frontend master plan

- Status: frontend P1–P6 implemented in `agents@09dbdf63` under
  [ADR-0015](../adr/0015-source-first-agent-frontend-vocabulary.md); this plan
  preserves the accepted delivery shape and current authoring syntax while
  the ADR-0027 managed-plane/full-removal joins remain target work
- Owner: APXM `agents`
- Scope: Python, TypeScript, FrontendGraph, Rust lowering, examples, generated
  Studio source, documentation, and frontend-to-evidence conformance
- Baseline: `agents@8ccf4040bd341d074954489bdd61112f3bee294b`
- Parent contract:
  [Agent Program composition and AIR](agent-program-composition-and-air-contract.md)
- Parent delivery plan:
  [Agent Program composition and AIR full replacement](agent-program-composition-and-air-full-replacement-plan.md)
- Frontend boundary:
  [ADR-0006](../adr/0006-authoring-frontends-use-explicit-compiler-bridges.md)
- Event/runtime boundary:
  [ADR-0018](../adr/0018-event-readiness-and-local-scheduling-are-agents-semantics.md)
  under accepted workspace ADR-0027

Accepted ADRs and the parent contract remain architectural authority. This
master plan records the delivered syntax-alignment shape for the source-first
frontend. Guides and READMEs may teach it, but they do not independently invent
public names, decorator behavior, lowering rules, or examples.

This plan owns authoring, FrontendGraph, Rust lowering, and source-to-evidence
conformance only. Agents owns portable Event/EventRef/occurrence/provenance,
dependency readiness, activation-runner, and Execution Commit semantics.
Server owns managed Source Contracts, accepted occurrences, delivery/target
application, activation/effect durability and leases, schedules, Host gateway,
retry/DLQ/recovery, and operational projections. Auth, Host SDK, and Adapters
retain their authority and protocol ownership. Contracts indexes and generates
the exact owner publications without owning their meaning. Target consumers
use those generated publications and Server-managed durability; the retiring
OS is only current-state removal evidence, publishes no target descriptor, and
has no target frontend or execution consumer.

## 1. Goal

Make an Agent Program look like an agent in ordinary Python and TypeScript.
Everyday authors should need five concepts:

```text
Agent     Context     Tool     Model     ordinary language control flow
```

The frontend parses and binds that source into an immutable typed source tree,
then deterministically traverses the tree into the single
`apxm.frontend-graph.v1` contract family. Phase D0/P1 replaces the current
shallow v1 shape in one compatibility-set cutover; it does not introduce a
second serialized frontend IR. Rust remains the canonical FrontendGraph
verifier, CFG/SSA constructor, AIS selector, and AIR lowerer. The simple names
are authoring projections over the precise contract types; they do not create
a frontend runtime or new AIS operations.

The imperative recorder baseline is historical evidence only. The current
packages and repository examples use the source-first `Agent`, `Context`,
`Tool`, and `Model` surface without authored node, region, or source-span
identities.

### 1.1 The frontend is the coding face of APXM

For a developer, the frontend is the product entry point. It must make the
power of the complete APXM execution spine available without exposing its
internal machinery:

- **expressive source** — ordinary typed functions, branches, loops,
  try/catch, Context, Models, Tools, Capabilities, Events, Hooks, composition,
  yield/return, and structured concurrency;
- **static understanding** — the frontend resolves every supported construct
  into a complete typed graph rather than discovering behavior by running one
  path;
- **safe optimization** — Rust may analyze and optimize the compiled program
  only when types, effects, authority, ordering, durability, source lineage,
  and observable outcomes are preserved;
- **portable execution** — source names exact portable requirements, while
  admission binds them to exact implementations without putting endpoints,
  credentials, placement, or provider branches in the Agent;
- **full observability** — source spans, static graph nodes, dynamic
  NodeExecutions, attempts, loop occurrences, Hooks, effects, context
  transitions, usage, outputs, and failures remain joinable; and
- **one semantic path** — Python, TypeScript, generated Studio source, local
  compilation, remote compilation, Agents generic runtime execution,
  Server-managed durability, and Studio inspection agree on the same program.

“Powerful” does not mean a large API. It means a small orthogonal vocabulary
can express rich behavior while the compiler and runtime retain enough typed
structure to optimize, govern, execute, and explain it.

### 1.2 A conversational Agent is an example, not a primitive

A conversational Agent is an ordinary `Agent` whose source usually contains:

```text
typed input
  -> authored loop
  -> explicit Model and optional Tool/Capability calls
  -> explicit Context replacement
  -> yield reply and accept the next typed input
```

Events can park an iteration for approval or external input. Hooks can observe
or transform declared scopes. Agent composition can call specialists. A
`TaskGroup` can run independent attached work and join it. None of those
features is conversation-specific, so the example needs no package-level
`ConversationalAgent`, hidden host loop, runtime mode, `Turn` contract, or
compiler branch.

The conversational repository examples and the Studio-owned Gao program are
therefore high-value conformance programs: if the generic frontend can express
them clearly in both languages, lower them equivalently, optimize them safely,
execute them through exact ports, and reconstruct them from evidence, the
frontend is doing its job. Agents does not own Gao product source or semantics.

### 1.3 One connected execution, including vLLM

The source-facing `Model` API remains provider-neutral. When an admitted model
deployment uses APXM-vLLM, the connected path is:

```text
Model(request) in Agent source
  -> typed Model-invocation intent in FrontendGraph
  -> Rust-selected model.call in AIR / ais.model_call in registered AIS
  -> artifact ModelTargetRef requirement
  -> exact admission-time target/deployment/port binding
  -> runtime NodeExecution + stable model-effect identity
  -> admitted ModelInferencePort adapter
  -> APXM-vLLM request/stream and native usage
  -> authoritative Execution Commit
  -> runtime evidence + non-authoritative backend telemetry
  -> Studio/source-level inspection
```

vLLM is an inference implementation behind the exact model port, not an Agent
runtime and not a frontend concept. The compiler may emit backend-neutral
analysis and scheduling metadata; an admitted adapter may translate supported
metadata into vLLM graph/prefix/priority hints. That translation must preserve
the same model request, result, cancellation, usage, failure, and evidence
semantics and must remain visible in implementation/backend evidence.

The accepted one-spine boundary is fixed by
[ADR-0011](../adr/0011-agent-program-execution-is-one-end-to-end-spine.md).
Current typed inference identity and exact-binding behavior is anchored in
[`apxm-inference`](../../crates/runtime/inference/src/lib.rs); the first-party
vLLM adapter mapping is owned by
[`adapters/crates/runtime/src/vllm.rs`](../../../adapters/crates/runtime/src/vllm.rs),
and the composition proof is owned by the adapter conformance suite at
[`adapters/crates/conformance/tests/spine_assembly.rs`](../../../adapters/crates/conformance/tests/spine_assembly.rs).
Those implementation pieces do not close the frontend/AIS gaps in section 9;
the target requires the whole path to share one correlation and conformance
contract.

## 2. Public vocabulary decision

### 2.1 Everyday surface

| Public name | Author meaning | Contract/lowering meaning |
| --- | --- | --- |
| `Agent` | Declares one typed Agent Program and returns a definition with `.new(...)` and `.invoke(...)` | `AgentProgram<I,O,C>`, `ProgramRef`, `program.new`, `program.invoke` |
| `agent` | Inferred body/Hook parameter exposing typed `.context`, `.yield_(...)`, and the views valid at that callsite | The current `AgentFacade<I,O,C>` contract; no public `AgentFacade` import |
| `Context` | Declares the typed initial and persistent Program Context schema | Explicit `C`, context value flow, loop-carried state, commit at yield/return |
| `Tool` | Declares a typed model-callable action reference or decorates a bundled typed handler | Tool schema over one Capability definition; calls lower only to `capability.invoke` |
| `Model` | Declares one exact typed model target; calling it records one typed Model invocation | `ModelTargetRef` requirement; Rust selects `model.call` |

The callback parameter is simply named `agent` and inferred. Documentation,
autocomplete, and generated declarations should not require a new author to
learn or import `AgentFacade`. The precise facade contract may remain an
internal/generated type as long as its authority ceiling and fields remain
unchanged.

### 2.2 Focused advanced surface

Advanced programs may additionally import:

| Public name | Use |
| --- | --- |
| `Capability` | A typed executable action that is not a model-callable Tool |
| `Event` | A typed durable event reference whose `.wait(...)` records `await.event` |
| `Hook` | Static before/after binding when definition-scoped decorators/methods are insufficient |
| `TaskGroup` | Language-neutral structured concurrency with mandatory join and no detach |

These names must stay out of the minimal examples until their behavior is
actually needed. Grants, credentials, Runtime Profiles, endpoints, graph nodes,
AIR, and deployment bindings never appear in author source.

### 2.3 Names that are not public authoring concepts

- `AgentProgram` is the semantic contract and internal definition type, not the
  beginner constructor.
- `AgentFacade` is the precise callback-view contract, not an import required
  in ordinary source.
- `FrontendGraph`, node ids, region ids, context edges, and source spans are
  generated compiler inputs and inspection results.
- `AIR`, `model.call`, `capability.invoke`, `program.new`, `program.invoke`,
  `await.event`, and `ais.loop` are lowering identities, not raw frontend
  builders.
- `AgentInstance` is not a second lifecycle. `Agent.new(...)` returns the
  existing typed `ProgramInstanceRef`, inferred for ordinary authors.

The full-replacement release removes the old public builder exports. It does
not keep `AgentProgram` or `AgentFacade` as compatibility aliases beside the
new surface. Compiler conformance tests may use a private recorder fixture.

### 2.4 Language projections and decorator matrix

Python and TypeScript expose the same declarations and semantics using the
most readable native form. Consistency means semantic and type equivalence,
not forcing identical punctuation onto languages with different declaration
systems.

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
| Durable Event value | `Event[T]`, then `await event.wait(...)` | `Event<T>`, then `await event.wait(...)` | `EventType` / `EventWait` |
| Structured task scope | `async with TaskGroup()` | `TaskGroup.run(...)` | `TaskScope` with mandatory join |

The exact `Capability` and `TaskGroup` spellings remain proposed until D0, as
does the final internal semantic-node naming. The matrix freezes the required
one-to-one roles: neither language may add a public construct, hidden default,
or lowering behavior that the other language cannot express equivalently.

TypeScript does not imitate Python by forcing class/member decorators onto
top-level Agent functions. Its typed declaration factories are the equivalent
declarative surface. If standard TypeScript eventually supports an equally
clear standalone-function decorator form, changing the spelling still requires
an owner decision and cross-language golden update.

### 2.5 Declarative marker rules

Python decorators and TypeScript declaration factories are compile-time
markers with closed behavior:

- the frontend resolves them by imported symbol identity and type, not by a
  coincidental local spelling such as a function named `Agent`;
- they declare schemas, bindings, handlers, or static Hook relationships; they
  do not execute the decorated function/class to discover the graph;
- arguments are statically resolvable types, exact references, closed options,
  or compile-time literals—never credentials, grants, endpoints, runtime
  objects, or arbitrary computed values;
- compiled declarations are immutable after binding and cannot be monkey
  patched or mutated through a registry;
- arbitrary third-party decorators on compiled APXM declarations fail unless
  an owner-approved composition rule proves their order and semantics;
- stacked APXM markers have an explicit canonical order, while Hook order is a
  declared contract field rather than Python decorator execution order;
- a local Tool/Capability marker emits handler-bundle metadata separately from
  the Agent graph and grants no execution authority; and
- diagnostics point to the marker, declaration, argument, or callsite that
  violated the closed frontend subset.

`Model` is intentionally a binding factory rather than a decorator because it
binds an exact external Model target and contains no local handler body.
`Event` is primarily a typed value returned by an admitted effect. `TaskGroup`
is a lexical structured-control scope. Giving every concept decorator syntax
would obscure these different roles rather than simplify them.

## 3. Frontend awareness and operation decision

The frontend must understand the authoring semantics. It binds `Agent`,
`Context`, `Tool`, `Model`, `Capability`, `Event`, and ordinary control flow by
symbol and type, rejects unsupported ambiguity, and records typed intent. It
must not choose an AIS wire spelling or accept one from source.

| Source | FrontendGraph intent | Rust-selected AIR operation |
| --- | --- | --- |
| `await SupportModel(request)` | typed Model invocation | `model.call` |
| `await SearchWeb(arguments)` | typed Tool invocation over one Capability | `capability.invoke` |
| `await AdvancedAction(arguments)` | typed Capability invocation | `capability.invoke` |
| `Specialist.new(context=...)` | typed Agent-instance creation | `program.new` |
| `await Specialist.invoke(input)` or `await instance.invoke(input)` | typed Agent invocation with definition/instance receiver | `program.invoke` |
| `await approval.wait()` | typed Event wait | `await.event` |

The frontend should not export `AgentLoop` or `Loop` as the normal loop API.
Python `while`/`for` and TypeScript `while`/`for` are already the simplest
source representation. Static capture records a language-neutral loop intent,
and Rust constructs structural `ais.loop`. `agent.yield_(output)` remains the
explicit stateful invocation boundary fixed by the parent contract; it is
structural syntax, not a sixth effect operation.

If stable author labels are needed for Hooks or evidence navigation, a label is
an optional source annotation attached to an ordinary loop. It must not require
a callback builder or let source choose a region/node id.

## 4. Python golden syntax

### 4.1 Minimal one-shot Agent

```python
from apxm_program import Agent, Model

SummarizerModel = Model[SummaryRequest, Summary]("model.summary.v1")


@Agent(input=SummaryRequest, output=Summary)
async def Summarizer(agent, request):
    return await SummarizerModel(request)
```

`agent` is inferred and may be unused. Importing `AgentFacade`, constructing a
graph, calling `.lower()`, or naming a node is unnecessary.

### 4.2 Contextual conversational Agent with a Tool

```python
from apxm_program import Agent, Context, Model, Tool


@Context
class Conversation:
    messages: tuple[Message, ...] = ()


SearchWeb = Tool[SearchRequest, SearchResult]("capability.search-web.v1")
SupportModel = Model[ModelRequest, ModelResponse]("model.support.v1")


@Agent(
    input=ConversationInput,
    output=ConversationOutput,
    context=Conversation,
)
async def Support(agent, incoming):
    while True:
        research = None
        if incoming.search_query is not None:
            research = await SearchWeb(SearchRequest(incoming.search_query))

        response = await SupportModel(
            ModelRequest(
                history=agent.context.messages,
                incoming=incoming.message,
                research=research,
            )
        )

        agent.context = Conversation(
            messages=(
                *agent.context.messages,
                incoming.message,
                response.message,
            )
        )
        incoming = await agent.yield_(ConversationOutput(response.message))
```

### 4.3 Bundled Tool handler

```python
from apxm_program import Tool


@Tool
async def NormalizeAddress(request: AddressInput) -> NormalizedAddress:
    return normalize_address(request)
```

The frontend recognizes the decorator and binds a typed Tool/Capability
declaration plus handler metadata into the semantic tree for the separate
handler bundler. Calling `NormalizeAddress(...)` inside an Agent becomes typed
Tool-invocation intent when that tree emits FrontendGraph; Rust selects
`capability.invoke`. The Python function is not executed by the authoring
frontend and the decorator grants no authority.

### 4.4 Agent composition

```python
specialist = SecurityReviewer.new(
    context=ReviewerContext(repository=request.repository),
)
review = await specialist.invoke(ReviewRequest(diff=request.diff))
summary = await Summarizer.invoke(SummaryRequest(review=review))
```

The inferred type of `specialist` is the existing
`ProgramInstanceRef<ReviewRequest, Review, ReviewerContext>`.

## 5. TypeScript golden syntax

### 5.1 Minimal one-shot Agent

```typescript
import { Agent, Model } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

const source = staticSource(import.meta.url);
const SummarizerModel = Model<SummaryRequest, Summary>("model.summary.v1");

export const Summarizer = Agent<SummaryRequest, Summary>({
  name: "Summarizer",
  source,
  use: { SummarizerModel },
  async run(agent, request) {
    return await SummarizerModel(request);
  },
});
```

The callback parameter is inferred; no `AgentFacade` type import is required.

### 5.2 Contextual conversational Agent with a Tool

```typescript
import { Agent, Context, Model, Tool } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type Conversation = {
  messages: readonly Message[];
};

const ConversationContext = Context<Conversation>({ messages: [] });
const SearchWeb = Tool<SearchRequest, SearchResult>("capability.search-web.v1");
const SupportModel = Model<ModelRequest, ModelResponse>("model.support.v1");
const source = staticSource(import.meta.url);

export const Support = Agent<
  ConversationInput,
  ConversationOutput,
  Conversation
>({
  name: "Support",
  source,
  context: ConversationContext,
  use: { SearchWeb, SupportModel },
  async run(agent, incoming) {
    for (;;) {
      const research = incoming.searchQuery === undefined
        ? undefined
        : await SearchWeb({ query: incoming.searchQuery });

      const response = await SupportModel({
        history: agent.context.messages,
        incoming: incoming.message,
        research,
      });

      agent.context = {
        messages: [...agent.context.messages, incoming.message, response.message],
      };
      incoming = await agent.yield_({ message: response.message });
    }
  },
});
```

### 5.3 Bundled Tool handler

```typescript
import { Tool } from "@apxm/frontend";

export const NormalizeAddress = Tool<AddressInput, NormalizedAddress>({
  async run(request) {
    return normalizeAddress(request);
  },
});
```

The declaration is statically bound into the semantic tree and marks the
handler for the separate bundler that produces the digest-bound sidecar. It is
not run while the frontend captures source; invoking the Tool emits typed
FrontendGraph intent from which Rust selects `capability.invoke`.

### 5.4 Agent composition

```typescript
const specialist = SecurityReviewer.new({
  context: { repository: request.repository },
});
const review = await specialist.invoke({ diff: request.diff });
const summary = await Summarizer.invoke({ review });
```

## 6. Tool, Capability, and authority boundary

`Tool` is an ergonomic frontend projection, not a replacement for the
Capability contract:

```text
Tool source declaration
  -> typed Tool schema shown to a model when source chooses
  -> Capability requirement in FrontendGraph/artifact
  -> capability.invoke at each callsite
  -> Auth-owned grant and approval at admission/runtime
  -> exact bound implementation/handler
```

An arbitrary `Tool` cannot carry a credential, grant, endpoint, shell command,
provider choice, or implementation selector. A Tool request returned by a
model remains typed data. Source explicitly chooses a closed Tool variant,
invokes it, and decides whether to call the model again. Neither the model
adapter nor runtime owns a hidden Tool loop.

Non-model-callable actions keep the advanced `Capability` name so APXM does not
mislabel every effect as a Tool. External Agent sessions, durable memory, and
other focused wrappers may hide that advanced type behind their own typed API,
but they still lower to `capability.invoke`.

`Model(...)` accepts a typed digest-pinned `ModelTargetRef`, and an imported
`Tool(...)` accepts a typed Capability-definition reference. A local `@Tool`
declaration derives its content identity from its exported source identity and
typed schema. Neither form accepts a mutable display name such as `"default"`,
`"support"`, or `"search-web"`.

## 7. Static source capture

### 7.1 Python

The `Agent`, `Context`, and `Tool` decorators are recognized declaration syntax;
compilation does not execute them as recorders. The frontend reads the Python
AST, resolves only the closed supported constructs, uses Python type
information plus generated APXM schemas, builds the bound typed source tree,
and traverses it into FrontendGraph. It does not execute the Agent body or Tool
handler to discover behavior.

### 7.2 TypeScript

The TypeScript frontend uses the TypeScript compiler API/transform pipeline to
read the `Agent`, `Context`, `Tool`, and `Model` declarations, typed callback
body, and ordinary structured control flow. It builds the equivalent bound
typed source tree and traverses it into the same FrontendGraph contract without
bundling the Rust compiler into the browser surface.

### 7.3 Shared rules

- Source location plus lexical structure produces stable internal node and
  region identities; authors never name them.
- Parser spans produce source maps; source never scans its own file.
- Assignments to `agent.context` produce explicit typed context flow.
- Calls to typed `Model`, `Tool`, `Capability`, Agent definitions/instances,
  and `Event` values produce closed typed frontend intents. Rust alone maps
  those intents to the five semantic AIS operations.
- Ordinary branches, loops, try/catch, task groups, yield, and return produce
  language-neutral control-flow intents. Rust alone constructs the closed
  structural AIS family.
- Unsupported dynamic reflection, monkey patching, computed call targets,
  unbounded dynamic imports, raw coroutine/Promise detachment, and ambiguous
  types fail with source diagnostics.
- No serialized frontend AST or alternate graph contract is introduced.

Runtime tracing and sample-value symbolic execution are rejected as primary
capture strategies because they observe paths rather than the complete authored
program and would turn frontend capture into execution.

### 7.4 Representation stack

The frontend is a compiler pipeline, not a decorator recorder. Each
representation has one abstraction level and one owner:

```text
Python source                     TypeScript source
      |                                  |
      v                                  v
Python AST + symbols/types        TypeScript AST + TypeChecker
      |                                  |
      +---------- bind and validate -----+
                         |
                         v
         frontend-internal BoundAgentTree
       (immutable, typed, lexical, source-mapped)
                         |
              deterministic traversal
                         v
          FrontendGraph versioned contract
      (language-neutral structured program graph)
                         |
                 Rust verification
                         v
       canonical CFG/SSA + regions/block arguments
                         |
                   AIR selection
                         v
          registered AIS/MLIR + legal passes
                         |
                         v
           verified immutable APXM artifact
```

| Representation | Lifetime / boundary | Purpose | Must not contain |
| --- | --- | --- | --- |
| Native Python/TypeScript AST | Per compilation; language-owned | Exact syntax, symbols, host-language types, parser spans | APXM lowering decisions |
| `BoundAgentTree` | Per compilation; frontend-internal | Resolved APXM semantics in immutable lexical form | Raw AIR/AIS names, runtime values, mutable recorder state |
| FrontendGraph | Versioned serialized compiler input | One language-neutral structured program and source map | Host AST nodes, arbitrary operation strings, provider/runtime decisions |
| AIR | Versioned canonical compiler contract | Closed semantic operations plus structural program/value flow | Frontend sugar, hidden effects, backend implementation choices |
| Registered AIS/MLIR | Compiler-internal verified IR | Typed regions/SSA and semantics-preserving analysis/optimization | Unregistered operations, missing operands, frontend-only concepts |
| APXM artifact | Immutable digest-bound release boundary | Admitted executable program, requirements, source/evidence correlation | Mutable source state or unresolved authority |

The design choice is therefore: **use the host AST as parser evidence, use a
bound typed tree as the frontend's semantic representation, and use
FrontendGraph as the only cross-language IR**. Serializing raw Python or
TypeScript AST would couple Rust to language syntax and make parity impossible.
Lowering each host AST directly to AIR would duplicate type, control-flow, and
effect semantics in two compilers. Making FrontendGraph itself the mutable
authoring API would expose compiler bookkeeping to users and lose the complete
lexical source model needed for diagnostics.

`BoundAgentTree` is a proposed implementation name for the frontend-internal
high-level representation. D0 may choose another internal name, but the phase
and its invariants are required. It is analogous to a compiler's bound/typed
AST, not to LLVM IR:

- it retains lexical constructs, resolved APXM declarations, inferred types,
  helper-function bodies, source spans, and diagnostics;
- it replaces surface sugar with a small closed set of semantic nodes without
  introducing AIR/AIS operation names;
- it is immutable after construction, so analysis and lowering cannot depend
  on decorator execution order or hidden mutable recorder state;
- it is frontend-internal and ephemeral: it is not serialized, versioned as a
  public contract, sent to Server, or accepted as executable input; and
- Python and TypeScript may use language-native host types, but both implement
  the same semantic node taxonomy and must pass the same conformance vectors.

FrontendGraph is the first serialized, language-neutral representation. It is
closer to a high-level compiler IR: structured control flow is explicit,
declarations and values have stable identities, and source semantics no longer
depend on Python or TypeScript syntax. It deliberately stops before canonical
SSA and AIS selection so the Rust compiler remains the one semantic authority.

AIR is the closed APXM semantic and structural program representation.
Registered AIS/MLIR is the compiler representation on which verified
canonicalization, analysis, and optimization operate. The executable artifact
is the object-format boundary. Collapsing these levels into one recorder loses
information; duplicating any serialized level creates competing authorities.

### 7.5 Traversal and pass discipline

The frontend pipeline uses explicit passes with verification between them:

1. **Parse** with the host parser; retain every source location.
2. **Bind** imported APXM symbols, declarations, lexical names, call targets,
   and helper functions.
3. **Type/effect check** Agent inputs/outputs/Context and every Model, Tool,
   Capability, Event, Hook, composition, yield, and return site.
4. **Normalize** only specified source sugar into closed `BoundAgentTree`
   nodes. Normalization preserves evaluation order and never executes user
   code.
5. **Analyze** capture, mutation, reachability, structured-task ownership, and
   unsupported dynamic behavior; diagnostics refer to original source spans.
6. **Emit FrontendGraph** with one deterministic tree visitor/folder. Internal
   builders are write-only implementation details, never public author APIs.
7. **Verify round-trip invariants**: every reachable semantic tree node has a
   graph representation and source-map entry, and every emitted graph value or
   edge has one typed origin.

Passes consume one immutable representation and produce either an analysis
result or a new immutable representation. They cannot mutate a global graph,
invoke a decorator body, inspect runtime values, or smuggle arbitrary JSON
attributes across the boundary. Stable identities derive from canonical
module/export identity and lexical structure after specified normalization.

This is the useful LLVM lesson for APXM: deliberate representation levels,
typed operations, verifiers at boundaries, explicit pass contracts, stable
source locations, and one canonical lowerer. The goal is not to expose an IR
to everyday authors or to copy LLVM's instruction set.

## 8. Representation and ownership contract

### 8.1 Source frontend semantics

Python and TypeScript own parsing, binding, type checking, effect checking,
`BoundAgentTree` construction/traversal, source diagnostics, and source
mapping. They recognize the public concepts and produce a complete typed graph
of every supported path. They do not expose raw operation constants, accept
authored node/region ids, print AIR/MLIR, or decide that a source loop is
literally named `ais.loop`.

The frontend may consume generated contract types and reference metadata. That
does not make it an AIS emitter: its output describes what the source means,
not which registered dialect operation implements it.

### 8.2 FrontendGraph

FrontendGraph is the only language-neutral handoff. Its target shape contains:

- typed Agent, Context, Model, Tool, Capability, imported-Agent, Event, and Hook
  declarations or bindings;
- typed functions, parameters, values, blocks, regions, results, data edges,
  control edges, and explicit context/state edges;
- discriminated semantic intents for Model invocation, Tool/Capability
  invocation, Agent creation/invocation, and Event wait;
- language-neutral conditional, loop, task-scope, try/catch, yield, and return
  intents;
- exact requirements, source/handler digests, semantic annotations, and source
  spans.

It contains neither arbitrary operation strings nor an `ais.*` kind. This is a
replacement shape for `apxm.frontend-graph.v1`, not a serialized Python AST and
not another IR between FrontendGraph and AIR. The exact discriminated unions
and field names are frozen by D0/P1 before frontend implementation.

### 8.3 Rust compiler and AIS

Rust owns FrontendGraph verification, CFG/SSA construction, selection of the
five semantic operations, construction of structural AIS, Hook-region
expansion, AIR verification/serialization, registered AIS MLIR emission, and
artifact binding. Python, TypeScript, Studio, and remote clients cannot replace
any of those steps.

The registered MLIR operation names are `ais.model_call`,
`ais.capability_invoke`, `ais.program_new`, `ais.program_invoke`, and
`ais.await_event`; the serialized AIR spellings remain `model.call`,
`capability.invoke`, `program.new`, `program.invoke`, and `await.event`. That
distinction is compiler-owned and never leaks into normal author source.

An Event declaration or wait carries only the Agents-owned portable Event,
EventRef, provenance, and PXM transition meaning through source, FrontendGraph,
AIR, artifact, continuation, and evidence. The frontend does not define a
Source Contract, accept an occurrence, apply a fulfillment, lease an
activation, schedule a retry, or persist delivery/effect work. Managed
end-to-end conformance supplies those facts through Server's generated target
contracts and adapts one exact Server claim to the Agents `ActivationRunner`.

## 9. Baseline findings and required corrections

At `agents@8ccf4040bd341d074954489bdd61112f3bee294b`, the repository proves
closure and byte determinism for a much thinner path than the target contract:

| Baseline finding | Evidence | Required correction |
| --- | --- | --- |
| Current frontends record directly into mutable graph structures; there is no explicit bound/typed source-tree phase with pass invariants. | Historical baseline paths `crates/compiler/frontend/python/apxm_program/agent_program.py` and `crates/compiler/frontend/typescript/src/agent-program.ts`; both were subsequently replaced | Introduce a frontend-internal immutable `BoundAgentTree`, closed visitors/passes, and coverage tests before emitting FrontendGraph. |
| FrontendGraph reuses AIR-owned `SemanticOpKind` and `StructuralOpKind`. | [`frontend_graph.rs`](../../crates/machine/program/src/frontend_graph.rs) | Give FrontendGraph its own typed source-semantic declarations, values, calls, CFG, and region records. |
| The schema lets frontends write the five AIR strings and literal `ais.loop`; `operands` is an untyped object. | [`apxm.frontend-graph.v1.json`](../../contracts/schemas/apxm.frontend-graph.v1.json) | Replace raw spellings with closed discriminated intent records and typed value references; Rust selects AIS. |
| Python and TypeScript publicly import/export generated operation constants and record raw operation/kind strings. | [Python frontend](../../crates/compiler/frontend/python/apxm_program/__init__.py) plus historical baseline TypeScript paths `crates/compiler/frontend/typescript/src/frontend-graph.ts` and `src/generated/frontend-contract.ts`, subsequently replaced | Generated frontend metadata describes declarations and intent DTOs; raw AIS/AIR names are absent from public authoring packages. |
| FrontendGraph → AIR copies operations, structural records, operands, and context edges almost field-for-field. | [`lower.rs`](../../crates/machine/program/src/lower.rs) | Implement verification, CFG construction, SSA/block arguments, loop-carried context, yield/resume, Hook expansion, and deterministic AIS selection. |
| FrontendGraph and AIR schemas omit the typed values/data edges/nested control flow required by the owner contract. | [FrontendGraph schema](../../contracts/schemas/apxm.frontend-graph.v1.json), [AIR schema](../../contracts/schemas/apxm.air.v1.json), [owner contract §7](agent-program-composition-and-air-contract.md#7-frontendgraph-v1) | Complete both schemas and their Rust types/vectors before either source frontend lands. |
| The AIR emitter writes unregistered `apxm.*` semantic operations as `() -> ()`, drops semantic operands/results, and writes flat structural token records. | [`canonical.rs`](../../crates/compiler/pipeline/src/canonical.rs) | Emit registered `ais.*` operations with complete typed SSA operands/results, attributes, nested regions, and block arguments. |
| The registered TableGen semantic operations currently declare only one token result and no operands. | [`AISOps.semantic.generated.td`](../../crates/compiler/pipeline/mlir/include/ais/Dialect/AIS/IR/AISOps.semantic.generated.td) | Amend the Rust-owned AIS signature definitions, regenerate TableGen, and verify each contract operand/result. |
| MLIR verification permits unregistered dialects, so the current `apxm.*` output can pass. | [`Internal.h`](../../crates/compiler/pipeline/mlir/include/ais/CAPI/Internal.h) | Canonical lowering must verify with unregistered dialects disabled; no success may depend on this escape hatch. |
| Native Python/Node bridges lower to AIR JSON or an artifact containing AIR; they do not exercise the registered AIS emitter. | [Python bridge](../../crates/compiler/frontend/native/python/src/lib.rs), [TypeScript bridge](../../crates/compiler/frontend/native/typescript/src/lib.rs), [`artifact.rs`](../../crates/machine/program/src/artifact.rs) | One compiler service boundary must expose graph, AIR, registered-AIS verification, diagnostics, and artifact results from the same lowering transaction. |

These are current-state findings, not permission to add a translator or retain
both graph shapes. D0 chooses the replacement contract and its compatibility-set
cutover.

## 10. Deterministic lowering algorithm

For a given source bundle and pinned contract set, the compiler performs these
steps in order:

1. The host parser produces its native AST and symbol/type information while
   retaining exact source locations.
2. The language frontend binds only recognized APXM symbols and constructs the
   immutable typed `BoundAgentTree`; it rejects dynamic call targets or
   unsupported language behavior before graph emission.
3. A deterministic visitor traverses the verified tree and emits declarations,
   typed values, lexical regions, control/data/context edges, call intents,
   Hook bindings, digests, and spans into FrontendGraph. Stable ids derive from
   canonical module/export identity plus normalized lexical preorder, never
   user strings, mutable recorder counters, or hash-map iteration.
4. Rust decodes and validates the graph closed-world: unique definitions,
   dominance, type equality, receiver kind, exact requirements, region
   containment, structured concurrency, Hook legality, and source-map coverage.
5. Rust builds the typed CFG. It converts joins to block arguments, assignments
   to SSA values, Context changes to explicit state edges, and loops to
   region-carried values with deterministic argument order.
6. Rust maps typed call intent to exactly one of the five AIR operations. A Tool
   and an advanced Capability both select `capability.invoke`, but retain enough
   typed metadata to validate the Tool schema and requirement.
7. Rust lowers conditionals, loops, task scopes, try/catch, return, and yield to
   structural AIS. Yield has an output/context commit edge and a distinct
   continuation block whose argument is the next invocation input; return has
   no resume successor.
8. Rust expands static Hooks around the selected target region in declaration
   order, without adding a dynamic registry or another effect operation.
9. Rust verifies canonical AIR, emits registered AIS operations with all typed
   operands/results and nested regions, verifies with unregistered dialects
   disabled, then canonically serializes AIR/source maps and digest-binds the
   artifact.

Canonical ordering rules are contract fields, not implementation accidents:
declarations use canonical source identity, blocks use dominance preorder,
operations use lexical order inside a block, block arguments use stable value
order, Hooks use declared scope/phase/order, and requirements use exact
reference identity. Python and TypeScript goldens must therefore converge after
canonicalization even when their ASTs differ.

## 11. Complete source-to-AIS mapping

| Author construct | FrontendGraph target intent | AIR | Registered AIS MLIR |
| --- | --- | --- | --- |
| `@Agent` / `Agent({...})` | typed Agent definition/function | no effect operation | function/region/block structure |
| `@Context` / `Context<T>` | context type, initializer, explicit value flow | structural values/block arguments | `ais.value` plus region/block arguments |
| `Model(ref)` | exact typed Model binding and requirement | no operation until invoked | none until invoked |
| `await model(request)` | Model invocation with request/options/result values | `model.call` | `ais.model_call` |
| `Tool(ref)` or `@Tool` | typed Tool/Capability binding, schema, requirement, optional handler digest | no operation until invoked | none until invoked |
| `await tool(arguments)` | Tool invocation with arguments/result values | `capability.invoke` | `ais.capability_invoke` |
| advanced Capability call | Capability invocation with arguments/result values | `capability.invoke` | `ais.capability_invoke` |
| `Agent.new(context=...)` | Agent creation with exact ref/initial-context/result values | `program.new` | `ais.program_new` |
| definition or instance `.invoke(input)` | Agent invocation with typed receiver/input/result values | `program.invoke` | `ais.program_invoke` |
| `Event<T>.wait(...)` | Event wait with ref/timeout/cancellation/result values | `await.event` | `ais.await_event` |
| `if` / `match` / `switch` | conditional CFG and typed joins | structural branch/switch | `ais.branch` / `ais.switch` |
| `while` / `for` | loop condition/body/exit and carried values | structural `ais.loop` | `ais.loop` |
| Context assignment | new typed context value and state edge | structural value/block argument | `ais.value` plus SSA use |
| `agent.yield_(output)` | output/context commit and resume-input successor | structural yield | `ais.yield` with continuation edge |
| `return output` | typed completion terminator | structural return | `ais.return` |
| `TaskGroup` | owned child regions and mandatory join | structural parallel join | `ais.parallel_join` |
| try/throw/catch | exceptional CFG and typed joins | structural try/throw/catch | `ais.try` / `ais.throw` / `ais.catch` |
| Hook declaration | static target binding and ordered wrapper region | compiler-generated structural regions around the target | registered structural AIS plus the target operation |

The exact TableGen operand representation—SSA operand versus required typed
attribute—is frozen with the AIR/AIS signature contract in D0/P1. It must carry
every semantic input listed in owner contract §8 and cannot reduce those inputs
to an opaque JSON blob or omit them from emitted AIS.

## 12. Delivery phases

### D0 — Owner decision and contract amendment

Affected authority:

- add an owner ADR fixing the short public vocabulary and superseding the
  public-spelling portions of ADR-0014/ADR-0010 without changing their semantic
  boundaries;
- amend `CONTEXT.md` to distinguish friendly frontend names from contract
  types;
- amend the Agent Program contract type model, FrontendGraph/AIR shapes,
  source examples, canonical ordering, and AIR-to-registered-AIS signatures;
- freeze the frontend representation stack, `BoundAgentTree` invariants,
  traversal/pass contract, and the point at which stable identities are
  assigned;
- freeze the cross-language declaration/decorator matrix in section 2.4 and
  one machine-readable frontend-surface manifest from which public export and
  documentation checks can be generated;
- decide that FrontendGraph owns typed source intents while Rust alone owns the
  mapping to AIS operations, including Tool/Capability convergence;
- freeze the source-to-evidence correlation identity, optimization legality
  rules, and backend-neutral compiler-metadata boundary;
- update the parent P2/P6/P9 plan gates and the event-driven runtime
  full-replacement joins; and
- freeze the Python/TypeScript examples in sections 4 and 5 as reviewed golden
  source.

Gate: every public name has one role, `Tool`/Capability authority is explicit,
all representation boundaries are frozen, and the decision adds no operation
or runtime behavior.

### P1 — Contract shapes and cross-language vectors

- replace the shallow FrontendGraph/AIR schemas and Rust types with the typed
  declarations, values, blocks, regions, edges, call intents, and operation
  signatures in sections 8-11;
- define positive vectors for minimal Agent, Context, Model, Tool reference,
  Tool handler, Hook, loop, yield/resume, Event, one-shot composition,
  stateful composition, task group, and typed Tool-request dispatch;
- define negative vectors for dynamic targets, missing schemas, mutable global
  context, detached work, raw graph/AIR use, credentials/grants in source, and
  unsupported reflection;
- pin canonical FrontendGraph, AIR, source-map, handler-sidecar, diagnostic,
  optimization-record, and evidence-correlation outputs for Python and
  TypeScript;
- define semantic-tree coverage vectors proving both frontends bind and
  normalize equivalent source constructs before graph emission;
- define public-surface vectors for every declaration/decorator, accepted
  argument, inferred type, diagnostic, and forbidden composition in sections
  2.4-2.5;
- publish Agents-owned schema sources and owner-local generator inputs so the
  Contracts index/generation cohort produces every direct consumer from the
  same exact owner digests without copying semantics; and
- include one conversational conformance vector that combines a loop, Context,
  Model, typed Tool request, Capability result, Event wait, Hook, specialist
  invocation, yield/resume, and return without a special runtime construct.

Gate: both language teams and the Rust lowerer can implement without inventing
syntax, graph fields, operand shapes, ordering rules, or diagnostics.

### P2 — Rust lowering and registered AIS

- implement closed FrontendGraph verification and canonicalization;
- construct typed CFG/SSA, nested structural regions, block arguments,
  loop-carried Context, yield/resume, structured joins, and Hook wrappers;
- select the five AIR operations from typed frontend intent;
- amend the Rust-owned AIS operation signatures and regenerate the registered
  dialect without adding an operation;
- emit and verify registered `ais.*` operations with complete operands/results
  and unregistered dialects disabled;
- preserve source/static-node identity through every legal optimization and
  emit explicit optimization provenance; and
- return graph/AIR/AIS diagnostics and the artifact from one compiler result.

Gate: end-to-end goldens prove every row in section 11; mutation tests prove
that dropped operands, raw op strings, malformed SSA, and unregistered
operations fail before artifact production.

### P3 — Python source-first frontend

- implement `Agent`, `Context`, `Tool`, `Model`, focused advanced values, and
  inferred callback typing;
- implement native-AST binding, immutable `BoundAgentTree`, closed passes,
  deterministic traversal, and static validation;
- record handler metadata for the separate bundler without executing it;
- submit FrontendGraph through the explicit PyO3 compiler bridge; and
- move the imperative recorder to private conformance support.

Gate: clean-wheel consumers compile every Python golden with no node/region id,
source-span binding, `AgentProgram`, `AgentFacade`, graph builder, CLI subprocess,
network request, or runtime dependency.

### P4 — TypeScript source-first frontend

- implement the equivalent browser-safe declarations and compiler transform;
- implement the equivalent typed `BoundAgentTree` and deterministic traversal
  over TypeScript compiler symbols/types;
- infer callback and `agent.context` types;
- emit the same handler metadata and FrontendGraph contracts for the separate
  bundler;
- submit local compilation through Node-API and browser compilation through the
  explicit generated remote client; and
- keep native compiler code out of browser bundles.

Gate: clean-package consumers compile every TypeScript golden to canonically
equivalent graph/AIR/diagnostics with no subprocess, implicit remote fallback,
runtime package, or private printer.

### P5 — Examples, Studio generation, and documentation

- replace the conversational teaching sources with the reviewed short API and
  consume Studio-owned Gao as an immutable external generic-program
  conformance input, while retaining low-level cases only as private compiler
  fixtures;
- establish one canonical Python/TypeScript golden source pair per public
  construct and validate or generate guide snippets from those files;
- update all Agent Program guides with the same paired examples and status
  labels, with no guide-local syntax invention;
- update Studio source generation to emit readable short-API source and then
  use the same frontend capture path;
- prove the conversational example through the provider-neutral inference
  contract and an exact APXM-vLLM binding without changing source semantics;
- prove Event wait and root activation examples through Server-managed
  occurrence/application, activation claims, prepared-effect work, and the
  Agents `ActivationRunner`/Execution Commit seam using only generated
  target-owner contracts;
- project the example only from canonical source maps and runtime evidence,
  including Model/Capability nodes, Hooks, Events, Context transitions, loop
  iterations, attempts, usage, outputs, and failures; and
- update generated reference/API docs from the packed packages.

Gate: an author can move between guide source, generated source, FrontendGraph,
AIR, Server-managed execution, and runtime evidence without encountering a
second vocabulary or behavior source. No target example, generated client, or
projection consumes a retiring OS contract or route.

### P6 — Full replacement and absence proof

- remove public `AgentProgram`, `AgentFacade`, recorder, node/region/source-span,
  raw operation constant, and direct lowering exports from authoring packages;
- remove the old imperative examples and docs;
- retain compiler bridges in their focused packages and graph inspection only
  as compile results/developer tooling; and
- regenerate and verify every affected target consumer from the exact
  owner-publication cohort, delete every retiring OS frontend/compile/evidence
  consumer and generated client, and publish Python, TypeScript, compiler,
  examples, Studio generator, generated docs, and current-owner consumers in
  one Compatibility Set with no OS target descriptor.

Gate: package export scans and clean-consumer negatives prove the old builder
and compatibility aliases are absent, while target-consumer and descriptor
scans prove every retiring OS surface is absent.

## 13. Affected files and consumers

Primary owner paths:

- `docs/adr/`, `CONTEXT.md`, this plan, the parent contract/plan, and guides;
- `crates/compiler/frontend/python/apxm_program/**`;
- `crates/compiler/frontend/typescript/src/**`;
- FrontendGraph/AIR schemas, vectors, Rust types, validation, and lowering;
- Rust AIS definitions, generated TableGen, canonical MLIR emission, and
  compiler bridge result contracts;
- native PyO3 and Node-API compiler bridges;
- `examples/agents/conversational/**`, `examples/agents/coder/**`, and the
  immutable Studio-owned Gao conformance input; and
- packed-package and clean-consumer fixtures.

Downstream consumers:

- Studio generated-source authoring and its explicit generated remote compile
  client;
- CLI/application source compilation and generated lifecycle/evidence clients;
- Server artifact admission, managed activation/runtime composition, and
  operational evidence projection;
- artifact source maps and handler bundles; and
- documentation/reference generation.

There is no target OS downstream consumer. Any OS-named source, client,
configuration, fixture, or release input found during implementation is a
full-removal item and must be deleted from the target candidate rather than
translated, aliased, or retained.

Public API impact: breaking full replacement. There is no dual constructor,
alias period, old graph reader, or compatibility translator.

### 13.1 Documentation and surface alignment

The following surfaces teach different depths of one API; they do not own
different APIs:

| Surface | Responsibility | Alignment evidence |
| --- | --- | --- |
| This master plan | Proposed names, language projections, representation stack, lowering, phases, gates | D0 owner review and pinned baseline |
| `creating-an-agent-program.md` | Minimal through advanced author journey | Validated snippets from canonical Python/TypeScript goldens |
| `creating-a-conversational-agent.md` | One loop-based example using generic constructs | Same declarations plus loop/Event/Hook/composition vectors |
| Composition and model guides | Focused `.new`/`.invoke` and exact Model behavior | Links and snippets from the same surface manifest |
| Frontend package READMEs | Current package status and target entry syntax | Packed clean-consumer export/type tests |
| Example READMEs | Honest current scaffold versus target example status | `dekk agents test-frontend-examples` |
| Generated API/reference docs | Exhaustive accepted signatures and types | Generated from packed package declarations and the surface manifest |
| Studio generated source | Readable source using only the public surface | Compile through the same Python/TypeScript frontend goldens |

The implementation phase introduces a documentation-alignment check that
fails when:

- a guide imports a public name absent from either language's surface manifest;
- paired examples represent different semantic nodes or lower differently;
- a code fence drifts from its canonical compilable golden;
- a target-design example loses its design status before the packed API ships;
- current-baseline prose claims proposed decorator/factory behavior is wired;
  or
- a README teaches recorder, node/region id, raw operation, AIR printer, or
  compatibility syntax as public authoring.

Until generated snippets exist, this plan is the proposed spelling authority
and every teaching document must link back to it and carry an explicit design
status.

## 14. Verification

| Phase | Required evidence |
| --- | --- |
| D0-P1 | owner-doc link/status audit, surface-manifest/decorator-matrix review, schema/vector validation, cross-language source and diagnostic review, exact owner-descriptor/generation-cohort drift checks |
| P2 | `dekk agents build-dialect`, `dekk agents codegen`, `dekk agents test -p apxm-program`, canonical compiler tests, registered-only MLIR negative tests |
| P3 | `dekk agents test-python-frontend`, Python clean-wheel consumer, Python golden graph/AIR/AIS/artifact checks |
| P4 | TypeScript typecheck/tests, browser-bundle ceiling check, Node-API and remote-client graph parity |
| P3-P4 graph changes | `dekk agents check-frontend-codegen`, cross-language canonical graph comparison |
| P5 | `dekk agents test-frontend-examples`, Studio-owned Gao generic-program conformance, Studio generated-source tests, Server-managed source-to-commit/activation/effect/schedule/Host/retry/DLQ/recovery/evidence conformance through generated target-owner clients |
| P6 | package export/retired-surface scans, retiring OS consumer/descriptor absence scan, `dekk agents test-cli`, focused workspace tests, `dekk agents doctor`, release checks |

P5 also requires owner-local inference/vLLM adapter conformance, the Agents
composition-spine test, evidence replay/projection checks, and coordinator
fanout verification. A backend smoke alone cannot close the gate.

AIS signature completion is in scope and follows `ais-op-design`, owner review,
`dekk agents build-dialect`, and `dekk agents codegen`. A missing operation is
not in scope: discovery of one stops implementation for a separate owner
decision. The expected design adds no effect/composition or structural
operation.

## 15. Boundaries

This plan does not:

- add `AgentLoop`, a conversational runtime, hidden model/Tool loop, or a sixth
  effect operation;
- make Python or TypeScript execute Agent Programs;
- let a Tool decorator mint authority or select an implementation;
- expose AIR or a raw graph builder as beginner API;
- reassign Server root admission, managed occurrence/delivery,
  activation/effect durability, schedules, Host gateway, retry/DLQ/recovery,
  Auth grants, runtime profiles, or provider selection;
- add runtime tracing as a compiler frontend; or
- preserve the current imperative API as a supported compatibility surface.

## 16. Rollback and integration

Planning/document edits are reversible before D0 acceptance. Implementation
lands on an isolated feature branch and does not mutate the current release.
Before the compatibility-set point of no return, a failed candidate is
discarded as a whole. Promotion publishes Python, TypeScript, compiler bridges,
examples, generated Studio source, documentation, and exact target-owner
consumers together. The target candidate already excludes every retiring OS
consumer, descriptor, configuration, and topology member before the shared
authority barrier. Before the first irreversible target write, rollback
restores the complete prior release and snapshot; afterward corrections are
forward-only and neither the old builder nor a retired product path is
re-enabled.

## 17. Completion definition

The frontend replacement is complete only when:

- a beginner can write the minimal Python or TypeScript Agent using only
  `Agent` and `Model`;
- contextual Tool-using Agents need only `Agent`, `Context`, `Tool`, and
  `Model` in ordinary source;
- callback types are inferred and no ordinary example imports `AgentFacade`;
- ordinary loops lower to `ais.loop` without `AgentLoop` or authored ids;
- Tool references/decorators lower only through Capability contracts and
  `capability.invoke`;
- Python and TypeScript produce equivalent FrontendGraph, AIR, artifact,
  diagnostics, source maps, and handler sidecars;
- native ASTs lower through a verified immutable bound/typed tree rather than
  mutable recording or execution, with full tree-to-graph coverage;
- Python decorators and TypeScript declaration factories implement every row
  of one reviewed surface manifest with equivalent types and semantic nodes;
- FrontendGraph contains no AIR/AIS operation strings, and public frontend
  packages contain no operation constants or structural AIS kinds;
- Rust emits registered `ais.*` operations with complete typed operands,
  results, CFG/SSA, regions, and block arguments, and canonical verification
  succeeds with unregistered dialects disabled;
- legal optimization preserves effects, authority, durability, source
  correlation, and observable results and emits inspectable provenance;
- an exact APXM-vLLM binding can execute the same provider-neutral Model node
  without adding vLLM names or behavior to Agent source/AIR;
- canonical evidence joins source, artifact, Program Invocation, loop
  occurrence, NodeExecution, attempt, effect/request identity, admitted model
  binding, usage, Context transition, output, and failure;
- Event/activation examples join Agents-owned portable semantics and
  `ActivationRunner`/Execution Commit behavior to Server-managed acceptance,
  application, leases, schedules, Host gateway, durable effects,
  retry/DLQ/recovery, and operational projections through generated contracts,
  with no frontend semantic copy;
- Studio emits the same readable source rather than private graph/AIR;
- guides, package READMEs, generated reference docs, examples, and Studio
  source pass the documentation-alignment check against canonical compilable
  goldens;
- old public recorder/build/lower APIs and compatibility aliases are absent;
  and
- target package, generated-client, conformance, configuration, topology, and
  release scans contain no retiring OS consumer or OS target descriptor.
