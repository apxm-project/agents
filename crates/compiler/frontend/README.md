# APXM Compiler Frontends

- Current implementation: low-level FrontendGraph conformance scaffold
- Target authoring syntax: design proposal pending the frontend D0 owner
  decision
- Target guide:
  [Author an Agent](../../../docs/guides/creating-an-agent-program.md)
- Delivery master plan:
  [Source-first Agent frontend](../../../docs/agents/simple-agent-authoring-frontend-plan.md)

Python and TypeScript must provide equivalent language-native views of one
Agent programming model. Both produce the same versioned FrontendGraph; Rust
alone validates it, constructs CFG/SSA and structural AIS, selects the five AIR
operations, verifies registered AIS, and builds the artifact. Frontend packages
never execute an Agent Program or print AIR/MLIR.

The target frontend pipeline is deliberately layered:

```text
native language AST + symbols/types
  -> immutable frontend-internal BoundAgentTree
  -> deterministic traversal
  -> versioned language-neutral FrontendGraph
  -> Rust CFG/SSA, AIR, and registered AIS/MLIR
```

`BoundAgentTree` is a proposed internal name, not another wire contract. It
retains typed lexical semantics and source locations while removing surface
syntax differences. FrontendGraph remains the sole language-neutral handoff.
Each transition is verified; public decorators/functions never act as mutable
graph recorders.

The intended everyday vocabulary is:

```text
Agent     Context     Tool     Model     ordinary language control flow
```

Focused programs may also use `Capability`, `Event`, `Hook`, and `TaskGroup`.
A conversational Agent is ordinary source containing a loop and yield/resume;
it is not a package export or runtime mode.

Python expresses local declarations with `@Agent`, `@Context`, `@Tool`,
`@Capability`, and static `@Hook` markers. TypeScript uses the equivalent typed
`Agent(...)`, `Context(...)`, `Tool(...)`, `Capability(...)`, and `Hook(...)`
declaration factories. Both bind to the same semantic-node matrix; neither
executes user callbacks while compiling.

The Python package is [`python/`](python/) and exports `apxm_program`.
TypeScript is [`typescript/`](typescript/) and exports `@apxm/frontend`.

At the pinned baseline, both packages expose an imperative `AgentProgram`
recorder that requires authored node/region identities and raw operation
kinds. That surface exists for executable parity fixtures while the contract
replacement is designed; it is not the target teaching or compatibility API.
