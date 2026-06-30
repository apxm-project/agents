# Domain — compiler

AIS dialect, MLIR passes, Python frontend codegen.

## Skills

- **ais-op-design** — design-before-code gate for new AIS ops.
- **mlir-pass-development** — add/modify MLIR passes.
- **compile-and-execute** — validate, compile, run `.apxmobj`.

## Ownership

- **`apxm-core`** defines AIS ops. No exceptions.
- **Canonical pass list**:
  `crates/compiler/compiler/src/passes/pipeline.rs::build_pass_list()`.
- **Attribute names** go through the canonical enum in `apxm-core` —
  never literal strings in Python/MLIR/Rust. See
  `feedback_attribute_dual_naming`.

## Cadence after `.td` or TableGen-shim edits

```bash
dekk agents build-dialect   # rebuild MLIR (TableGen + C++ + Rust)
dekk agents codegen         # regenerate Python frontend bindings
dekk agents test -p apxm-compiler
```

## Docs

- `docs/compiler/pipeline.md` — the pass pipeline.
- `crates/compiler/compiler/src/passes/` — pass implementations.
- `crates/compiler/frontend/python/` — Python frontend.

## Related rules

- `_shared/apxm-development-rules.md`
