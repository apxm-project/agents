# Author an Agent

- Architectural status: accepted APXM v1 source → FrontendGraph → Rust → AIR
  direction
- Frontend syntax status: design proposal; the short API shown here is not
  implemented at `agents@8ccf4040bd341d074954489bdd61112f3bee294b`
- Implementation plan:
  [Source-first Agent frontend master plan](../agents/simple-agent-authoring-frontend-plan.md)
- Audience: Python and TypeScript authors

## 1. The simple rule

Writing an Agent should require a small vocabulary:

```text
Agent     Context     Tool     Model     ordinary Python/TypeScript
```

- `Agent` declares behavior.
- `Context` declares typed persistent state.
- `Tool` declares a typed model-callable action.
- `Model` declares one exact typed model target.
- `if`, `match`/`switch`, `while`, `for`, try/catch, and return express control
  flow normally.

The callback parameter is simply named `agent`. Its type is inferred, so normal
source does not import or mention `AgentFacade`.

### 1.1 Decorators and declaration factories

Python uses decorators for local declarations because that is the clearest
native syntax: `@Agent`, `@Context`, `@Tool`, `@Capability`, and
`@Hook.before`/`@Hook.after`. Imported Models, Tools, and Capabilities use typed
binding factories because they bind exact external references rather than
decorate local handlers.

TypeScript uses typed declaration factories—`Agent({...})`, `Context(...)`,
`Tool({...})`, `Capability({...})`, and `Hook.before({...})`—because TypeScript
does not have an equally direct standalone-function decorator form. These are
semantic equivalents, not different programming models. Both produce the same
bound semantic nodes and FrontendGraph intents.

Markers are statically recognized by imported symbol identity. Compiling does
not execute the decorator, declaration factory callback, Agent body, or handler
to discover behavior. The complete proposed matrix and composition rules live
in the master plan's
[language projections and decorator matrix](../agents/simple-agent-authoring-frontend-plan.md#24-language-projections-and-decorator-matrix).

## 2. What the frontend does

The frontend parses the complete source, binds and type-checks it into an
immutable semantic tree, and deterministically traverses that tree into
`apxm.frontend-graph.v1`. It does not execute the Agent to trace one path, and
authors do not construct graph nodes.

```text
typed Python or TypeScript
        |
        | host parser + symbol/type binding
        v
immutable typed BoundAgentTree
        |
        | deterministic verified traversal
        v
typed FrontendGraph intent
        |
        | explicit native or remote compiler bridge
        v
Rust validation, CFG/SSA construction, and AIS selection
        |
        v
AIR v1 -> registered AIS verification -> executable artifact
        -> admission -> generic runtime
```

`BoundAgentTree` is a proposed internal implementation name, not a public API
or serialized contract. It is the frontend's immutable, typed view of the
complete source after names and APXM constructs are bound. FrontendGraph is the
first language-neutral wire representation. Rust then becomes the single
authority that turns structured intent into canonical CFG/SSA and AIR/AIS.

That one path is what makes the frontend more than syntax sugar. The compiler
can analyze types, dependencies, effects, Context flow, loops, and structured
concurrency; legal optimizations retain source/static-node identity. Runtime
then records dynamic NodeExecutions, attempts, effects, loop occurrences,
Context transitions, usage, outputs, and failures that Studio can project back
onto the same source. Exact inference implementations such as APXM-vLLM remain
behind admitted ports and do not change the Agent API.

This boundary is fixed by
[ADR-0006](../adr/0006-authoring-frontends-use-explicit-compiler-bridges.md),
the
[Agent Program contract](../agents/agent-program-composition-and-air-contract.md),
and
[ADR-0014](../adr/0014-conversational-agent-and-gao-are-examples-over-generic-agent-program-apis.md).

## 3. Python

### 3.1 Minimal Agent

```python
from apxm_program import Agent, Model

SummarizerModel = Model[SummaryRequest, Summary](ExactSummarizerModelRef)


@Agent(input=SummaryRequest, output=Summary)
async def Summarizer(agent, request):
    return await SummarizerModel(request)
```

That is a complete one-shot Agent. The source does not name a graph, node,
region, model operation, compiler, AIR module, runtime, or deployment.

### 3.2 Context, Tool, Model, and a loop

```python
from apxm_program import Agent, Context, Model, Tool


@Context
class Conversation:
    messages: tuple[Message, ...] = ()


SearchWeb = Tool[SearchRequest, SearchResult](SearchWebCapabilityRef)
SupportModel = Model[ModelRequest, ModelResponse](ExactSupportModelRef)


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

The loop, optional Tool call, Model call, context update, and stateful yield are
visible in ordinary source. There is no hidden model/Tool loop or implicit
memory update.

### 3.3 A bundled Tool

```python
from apxm_program import Tool


@Tool
async def NormalizeAddress(request: AddressInput) -> NormalizedAddress:
    return normalize_address(request)
```

The frontend recognizes the decorator and binds the typed Tool declaration and
handler metadata into its semantic tree for the separate handler bundler. It
does not run the handler while compiling and does not grant permission.
Calling `NormalizeAddress(...)` from an Agent emits one typed Capability intent
into FrontendGraph; execution still requires admission.

### 3.4 Compose Agents

```python
specialist = SecurityReviewer.new(
    context=ReviewerContext(repository=request.repository),
)
review = await specialist.invoke(ReviewRequest(diff=request.diff))
summary = await Summarizer.invoke(SummaryRequest(review=review))
```

`.new(...)` creates a typed stateful Program Instance reference. Invoking the
Agent definition is one-shot; invoking the returned instance is stateful.

## 4. TypeScript

### 4.1 Minimal Agent

```typescript
import { Agent, Model } from "@apxm/frontend";

const SummarizerModel = Model<SummaryRequest, Summary>(
  ExactSummarizerModelRef,
);

export const Summarizer = Agent<SummaryRequest, Summary>({
  async run(agent, request) {
    return await SummarizerModel(request);
  },
});
```

The callback parameter is inferred. Authors do not import `AgentFacade`.

### 4.2 Context, Tool, Model, and a loop

```typescript
import { Agent, Context, Model, Tool } from "@apxm/frontend";

type Conversation = {
  messages: readonly Message[];
};

const ConversationContext = Context<Conversation>({ messages: [] });
const SearchWeb = Tool<SearchRequest, SearchResult>(SearchWebCapabilityRef);
const SupportModel = Model<ModelRequest, ModelResponse>(ExactSupportModelRef);

export const Support = Agent<
  ConversationInput,
  ConversationOutput,
  Conversation
>({
  context: ConversationContext,
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

### 4.3 A bundled Tool

```typescript
import { Tool } from "@apxm/frontend";

export const NormalizeAddress = Tool<AddressInput, NormalizedAddress>({
  async run(request) {
    return normalizeAddress(request);
  },
});
```

### 4.4 Compose Agents

```typescript
const specialist = SecurityReviewer.new({
  context: { repository: request.repository },
});
const review = await specialist.invoke({ diff: request.diff });
const summary = await Summarizer.invoke({ review });
```

## 5. Friendly operations and exact lowering

The frontend exposes operations through typed values instead of raw operation
builders. It understands whether a bound value is a Model, Tool, Capability,
Agent definition, Agent instance, or Event. It records that typed intent;
Rust—not Python or TypeScript—selects the AIR/AIS operation.

| Friendly source | FrontendGraph intent | Rust-selected AIR | Registered AIS MLIR |
| --- | --- | --- | --- |
| `await SupportModel(request)` | typed Model invocation | `model.call` | `ais.model_call` |
| `await SearchWeb(arguments)` | typed Tool invocation + Capability requirement | `capability.invoke` | `ais.capability_invoke` |
| `Specialist.new(context=...)` | typed Agent creation | `program.new` | `ais.program_new` |
| `await Specialist.invoke(input)` | typed definition receiver + input | `program.invoke` | `ais.program_invoke` |
| `await instance.invoke(input)` | typed instance receiver + input | `program.invoke` | `ais.program_invoke` |
| `await approval.wait()` | typed Event wait | `await.event` | `ais.await_event` |
| ordinary `while`/`for` | loop CFG with carried values | structural `ais.loop` | `ais.loop` |
| `agent.context = next_context` | typed state value/edge | structural value and block arguments | registered structural AIS/SSA |
| `await agent.yield_(output)` | commit values + distinct resume-input successor | structural yield | `ais.yield` |

There is deliberately no normal `AgentLoop` import. A language loop is easier
to read and analyze. The frontend captures it, and Rust emits `ais.loop`.
Optional source labels may identify a loop for Hooks or evidence, but authors
never supply graph region ids.

Declarations do not execute effects. `Model(...)`, `Tool(...)`, `Context(...)`,
and `Agent(...)` bind typed semantic-tree declarations that later emit
FrontendGraph; an AIR effect appears only at an invoked callsite. Context
assignments become value flow, not a sixth operation.

This complete lowering is part of the linked design plan. At the pinned
baseline, FrontendGraph still carries AIR operation strings and `ais.loop`, the
Rust lowerer mostly copies those records, and the MLIR emitter omits semantic
operands. The
[baseline gap table](../agents/simple-agent-authoring-frontend-plan.md#9-baseline-findings-and-required-corrections)
identifies the owning source for each gap; the guide syntax is not implemented
until those gates close.

## 6. Tool versus Capability

`Tool` is the simple author-facing form for an action that can be shown to a
model. Underneath, it remains a Capability:

```text
Tool declaration
  -> typed Tool schema
  -> Capability requirement
  -> capability.invoke
  -> Auth-owned grant/approval
  -> exact admitted implementation
```

A Tool declaration cannot contain credentials, grants, endpoints, provider
fallbacks, or implementation selection. A Tool request returned by a model is
data. Source chooses a closed Tool variant, invokes it, and decides whether to
call the Model again.

`Model(...)` receives a typed digest-pinned Model Target reference. An imported
`Tool(...)` receives a typed Capability-definition reference; a local `@Tool`
derives its identity from the exported source declaration and typed schema.
Mutable display names such as `"default-model"` and `"search-web"` are not
executable source references.

Programs that need an executable action which is not model-callable may use the
focused advanced `Capability` API. That distinction keeps APXM authority
precise without making every beginner learn the infrastructure vocabulary.

## 7. Context and the `agent` value

`Context` is ordinary typed information, not authority. Source reads and
replaces `agent.context` explicitly. Only a successful yield/return commits the
next value for a stateful instance.

The `agent` parameter is the safe program-visible view. Depending on the
callsite, it can expose typed context, input/output, current model/Tool call,
identity view, budget/deadline, and cancellation state. It never exposes
credentials, grants, scheduler, database, runtime object, graph mutation, or
ambient filesystem/network access.

The precise internal contract may still call this view `AgentFacade`; normal
source should not import or annotate it. Python decorators and TypeScript
generics infer the type.

## 8. Graphs are output, not authoring syntax

The frontend derives node/region identities, source spans, execution order,
typed values, and context edges from lexical source structure. Authors may
inspect FrontendGraph and AIR after compilation, but neither is editable
behavior source.

Visual authoring and Studio should generate readable Python or TypeScript and
use the same capture path. They must not maintain a Studio-only graph emitter.

Internally, the language frontend may use visitors and compiler passes over its
bound typed source tree. Those passes have explicit inputs and outputs, retain
source locations, preserve evaluation order, and never execute the Agent body.
This provides the same separation that makes mature compiler toolchains
powerful: source syntax, high-level semantic representation, language-neutral
graph, canonical IR, optimization IR, and executable artifact are distinct
verified levels.

At the current baseline, the repository
[conversational example](../../examples/agents/conversational/README.md)
still uses the low-level recorder API. It remains an executable parity fixture
until the plan linked above implements the short API and replaces the example
atomically in Python and TypeScript.

## 9. Acceptance checklist

The frontend is ready when:

- minimal Agents need only `Agent` and `Model`;
- contextual Tool-using Agents need only `Agent`, `Context`, `Tool`, and
  `Model`;
- callback types are inferred and examples never import `AgentFacade`;
- authors write ordinary loops without `AgentLoop`, node ids, or region ids;
- each frontend binds its native AST into an immutable typed source tree and
  deterministically traverses it into FrontendGraph without running user code;
- Python and TypeScript produce equivalent FrontendGraph, AIR, diagnostics,
  source maps, artifacts, and handler sidecars;
- FrontendGraph contains typed source intent rather than raw AIR/AIS operation
  strings, and registered AIS verification preserves every operand/result;
- legal optimization preserves program meaning, authority, durability,
  source/evidence correlation, and observable outcomes;
- exact inference backends, including APXM-vLLM, execute through the same
  provider-neutral Model contract without appearing in source/AIR; and
- Tool decorators/references lower only through Capability contracts and never
  grant authority;
- frontend packages contain no runtime, AIR printer, compiler fallback, or raw
  public operation builder; and
- the old imperative teaching examples and compatibility exports are absent.
