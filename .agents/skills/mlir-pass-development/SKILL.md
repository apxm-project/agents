---
name: mlir-pass-development
group: Domain
description: Use when adding, modifying, or reordering MLIR passes in the APXM compiler pipeline. Enforces the single pass-list source of truth in apxm-ais, the AIS ownership rule, and the build-dialect + codegen cadence after .td edits.
user-invocable: true
---

# APXM MLIR Pass Development

Load `_shared/apxm-development-rules.md` before broad work.

## Ownership

- **AIS ops are defined exclusively in `apxm-ais`**
  (`crates/machine/ais/src/operations/definitions.rs`). Compiler passes consume
  them; they never define them. See `_shared/apxm-development-rules.md`.
- **Canonical pass list**: `crates/machine/ais/src/passes/mod.rs` — the
  `PassSpec` constants and the `AIS_PASSES` / `ALL_PASSES` slices. Add or
  reorder passes only there. Do not register passes in the compiler crate.
- `crates/compiler/pipeline/build.rs` consumes those specs at build time
  (`generate_passes_tablegen`, `generate_pass_dispatch`,
  `generate_pass_descriptors`) to emit `Passes.generated.td`,
  `PassDispatch.inc`, and `PassDescriptors.inc`. Editing the generated files is
  editing build output.

## Cadence after edits

After editing any `.td` file or TableGen-emitted C++ shim:

```bash
dekk agents build-dialect     # rebuild MLIR (TableGen + C++ + Rust)
dekk agents codegen           # regenerate Python frontend bindings
dekk agents test-compiler     # focused test
```

After editing a pass source (without touching ops or `.td`):

```bash
dekk agents test-compiler
```

There is no bare `dekk agents test -p <crate>` scope. Use the named per-crate
recipe.

## Rules

- New op? Invoke `ais-op-design` first (the design-before-code
  gate) — even before adding the `.td` entry.
- New pass? Add a `PassSpec` in `crates/machine/ais/src/passes/mod.rs` and list
  it in `AIS_PASSES` and `ALL_PASSES`, with a one-line comment explaining the
  real invariant it preserves.
- Reordering passes? Confirm with the user — pass order has subtle
  effects on later passes and on the runtime executor.
- Attribute names go through the canonical enum in `apxm-ais`. Never
  literal strings in the pass body — see
  `feedback_attribute_dual_naming` for the prior incident.

## Diagnostics

- Live op surface: `dekk agents ops list`.
- Validate a sample graph against the new pipeline:
  `dekk agents validate <sample.json>`.
- Critical-path / parallelism sanity: `dekk agents analyze <sample.json>`.

## Anti-patterns

- Defining an op outside `apxm-ais` because "it's only used by this
  one pass". It will leak.
- Skipping `dekk agents codegen` after a `.td` edit and being confused by
  phantom Python frontend errors.
- Registering a pass in the compiler crate, or editing the generated
  `Passes.generated.td` / `PassDispatch.inc` / `PassDescriptors.inc`, rather
  than adding a `PassSpec` in the AIS owner.
- Using literal attribute-name strings in a pass.
