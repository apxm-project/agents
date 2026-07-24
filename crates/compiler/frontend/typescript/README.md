# @apxm/frontend — TypeScript Agent authoring frontend

Author an Agent in ordinary TypeScript with typed definitions and inferred
callback types:

```typescript
import { Agent, Model } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type SummaryRequest = { readonly text: string };
type Summary = { readonly text: string };

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

`Agent`, `Context`, `Tool`, and `Model` cover ordinary programs;
`Capability`, `Event`, `Hook`, and `TaskGroup` are focused extensions.
TypeScript uses the compiler AST, symbols, and TypeChecker to build an immutable
frontend-internal typed source tree, then deterministically traverses it into
FrontendGraph. It never executes the Agent or prints AIR or MLIR.

For Node compilation, give every Agent a portable static source token. Keep
this helper beside the Agent source:

```typescript
// static-source.ts
import { readFileSync } from "node:fs";
import { relative } from "node:path";
import { fileURLToPath } from "node:url";

export function staticSource(url: string) {
  const absoluteFileName = fileURLToPath(url);
  return {
    fileName: relative(process.cwd(), absoluteFileName),
    text: readFileSync(absoluteFileName, "utf8"),
  };
}
```

Declare Models, Tools, Context, Events, and composed Agents at module scope.
Pass every binding used in an Agent body through `use`; the frontend resolves
those bindings by symbol identity and rejects dynamic lookup or shadowing.

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
