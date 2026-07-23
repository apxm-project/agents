# Retired prototype graph model

- Status: historical implementation note; not an authoring or compiler contract
- Target contract:
  [FrontendGraph v1](../../../../../docs/agents/agent-program-composition-and-air-contract.md#7-frontendgraph-v1)
- Replacement plan:
  [Source-first Agent frontend master plan](../../../../../docs/agents/simple-agent-authoring-frontend-plan.md)

The former `ApxmGraph`/`GraphRecorder` model used a plain
`name`/`nodes`/`edges`/`parameters`/`metadata` DTO with arbitrary operation
strings. Its cited Python and TypeScript implementation files are no longer
present in this package. That shape is not `apxm.frontend-graph.v1` and must not
be revived as a frontend, Studio, compiler, or compatibility path.

The current executable `apxm_program` scaffold records the shallow
`apxm.frontend-graph.v1` schema through
[`apxm_program`](../apxm_program/__init__.py). The target replacement records a
complete typed language-neutral graph containing Agent/Context/Model/Tool/
Capability/Event/Hook declarations, typed values, functions, blocks, regions,
data/control/context edges, source-semantic call intents, and source maps.

Authors never construct this graph directly. Python and TypeScript capture
ordinary typed Agent source, and Rust alone validates the graph, constructs
CFG/SSA, selects AIR/AIS operations, performs legal optimization, and builds
the artifact. FrontendGraph remains inspectable compiler input, not a second
behavior language.
