---
name: apxm-ais-op-design
description: Use before adding or modifying an AIS op in apxm-core. Enforces the design-before-code gate, the canonical-attribute rule, the definitions.rs source-of-truth layer map, and the build-dialect + codegen cadence.
user-invocable: true
---

# APXM AIS Op Design

The AIS dialect is the public IR contract — compiler passes, runtime
handlers, the Python frontend, and the vLLM fork all consume it.
**Every other crate consumes; only `apxm-core` defines.**

Load `_shared/apxm-development-rules.md` before broad work.

## When this skill is required

- Adding a new AIS op.
- Renaming, removing, or changing the type signature of an existing op.
- Adding or renaming an op attribute.
- Anything that would change `dekk apxm ops list` output.

## Design-before-code gate

Before editing `definitions.rs`, answer in writing (in the plan):

1. **Why an op?** What can't be expressed by composing existing ops?
   Three composed ops beat a premature new op.
2. **Op vs. compose.** Will every consumer need it, or only one pass?
   If only one — keep it a composition.
3. **Type signature.** Operands/results, and which attributes it carries.
4. **Attribute naming.** Every attribute name is an `attrs.rs` constant
   added to `ALL_ATTR_NAMES`. No string literals in spec/TableGen/handler.
5. **Frontend / runtime / vLLM impact.** What `dekk apxm validate` must
   accept; which runtime handler owns dispatch; whether it crosses
   `/v1/apxm/*` (if so coordinate with `apxm-fork-vllm-rebase`).

Get user sign-off on the design before any code change.

## Source of truth and what's generated

`apxm-core/src/operations/definitions.rs` is the source of truth. Two
layers are **generated from it — never hand-edit**:

- the C++ `OperationKind` enum + lowering `TypeSwitch` cases
  (`*.generated.inc`, from `WIRE_INDEXED_OPERATIONS` via `artifact_wire.rs`);
- the Python frontend bindings (via `dekk apxm codegen`).

The MLIR dialect file `AISOps.td` is a **hand-maintained mirror**, not
generated — its `arguments` must match the spec's attribute fields by name.

## Layers to edit, in order (a new op)

1. `apxm-core/src/operations/definitions.rs`:
   - add the `AISOperationType` variant;
   - append `(N, AISOperationType::Yours)` to `WIRE_INDEXED_OPERATIONS`
     (next free index — **append-only; index 30 is reserved**);
   - add the `OperationSpec` entry to `AIS_OPERATIONS` (category, field
     schema, latency, `example_json`, emission spec).
   The exhaustive matches (`Display`, `FromStr`, `mlir_mnemonic`,
   `to_tablegen_name`) **fail to compile** until you add each arm — let
   the compiler drive them.
2. `apxm-core/src/operations/attrs.rs`: add each attribute-name const and
   list it in `ALL_ATTR_NAMES`.
3. `mlir/include/ais/Dialect/AIS/IR/AISOps.td`: hand-write the `AIS_Op`
   def; its `arguments` mirror the spec fields by the same attr names
   (typed `OptionalAttr<...>`, not bare).
4. `crates/runtime/apxm-runtime/src/executor/handlers/<op>.rs` (+ `pub mod`
   in `handlers/mod.rs`) and a dispatch arm in
   `executor/dispatcher.rs` (`match node.op_type`). Lowering in
   `mlir/lib/Dialect/AIS/Conversion/Artifact/ArtifactEmitter.cpp` is
   usually untouched — its cases are generated.
5. Consumers (`apxm-studio/src/air.rs`, prompt-as-workflow catalog) only
   if the op is author-facing.

## Build + verify cadence

```bash
dekk apxm build-dialect   # after any .td or C++ shim edit
dekk apxm codegen         # regenerate frontend bindings (do BEFORE testing)
dekk apxm test -p apxm-core -p apxm-compiler -p apxm-runtime
dekk apxm test-python-frontend
dekk apxm ops list | grep <new-op>
```

The invariant tests are the gate: `test_operation_counts`,
`test_all_ops_have_specs`, `mlir_tablegen_declares_all_rust_operations`,
and the spec-field ↔ TableGen-attr parity test.

## Rules & anti-patterns

- Add only to `apxm-core`; never define an op elsewhere.
- Attribute names are `attrs.rs` constants — never literals.
- No referential comments in `.td` (see `_shared/apxm-agent-operating-rules.md`).
- Don't skip `build-dialect`/`codegen` — stale bindings cause phantom
  frontend errors.
- Don't add an op for "future flexibility" with no current consumer.
- Pass-list edits (`build_pass_list`) apply **only** if the op also ships
  a new pass — not a normal add-op step.

## Self-hosted workflow

`dekk apxm execute examples/python/self-hosted/add_op.py` runs the add-op
procedure (architect → parallel compiler/runtime impl → reviewer).
Execute, then verify against this skill. See `_shared/apxm-self-host-rules.md`.
