# @apxm/frontend — TypeScript Agent authoring frontend

Author an Agent in ordinary TypeScript with typed definitions and inferred
callback types:

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

`Agent`, `Context`, `Tool`, and `Model` cover ordinary programs;
`Capability`, `Event`, `Hook`, and `TaskGroup` are focused extensions.
TypeScript uses the compiler AST, symbols, and TypeChecker to build an immutable
frontend-internal typed source tree, then deterministically traverses it into
FrontendGraph. It never executes the Agent or prints AIR or MLIR.

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
