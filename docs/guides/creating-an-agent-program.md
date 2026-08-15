# Author an Agent

- Architectural status: accepted APXM v1 source → FrontendGraph → Rust → AIR
  direction
- Frontend syntax status: implemented source-first authoring surface. Every
  declaration below is stated per language in
  `contracts/vectors/apxm.frontend-surface.json` and held to both frontends'
  own sources by `dekk agents check-frontend-surface`
  (`tools/scripts/check_frontend_surface.py`). That gate reads the frontend
  packages and every authoring document, this guide included, so a snippet
  here that imports a name the surface does not publish, or binds a capability
  reference no catalogue mints, fails CI rather than review.
- Implementation contract:
  [Agent Program composition and AIR](../agents/agent-program-composition-and-air-contract.md)
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

Python uses decorators where a declaration attaches to a local definition,
because that is the clearest native syntax: `@Agent`, `@Context`, and
`@Hook.before`/`@Hook.after`. TypeScript uses declaration factories for the
same three — `Agent({...})`, `Context(...)`, `Hook.before({...})` — because it
has no equally direct standalone-function decorator form.

`Model`, `Tool`, and `Capability` are binding factories in both languages,
never decorators, because they bind an exact external reference rather than
decorate a local body. A Capability a *package ships* is a different thing
again, and it is not spelled with those markers at all — see
[§3.3](#33-a-tool-reference-and-a-shipped-handler).

The whole declaration surface, in the form each language projects it:

<!-- BEGIN DECLARATION SURFACE -->
| Declaration | Tier | Python | TypeScript |
| --- | --- | --- | --- |
| Agent definition | `everyday` | `@Agent(input=InputType, output=OutputType, context=ContextType) on one async def` | `Agent<Input, Output, Context>({ name?, context?, async run(agent, input) })` |
| Context schema/default | `everyday` | `@Context on one typed class whose field defaults are the initial Context` | `Context<Schema>(initial)` |
| Exact Model binding | `everyday` | `Model[Input, Output](ref)` | `Model<Input, Output>(ref)` |
| Imported Tool binding | `everyday` | `Tool[Input, Output](capability_ref, permission=...)` | `Tool<Input, Output>(capabilityRef, { permission })` |
| Imported Capability | `advanced` | `Capability[Input, Output](ref, permission=...)` | `Capability<Input, Output>(ref, { permission })` |
| Shipped Capability handler | `advanced` | `capability({ name, description, read_only, input, run }) -> CapabilityId` | `Tool.define({ name, description, readOnly, input, run }) -> CapabilityId` |
| Static Hook | `advanced` | `@Hook.before(target=..., scope=...) / @Hook.after(target=..., scope=...)` | `Hook.before({ agent, target, scope?, run }) / Hook.after({ ... })` |
| Durable Event value | `advanced` | `Event[Payload](ref), then await event.wait()` | `Event<Payload>(ref), then await event.wait()` |
| Declared Agent Skill | `advanced` | `Skill(skill_id, entry=...) or Skill(skill_id, text=...), then await skill.load()` | `Skill(skillId, { entry }) or Skill(skillId, { text }), then await skill.load()` |
| Structured task scope | `advanced` | `async with TaskGroup()` | `TaskGroup.run(async () => { ... })` |
<!-- END DECLARATION SURFACE -->

Markers are statically recognized by imported symbol identity. Compiling does
not execute the decorator, declaration factory callback, Agent body, or handler
to discover behavior. `contracts/vectors/apxm.frontend-surface.json` is the
declaration matrix itself — it names each language's module, symbol, and
argument form per declaration, and `dekk agents check-frontend-surface` extracts
the real shape from
[Python](../../crates/compiler/frontend/python/README.md) and
[TypeScript](../../crates/compiler/frontend/typescript/README.md) and holds them
to it. The table above is generated from that manifest by
`dekk agents codegen-docs`, so a marker that gains an argument gains it here
rather than waiting for someone to notice.

### 1.2 What a declaration can refuse

A rejection carries a code, so a build can branch on the reason rather than on
the message text. The codes are the manifest's, projected into both frontends by
`dekk agents codegen-diagnostics` and into this table by
`dekk agents codegen-docs`:

<!-- BEGIN DIAGNOSTIC CODES -->
| Declaration | Rejection reasons |
| --- | --- |
| Agent definition | `AgentBodyNotAsync`, `AgentMissingInputOutput`, `AgentDynamicArgument`, `AgentFacadeImported` |
| Context schema/default | `ContextNotTyped`, `ContextMutableGlobal`, `ContextDynamicDefault` |
| Exact Model binding | `ModelRefNotExact`, `ModelDisplayNameRejected`, `ModelUntypedSchema` |
| Imported Tool binding | `ToolRefNotCapability`, `ToolDisplayNameRejected`, `ToolCredentialInSource` |
| Imported Capability | `CapabilityRefNotExact`, `CapabilityDisplayNameRejected` |
| Shipped Capability handler | `CapabilityHandlerUntypedSchema`, `CapabilityHandlerOpenObject`, `CapabilityHandlerReadOnlyUndeclared` |
| Static Hook | `HookTargetUnresolved`, `HookDynamicRegistration`, `HookOrderAmbiguous` |
| Durable Event value | `EventNotTyped`, `EventWaitOutsideBody` |
| Declared Agent Skill | `SkillIdNotExact`, `SkillSourceMissing`, `SkillSourceAmbiguous`, `SkillEntryPathNotCanonical`, `SkillInstructionsOverlong`, `SkillLoadOutsideBody` |
| Structured task scope | `TaskGroupMissingJoin`, `TaskGroupDetachedWork`, `TaskGroupRawCoroutine` |
<!-- END DIAGNOSTIC CODES -->

### 1.3 Authoring conventions

Declare Models, Tools, Capabilities, Events, Context, and composed Agents at
module scope: the declarations a module makes are its Agents' declarations, in
both languages, and an author never lists again the names their own body already
names. Python capture reads the decorated function source directly. A TypeScript
module states its own source once with `source(import.meta.url)` from
`@apxm/frontend/node`. This keeps symbol resolution exact, makes source maps
portable, and lets the frontend reject dynamic lookup or shadowed marker names
rather than infer behavior from text.

A program's typed interface is the types themselves — `@Agent(input=Request,
output=Reply)` and `Agent<Request, Reply>` — never strings naming them.

## 2. What the frontend does

The frontend parses the complete source, binds and type-checks it into an
immutable semantic tree, and deterministically traverses that tree into
`apxm.frontend-graph`. It does not execute the Agent to trace one path, and
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
AIR -> registered AIS verification -> executable artifact
        -> admission -> generic runtime
```

`BoundAgentTree` is a frozen internal implementation name, not a public API
or serialized contract. It is the frontend's immutable, typed view of the
complete source after names and APXM constructs are bound. FrontendGraph is the
first language-neutral wire representation. Rust then becomes the single
authority that turns structured intent into canonical CFG/SSA and AIR/AIS.

That one path is what makes the frontend more than syntax sugar. The compiler
can analyze types, dependencies, effects, Context flow, loops, and structured
concurrency; legal optimizations retain source/static-node identity. Runtime
then records dynamic NodeExecutions, attempts, effects, loop occurrences,
Context transitions, usage, outputs, and failures that a host can project back
onto the same source. Exact inference implementations remain behind admitted
ports and do not change the Agent API.

This boundary is fixed by
[ADR-0006](../adr/0006-authoring-frontends-use-explicit-compiler-bridges.md),
the
[Agent Program contract](../agents/agent-program-composition-and-air-contract.md),
and the repository example boundary.

## 3. Python

### 3.1 Minimal Agent

```python
from apxm_program import Agent, Model

SummarizerModel = Model[SummaryRequest, Summary]("model.summarizer")


@Agent(input=SummaryRequest, output=Summary)
async def Summarizer(agent, request):
    return await SummarizerModel(request)
```

That is a complete one-shot Agent. The source does not name a graph, node,
region, model operation, compiler, AIR module, runtime, or deployment.

### 3.2 Context, Tool, Model, and a loop

```python
from apxm_program import Agent, Context, Model, Tool
from apxm_program.capabilities import SEARCH_WEB


@Context
class Conversation:
    messages: tuple[Message, ...] = ()


SearchWeb = Tool[SearchRequest, SearchResult](SEARCH_WEB)
SupportModel = Model[ModelRequest, ModelResponse]("model.support")


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

### 3.3 A Tool reference, and a shipped handler

A `Tool` reference names an id that something can already satisfy: a builtin
from the generated catalogue, or the handler declaration a package ships at
`capabilities/<id>/handler.ts`. It is not a name an author invents, and it is
not one an author *can* invent — those two arms are the whole accepted set, in
the TypeScript type and in the Python constructor alike.

```python
from apxm_program import Tool
from apxm_program.capabilities import READ
from apxm_program.permissions import Ask


ReadFile = Tool[ReadInput, ReadResult](
    READ,
    permission=Ask("Reads whatever path the model asks for."),
)
```

The Python frontend records this typed Capability reference; it never runs a
handler while compiling. Calling `ReadFile(...)` from an Agent emits one typed
Capability intent into FrontendGraph; execution still requires admission.

`permission=` does not grant anything either — it records what the program is
*asking for*. It is captured onto the FrontendGraph as
`CapabilityRequirement.requested_permission`, whose own contract says it "is the
program's *request*, recorded and digest-bound as authored, and it confers no
authority" (`crates/machine/program/src/frontend_graph.rs`).

Authority is settled above the program, by a closed tighten-only layer stack —
code (10), package (20), deployment (30) — applied in the machine's own enum
order rather than in whatever order a caller assembles it
(`crates/machine/ais/src/permissions.rs`). A layer above may narrow `allow` to
`ask` or `deny`; widening a held decision, or deciding for a capability the
program never requested, is a hard failure that leaves nothing resolved. That is
why writing a permission in source is safe: the widest thing a program can say
is still only a request, and every narrowing below it belongs to the machine.
The one authored layer above code is `agent.toml [permissions]` — see
[the package format](agent-package-format.md#5-permissions-the-tighten-only-layer).

What you write reaches the running machine. Lowering carries the authored
request into AIR as `capability_permission_requests`
(`crates/machine/program/src/lower.rs`), and a composition root handed nothing
but AIR bytes reads it as the code layer: `local_capability_permissions` states
your decision for a reference you wrote one for and a bare `allow` for one you
did not (`crates/tools/cli/src/commands/canonical_execute.rs`). The canonical
local driver brokers no approvals, so an `Ask` that reaches it refuses the
effect and commits the refusal as evidence naming the code layer
(`crates/runtime/execution/src/driver.rs`).

One caveat, so you don't expect more than the tree does: `agent lint` compiles
no program, so it cannot see your request — it resolves `agent.toml` against a
blanket `allow` floor (`crates/tools/cli/src/commands/agent.rs`). The caller
that holds both your request and the package policy is
`compile-service-canonical`, which refuses to emit AIR when `agent.toml` tries
to widen what your source asked for
(`crates/tools/cli/src/commands/compile_service_canonical.rs`).

A Python package declares the handlers it ships with `capability(...)` from
`apxm_program.handlers`, in the same shape `Tool.define` uses in TypeScript, and
gets back the exact Capability id it implements, so the reference and the
implementation are one object. Python has no type-checker in the build, so the
same closed set is settled where the binding is constructed
(`crates/compiler/frontend/python/apxm_program/_markers.py`): `Tool[...]` and
`Capability[...]` admit a catalogue id or a `capability(...)` declaration and
refuse anything else with `ToolRefNotCapability` / `CapabilityRefNotExact`. The
declaration subclasses `str`, so it still prints and compares as the id it
spells, but being the declaration — not equalling one — is what admits it:

```python
from apxm_program import Tool
from apxm_program.handlers import answer, capability, schema, text

edit = capability({
    "name": "edit",
    "description": "Return a before/after proposal without changing a file.",
    "read_only": True,
    "input": schema(
        additional_properties=False,
        properties={
            "file_path": text(required=True, min_length=1),
            "before": text(required=True, min_length=1),
            "after": text(required=True, min_length=1),
        },
    ),
    "run": lambda args: answer({**args, "mutates": False}),
})

ProposeEdit = Tool[dict, dict](edit)
```

Declaring is not executing. `HandlerLanguage` admits only `typescript`
(`crates/machine/contracts/src/types/handler_manifest.rs`),
`CapabilityBindingHandler` has no Python variant
(`crates/machine/contracts/src/types/capability/capability_binding.rs`), and the
package build recognizes only `capabilities/<id>/handler.ts`
(`crates/tools/cli/src/commands/agent.rs`). Python has neither a deterministic
bundler nor an admitted worker adapter, so a Python handler declaration states
something true about a package without becoming an executable handler in a
compiled artifact. See
[ADR-0016](../adr/0016-tool-authoring-and-handler-execution-are-separate.md) and
its amending record
[ADR-0022](../adr/0022-capability-references-resolve-against-a-catalogue-and-permissions-are-declared-requests.md).

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
import { source } from "@apxm/frontend/node";

source(import.meta.url);

type SummaryRequest = { readonly text: string };
type Summary = { readonly text: string };

const SummarizerModel = Model<SummaryRequest, Summary>(
  "model.summarizer",
);

export const Summarizer = Agent<SummaryRequest, Summary>({
  name: "Summarizer",
  async run(agent, request) {
    return await SummarizerModel(request);
  },
});
```

The callback parameter is inferred. Authors do not import `AgentFacade`.

### 4.2 Context, Tool, Model, and a loop

```typescript
import { Agent, Context, Model, Tool } from "@apxm/frontend";
import { SEARCH_WEB } from "@apxm/frontend/capabilities";
import { source } from "@apxm/frontend/node";

source(import.meta.url);

type Conversation = {
  messages: readonly Message[];
};

const ConversationContext = Context<Conversation>({ messages: [] });
const SearchWeb = Tool<SearchRequest, SearchResult>(SEARCH_WEB);
const SupportModel = Model<ModelRequest, ModelResponse>("model.support");

export const Support = Agent<
  ConversationInput,
  ConversationOutput,
  Conversation
>({
  name: "Support",
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

### 4.3 A package-local TypeScript Tool implementation

```typescript
import { Tool } from "@apxm/agent-packaging";

type AddressInput = { line: string };
type NormalizedAddress = { normalized: string };

export const normalizeAddress = Tool.define({
  name: "normalize_address",
  description: "Normalize one address without changing external state.",
  input: Tool.object<AddressInput>({
    line: Tool.text({ minLength: 1 }),
  }),
  run(input) {
    return Tool.answer({ normalized: normalize(input.line) });
  },
});
```

This declaration lives in a package handler module at
`capabilities/normalize_address/handler.ts`. The directory name *is* the
capability id — the handler's existence is the whole declaration that the
package supplies it — so the Agent Program binds the declaration itself:

```typescript
import { normalizeAddress } from "../capabilities/normalize_address/handler.js";

const NormalizeAddress = Tool<AddressInput, NormalizedAddress>(normalizeAddress);
```

That is what [`examples/agents/coder/src/main.ts`](../../examples/agents/coder/src/main.ts)
does for its own `edit` and `test` handlers. Binding the declaration is not a
style preference: `CapabilityReference` is the generated catalogue's closed
`CapabilityId` union or the object a handler declaration returns
(`crates/compiler/frontend/typescript/src/markers.ts`), so
`Tool<AddressInput, NormalizedAddress>("normalize_address")` does not typecheck
at all. Referencing one capability while implementing another is unrepresentable
rather than merely checked, because there is no bare-string arm to spell the
second name in.

The packaging helper generates the Rust-owned handler manifest; it is not a
frontend runtime or an authority path. Authors never write the manifest, JSON
Schema, or worker protocol.

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

The source-first implementation is verified as one representation stack:
FrontendGraph carries typed source intent, Rust verifies it before constructing
CFG/SSA and registered AIS, and package checks reject unsupported recorder or
raw-operation exports.

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
fallbacks, or implementation selection. It may carry a `permission`, but that is
a request the layer stack above it may only tighten, never a grant
([§3.3](#33-a-tool-reference-and-a-shipped-handler)). A Tool request returned by
a model is data. Source chooses a closed Tool variant, invokes it, and decides
whether to call the Model again.

`Model(...)` receives a typed digest-pinned Model Target reference. An imported
`Tool(...)` receives an exact Capability reference, and the accepted set is
closed: a builtin id from the generated catalogue, or the declaration object a
package's own handler returns. A separate package handler module uses
`Tool.define` (TypeScript) or `capability(...)` (Python) to declare one, but
neither can select an implementation or grant authority, and only the TypeScript
declaration reaches an executable handler.

A mutable display name is not an executable source reference. The marker
refuses `"default"`, `"model.default"`, `"support"`, `"search-web"`, and the
empty string outright in both languages
(`crates/compiler/frontend/typescript/src/markers.ts`,
`crates/compiler/frontend/python/apxm_program/_markers.py`); the builtin ids
those names get confused with are `search_web` and the rest of
`apxm_program.capabilities` / `@apxm/frontend/capabilities`.

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

Visual authoring should generate readable Python or TypeScript and use the same
capture path. It must not maintain a product-only graph emitter.

Internally, the language frontend may use visitors and compiler passes over its
bound typed source tree. Those passes have explicit inputs and outputs, retain
source locations, preserve evaluation order, and never execute the Agent body.
This provides the same separation that makes mature compiler toolchains
powerful: source syntax, high-level semantic representation, language-neutral
graph, canonical IR, optimization IR, and executable artifact are distinct
verified levels.

The repository [conversational example](../../examples/agents/conversational/README.md)
uses the same short API in Python and TypeScript. It is an executable parity
fixture, not a package-level conversational abstraction.

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
  source maps, and artifacts, and every declaration's per-language projection is
  stated in `contracts/vectors/apxm.frontend-surface.json` and checked against
  each frontend's own source; TypeScript package handlers separately generate
  the one Rust-validated sidecar when a package supplies them;
- FrontendGraph contains typed source intent rather than raw AIR/AIS operation
  strings, and registered AIS verification preserves every operand/result;
- legal optimization preserves program meaning, authority, durability,
  source/evidence correlation, and observable outcomes;
- exact inference backends execute through the same provider-neutral Model
  contract without appearing in source/AIR; and
- Tool references lower only through Capability contracts and never grant
  authority, and a declared permission is resolved by the tighten-only layer
  stack rather than honoured as written;
- frontend packages contain no runtime, AIR printer, compiler fallback, or raw
  public operation builder; and
- the old imperative teaching examples and compatibility exports are absent.
