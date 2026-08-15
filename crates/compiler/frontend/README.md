# APXM Compiler Frontends

- Current implementation: shipped source-first Python and TypeScript authoring
  surface
Python and TypeScript must provide equivalent language-native views of one
Agent programming model. Both produce the same versioned FrontendGraph; Rust
alone validates it, constructs CFG/SSA and structural AIS, selects the five AIR
operations, verifies registered AIS, and builds the artifact. Frontend packages
never execute an Agent Program or print AIR/MLIR.

The frontend pipeline is deliberately layered:

```text
native language AST + symbols/types
  -> immutable frontend-internal BoundAgentTree
  -> deterministic traversal
  -> versioned language-neutral FrontendGraph
  -> Rust CFG/SSA, AIR, and registered AIS/MLIR
```

`BoundAgentTree` is an internal name, not another wire contract. It
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

Python expresses local declarations two ways. `@Agent(...)`, `@Context`, and
`@Hook.before(...)` / `@Hook.after(...)` are decorators, because each attaches
to a definition the author is already writing. `Tool`, `Capability`, `Model`,
`Event`, and `Skill` are typed subscript factories bound to a name —
`Tool[In, Out](ref)`, `Model[In, Out](ref)`, `Skill(id, entry=...)` — not
decorators. TypeScript uses the equivalent typed `Agent(...)`, `Context(...)`,
`Tool(...)`, `Capability(...)`, and `Hook(...)` declaration factories. Both bind
to the same semantic-node matrix; neither executes user callbacks while
compiling.

The Python package is [`python/`](python/) and exports `apxm_program`.
TypeScript is [`typescript/`](typescript/) and exports `@apxm/frontend`.

The packages expose the source-first declaration surface only. They do not
export an imperative `AgentProgram` recorder, graph builder, node or region
identity, raw operation constant, or AIR printer.
