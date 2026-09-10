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

Typed entrypoints can carry a compiler-owned `input_schema` in the
FrontendGraph and the digest-bound executable artifact. The source schema
generates both frontend record types; the shared `apxm-program` contract
validates the closed schema before lowering and when an artifact is decoded.
Consumers use that exact artifact contract rather than interpreting type names.

TypeScript projects resolved object types, arrays, strings, numbers, booleans,
null and optional properties. Python projects `TypedDict` (including
`Required`/`NotRequired`), `list[T]`, `str`, `float`, `int`, `bool` and `NoneType`.
Objects reject undeclared fields. Recursive, unresolved, dynamic, union,
tuple and index-signature types do not produce a schema; a nested unsupported
type makes the complete input schema unavailable. A missing schema is not an
open schema and cannot authorize schema-bound input admission. Other existing
compile paths remain compatible. Schema projection is bounded to 32 nested
levels and 512 type nodes.

Named pure data uses the existing value-assembly contract, not another effect
operation. TypeScript `const` declarations and single-assignment Python locals
can map immutable input or prior results into static JSON objects and arrays.
They stay inside the block where they are declared, cannot be mutated or
reassigned, and cannot capture mutable Context. The compiler refuses unknown
variables instead of interpreting source in the consumer.

Use the Dekk authority surface for checks:

```sh
dekk agents check-frontend-parity
dekk agents test-compiler
dekk agents test-program-source
```

For the conceptual model, read the [PXM theory](../pxm/theory.md) and the
[Agent Program composition/AIR contract](../agents/agent-program-composition-and-air-contract.md).
