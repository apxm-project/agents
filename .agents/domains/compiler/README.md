# Domain — compiler

AIS dialect, MLIR passes, Python frontend codegen.

## Skills

- **ais-op-design** — design-before-code gate for new AIS ops.
- **frontend-implementation** — Rust/TypeScript/Python frontend lowering,
  op-catalog consumption, and the single Rust AIR printer boundary.
- **mlir-pass-development** — add/modify MLIR passes.
- **compile-and-execute** — validate, compile, run `.apxmobj`.

## Ownership

- **`apxm-ais`** defines AIS ops
  (`crates/machine/ais/src/operations/definitions.rs`). No exceptions.
- **Canonical pass list**: `crates/machine/ais/src/passes/mod.rs`.
  `crates/compiler/pipeline/build.rs` generates the TableGen and C-API dispatch
  from it; the compiler crate registers no passes of its own.
- **Attribute names** go through the canonical enum in `apxm-ais` —
  never literal strings in Python/MLIR/Rust. See
  `feedback_attribute_dual_naming`.

## Cadence after `.td` or TableGen-shim edits

```bash
dekk agents build-dialect   # rebuild MLIR (TableGen + C++ + Rust)
dekk agents codegen         # regenerate Python frontend bindings
dekk agents test-compiler
```

## Docs

- `docs/compiler/pipeline.md` — the pass pipeline.
- `crates/machine/ais/src/passes/mod.rs` — pass specs (source of truth).
- `crates/compiler/pipeline/src/canonical.rs` — canonical AIR → AIS MLIR.
- `crates/compiler/frontend/python/` — Python frontend.
- `crates/compiler/frontend/typescript/` — TypeScript frontend.

## Related rules

- `_shared/apxm-development-rules.md`
