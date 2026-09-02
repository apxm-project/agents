# Compiler pipeline

The compiler has one source of truth for lowering: the typed FrontendGraph is
validated by Rust, lowered to AIR, and then rendered as canonical MLIR using
the registered AIS dialect. Native MLIR verification is an explicit toolchain
gate; the semantic AIR/artifact boundary remains backend-neutral. The emitted
artifact is executable only after exact capability and model ports are
admitted by the host.

```text
Python / TypeScript source
  -> FrontendGraph
  -> Rust validation and AIR
  -> canonical AIS MLIR lowering
  -> verified artifact
  -> exact runtime admission
```

Semantic operations are closed in
`crates/machine/ais/src/operations/definitions.rs`; the compiler consumes that
catalogue and does not define a second operation family. Canonical lowering is
implemented in `crates/compiler/pipeline/src/canonical.rs`. Structural control
flow such as loops is represented in the program/AIR layer and is not a new
effect operation.

Use the Dekk authority surface for checks:

```sh
dekk agents check-frontend-parity
dekk agents test-compiler
dekk agents test-program-source
```

For the conceptual model, read the [PXM theory](../pxm/theory.md) and the
[Agent Program composition/AIR contract](../agents/agent-program-composition-and-air-contract.md).
