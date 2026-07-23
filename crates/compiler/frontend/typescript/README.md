# @apxm/frontend — TypeScript Agent authoring frontend

- Current implementation: low-level conformance recorder
- Target syntax: design proposal; not implemented at the pinned frontend
  baseline
- Target guide:
  [Author an Agent](../../../../docs/guides/creating-an-agent-program.md)

The target TypeScript experience uses typed definitions and inferred callback
types:

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
FrontendGraph. It never executes the Agent or prints AIR/MLIR.

## Target compilation contract

```text
typed Agent source
  -> native AST + symbols/types
  -> immutable bound/typed source tree
  -> FrontendGraph
  -> explicit native compiler bridge
  -> Rust AIR/AIS lowering and artifact result
```

The current package still exposes imperative `AgentProgram`, raw region/node
recorders, and direct graph/AIR inspection helpers for repository parity
fixtures. Those APIs are baseline evidence, not the intended author surface,
and the full replacement keeps no compatibility alias for them.

## Checks

```sh
dekk agents test-typescript-frontend
```
