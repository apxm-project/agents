# @apxm/frontend — generic TypeScript Agent Program frontend

`@apxm/frontend` exposes generic Agent Program, Hook, Context, composition,
structured-control-flow, source-map, and native compiler-bridge APIs. It
records `apxm.frontend-graph.v1`; the Rust compiler alone validates and lowers
that graph to AIR.

Structured authoring uses `branch`, `switch`, `loop`, `parallel`, `tryCatch`,
`throwRegion`, `returnRegion`, `yieldRegion`, and joined `structuredTask`
scopes.

## Compilation contract

```text
AgentProgram
  -> FrontendGraph
  -> explicit native compiler bridge
  -> canonical AIR
```

## Authoring

```ts
import { AgentProgram } from "@apxm/frontend";

const program = new AgentProgram({
  program_id: "hello",
  input_type_ref: "Input",
  output_type_ref: "Output",
  context_type_ref: "Context",
});
program.loop("region.loop", (body) =>
  body.modelCall("node.greet", "model.default"),
);
program.returnRegion("region.return");

console.log(program.canonicalAir());
```

## Checks

```sh
dekk agents test-typescript-frontend
```
