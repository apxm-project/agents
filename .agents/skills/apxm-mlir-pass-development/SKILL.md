---
name: apxm-mlir-pass-development
group: Domain
description: Use when adding, modifying, or reordering MLIR passes in the APXM compiler pipeline. Enforces the single pass-list source of truth, the AIS-core ownership rule, and the build-dialect + codegen cadence after .td edits.
user-invocable: true
---

# APXM MLIR Pass Development

Load `_shared/apxm-development-rules.md` before broad work.

## Ownership

- **AIS ops are defined exclusively in `apxm-core`.** Compiler passes
  consume them; they never define them. See `_shared/apxm-development-rules.md`.
- **Canonical pass list**:
  `crates/compiler/apxm-compiler/src/passes/pipeline.rs::build_pass_list()`.
  Add or reorder passes only there. Do not register passes elsewhere.

## Cadence after edits

After editing any `.td` file or TableGen-emitted C++ shim:

```bash
dekk apxm build-dialect     # rebuild MLIR (TableGen + C++ + Rust)
dekk apxm codegen           # regenerate Python frontend bindings
dekk apxm test -p apxm-compiler   # focused test
```

After editing a pass source (without touching ops or `.td`):

```bash
dekk apxm test -p apxm-compiler
```

## Rules

- New op? Invoke `apxm-ais-op-design` first (the design-before-code
  gate) — even before adding the `.td` entry.
- New pass? Add it to `build_pass_list()`, with a one-line comment
  explaining the *why* (a real reason, never "for plan04" — see
  `_shared/apxm-agent-operating-rules.md`).
- Reordering passes? Confirm with the user — pass order has subtle
  effects on later passes and on the runtime executor.
- Attribute names go through the canonical enum in `apxm-core`. Never
  literal strings in the pass body — see
  `feedback_attribute_dual_naming` for the prior incident.

## Diagnostics

- Live op surface: `dekk apxm ops list`.
- Validate a sample graph against the new pipeline:
  `dekk apxm validate <sample.json>`.
- Critical-path / parallelism sanity: `dekk apxm analyze <sample.json>`.

## Anti-patterns

- Defining an op outside `apxm-core` because "it's only used by this
  one pass". It will leak.
- Skipping `dekk apxm codegen` after a `.td` edit and being confused by
  phantom Python frontend errors.
- Registering a pass via ad-hoc `add_pass(...)` rather than appending
  to `build_pass_list()`.
- Using literal attribute-name strings in a pass.
