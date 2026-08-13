# APXM Agent Programs

This vocabulary is intentionally small and product-neutral.

## Program

An authored Agent Program is source code that declares typed inputs, outputs,
Context, model bindings, capability bindings, event waits, composition, and
ordinary control flow. Python and TypeScript are equivalent source projections.

## Compilation

The frontend captures source without executing effects and emits one
`apxm.frontend-graph.v2` document. Rust verifies and lowers it to
`apxm.air.v2`, then emits a digest-bound executable artifact and source map.

The public semantic operation family is closed:

- `model.call`
- `capability.invoke`
- `program.new`
- `program.invoke`
- `await.event`

Structural regions, blocks, branches, loops, and yields are compiler-owned.

## Runtime

An artifact is executable only after exact Invocation Admission supplies the
required Port bindings, authority, target identities, confinement, and
resource ceilings. The runtime does not discover providers, select a fallback,
or infer authority from a handler name.

Capabilities are typed callable bindings plus permission policy. Model targets,
events, composition, and execution commit are the same kind of explicit Port
boundary. External products may provide implementations, but they do not
change the abstract-machine semantics.

## State and evidence

Program Context is an explicit typed value owned by the authored program.
Hooks are static before/after callbacks over the Agent Facade. Loops are
ordinary authored control flow; runtime evidence records generic committed loop
iterations and does not introduce a product-specific `Turn` type.

Execution is single-flight per Program Instance. Yield retains the continuation;
return completes it; `await.event` parks and resumes the same invocation. Every
authoritative transition is append-only and monotonic.

## Boundaries

Studio, Server, Auth, OS hosts, Telegram, ACP clients, deployment, provider
fleets, and evaluation studies are external composition roots. They may use
these contracts through exact admission and Port bindings, but are not
repository concepts or frontend APIs.

For historical theory, read [docs/pxm](docs/pxm/readme.md). For implementation
authority, read [the AIR contract](docs/agents/agent-program-composition-and-air-contract.md)
and [the portable core contract](docs/agents/portable-core-interface-contract.md).
