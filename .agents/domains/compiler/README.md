# Domain — compiler

AIS dialect, MLIR passes, Python frontend codegen.

## Skills

- **apxm-ais-op-design** — design-before-code gate for new AIS ops.
- **apxm-mlir-pass-development** — add/modify MLIR passes.
- **apxm-compile-and-execute** — validate, compile, run `.apxmobj`.

## Ownership

- **`apxm-core`** defines AIS ops. No exceptions.
- **Canonical pass list**:
  `crates/compiler/apxm-compiler/src/passes/pipeline.rs::build_pass_list()`.
- **Attribute names** go through the canonical enum in `apxm-core` —
  never literal strings in Python/MLIR/Rust. See
  `feedback_attribute_dual_naming`.

## Cadence after `.td` or TableGen-shim edits

```bash
dekk apxm build-dialect   # rebuild MLIR (TableGen + C++ + Rust)
dekk apxm codegen         # regenerate Python frontend bindings
dekk apxm test -p apxm-compiler
```

## Docs

- `docs/compiler/pipeline.md` — the pass pipeline.
- `crates/compiler/apxm-compiler/src/passes/` — pass implementations.
- `crates/compiler/apxm-frontend/python/` — Python frontend.

## Related rules

- `_shared/apxm-development-rules.md`
