# @apxm/frontend — TypeScript authoring frontend

`@apxm/frontend` is one of APXM's three supported authoring frontends. Its
`GraphBuilder` and `ApxmGraph` record the compiler-owned `FrontendGraph` DTO;
they do not format MLIR or own a second operation catalog.

## Compilation contract

```text
GraphBuilder / ApxmGraph
  -> ApxmGraph.toDict()
  -> apxm canonical-air
  -> Rust FrontendGraph validation and native AIR bridge
  -> canonical AIR
  -> MLIR compiler pipeline
  -> .apxmobj
```

The TypeScript frontend records a canonical `apxm.frontend-graph.v1` value. A
caller that needs AIR passes that graph to the explicit native bridge (`apxm
canonical-air`) or to an admitted remote compile client. The frontend package
does not invoke the CLI, print AIR, or select a fallback compiler mode.

## Authoring

```ts
import { GraphBuilder } from "@apxm/frontend";

const graph = new GraphBuilder("hello", { metadata: { is_entry: true } });
graph.param("name", "str");
const greeting = graph.ask({ name: "greet", prompt: "Greet {name}." });
graph.done(greeting, "out");

console.log(JSON.stringify(graph.toDict()));
```

Entry metadata is explicit. Executable programs contain exactly one
`metadata.is_entry: true` flow; multi-flow programs mark every other flow
`false`.

Operation names and required attributes are generated from Rust-owned AIS
definitions. `GraphBuilder` uses the generated `REQUIRED_ATTRS` table for early
authoring errors, and the Rust `FrontendGraph` boundary validates again before
printing AIR.

## Tools and hooks

`compileHandlers()` creates the versioned `HandlerManifest` sidecar used by
both Python and TypeScript. Each descriptor carries a kind (`tool` or `hook`),
stable handler ID, language, typed hook fields where applicable, and
artifact-local bundled source. The runtime indexes tools by capability name and
all handlers by handler ID.

## Generated code and checks

`src/generated/` is generated from the Rust AIS operation definitions. Do not
edit it directly. Regenerate through the owning Dekk command:

```bash
dekk agents codegen
dekk agents test-typescript-frontend
dekk agents check-frontend-codegen
dekk agents check-frontend-parity
```

The parity command builds a real `apxm` binary and executes both Python and
TypeScript native-authoring vectors against the same DTO and canonical AIR
fixtures. It fails if either supported frontend cannot run.
