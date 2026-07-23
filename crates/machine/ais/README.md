# apxm-ais

- Current role: Rust-owned AIS operation and code-generation source
- Target contract:
  [Agent Program composition and AIR](../../../docs/agents/agent-program-composition-and-air-contract.md)
- Frontend/lowering plan:
  [Simple source-first Agent frontend](../../../docs/agents/simple-agent-authoring-frontend-plan.md)

`apxm-ais` owns the two closed canonical operation families consumed by the
compiler and runtime. No frontend, Studio component, adapter, or runtime module
may define another operation list.

## Canonical families

The effect/composition family contains exactly five operations:

1. `model.call`
2. `capability.invoke`
3. `program.new`
4. `program.invoke`
5. `await.event`

The separate compiler-emitted structural family contains functions, regions,
blocks, values, branch, switch, `ais.loop`, parallel join, try/throw/catch,
return, and yield. Structural operations are not raw public Agent builders;
`ais.loop` is not a sixth effect operation.

The source of truth is
[`src/operations/definitions.rs`](src/operations/definitions.rs). It generates
the operation catalogue and TableGen inputs consumed by the compiler. Generated
files are outputs and must not be edited to conceal source/generator drift.

## Frontend boundary

Python and TypeScript understand typed source concepts—Agent, Context, Model,
Tool/Capability, Event, Hook, and ordinary control flow—and emit FrontendGraph
intent. Rust maps that intent to AIS. Author source and public frontend packages
do not contain operation constants, raw `ais.*` kinds, or AIR/MLIR printers.

## Current completion gap

The closed operation inventory is present, but the registered semantic
TableGen operations currently expose only a token result and not the complete
typed operands required by the owner contract. The frontend plan's P1/P2 gates
complete signatures, CFG/SSA/region lowering, registered-only verification,
and end-to-end artifact/evidence correlation without adding an operation.

## Verification

```bash
dekk agents ops list
dekk agents check-frontend-codegen
```

After changing an AIS definition:

```bash
dekk agents build-dialect
dekk agents codegen
```
