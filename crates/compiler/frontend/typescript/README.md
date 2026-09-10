# @apxm/frontend — TypeScript Agent authoring frontend

Author an Agent in ordinary TypeScript with typed definitions and inferred
callback types:

```typescript
import { Agent, Model } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";

source(import.meta.url);

type SummaryRequest = { readonly text: string };
type Summary = { readonly text: string };

const SummarizerModel = Model<SummaryRequest, Summary>("model.summary");

export const Summarizer = Agent<SummaryRequest, Summary>({
  name: "Summarizer",
  model: SummarizerModel,
  async run(agent, request) {
    return await SummarizerModel(request);
  },
});
```

`Agent` requires an explicit typed `model` binding invoked by the captured body.
`Workflow` declares general orchestration and needs no model when it makes no
model call. Both return the same sealed `Program<Input, Output, Context>` with
typed `invoke(input)` and `new({context})`; neither exposes a raw graph constructor.
Loops, Context, Hooks and yield/resume are shared program behavior.

`Agent`, `Workflow`, `Context`, `Tool`, and `Model` cover ordinary programs;
`Capability`, `Event`, `Hook`, `Skill`, and `TaskGroup` are focused extensions.
`Skill` declares instructions the program can load — carried as a package file
with `{ entry }`, or written in the source with `{ text }`, never both — and
`await skill.load()` reads them through the skill-reading Capability, so the
artifact declares that authority the way it declares any other.
TypeScript uses the compiler AST, symbols, and TypeChecker to build an immutable
frontend-internal typed source tree, then deterministically traverses it into
FrontendGraph. It never executes the Agent or prints AIR or MLIR.

`Agent<Input, Output, Context>` states the program's typed interface: the types
themselves, never strings naming them. Python states the same three the same way,
as the real classes passed to `@Agent`.

A module states its own source once, above its Agent definitions, with
`source(import.meta.url)` from `@apxm/frontend/node`. Python recovers the
authored text through `inspect`; JavaScript has no equivalent, so the module
supplies it — but it is a fact about where the Agent was written, not an argument
its author passes.

Declare Models, Tools, Context, Events, and composed Agents at module scope. The
declarations a module makes are its Agents' declarations: an author does not list
again the names their own body already names. The frontend resolves those
bindings by symbol identity and rejects dynamic lookup or shadowing.

## Compilation contract

```text
typed Agent source
  -> native AST + symbols/types
  -> immutable bound/typed source tree
  -> FrontendGraph
  -> explicit native compiler bridge
  -> Rust AIR/AIS lowering and artifact result
```

The package exposes only these declaration markers. It has no imperative program
builder, region or node recorder, operation constant, or raw graph/AIR
inspection API.

## Checks

```sh
dekk agents test-typescript-frontend
```
