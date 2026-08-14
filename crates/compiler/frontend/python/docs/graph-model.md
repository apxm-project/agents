# Retired prototype graph model

This is a historical implementation note, not an authoring or compiler
contract. The current frontend emits the versioned `FrontendGraph` consumed by
the Rust compiler; the Python package does not expose this prototype DTO.
The former `ApxmGraph`/`GraphRecorder` model used a plain
`name`/`nodes`/`edges`/`parameters`/`metadata` DTO with arbitrary operation
strings. Its cited Python and TypeScript implementation files are no longer
present in this package. That shape is not `apxm.frontend-graph` and must not
be revived as a frontend, host integration, compiler, or compatibility path.

The current executable `apxm_program` scaffold records the shallow
`apxm.frontend-graph` schema through
[`apxm_program`](../apxm_program/__init__.py). The target replacement records a
complete typed language-neutral graph containing Agent/Context/Model/Tool/
Capability/Event/Hook declarations, typed values, functions, blocks, regions,
data/control/context edges, source-semantic call intents, and source maps.

Authors never construct this graph directly. Python and TypeScript capture
ordinary typed Agent source, and Rust alone validates the graph, constructs
CFG/SSA, selects AIR/AIS operations, performs legal optimization, and builds
the artifact. FrontendGraph remains inspectable compiler input, not a second
behavior language.
