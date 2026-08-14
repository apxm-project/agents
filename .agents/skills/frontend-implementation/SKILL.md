---
name: frontend-implementation
group: Domain
description: Use when changing APXM compiler frontends: Rust FrontendGraph lowering, TypeScript @apxm/frontend, Python apxm_program, frontend codegen, or downstream source lowering into AIR.
user-invocable: true
---

# APXM Frontend Implementation

Load `_shared/apxm-development-rules.md` before broad work. When writing code
load `_shared/apxm-comment-rules.md`; when writing tests load
`_shared/apxm-test-rules.md`.

## Ownership

- The AIS op catalog is the single source of operations:
  `crates/machine/ais/generated/op-spec.json`, generated from Rust-owned
  AIS definitions.
- The Rust compiler owns canonical FrontendGraph validation and AIR lowering in
  `crates/machine/program/src/lower.rs`.
- TypeScript and Python frontends own authoring ergonomics, graph recording,
  validation, and handler sidecars. They submit FrontendGraph to the compiler
  before AIR exists.
- Downstream authoring surfaces generate TypeScript or Python frontend source,
  then invoke the frontend path. They do not keep private AIR emitters,
  alternate target binders, or product-specific lowering paths.

## Rules

- Every supported frontend must produce FrontendGraph: Rust, TypeScript, and Python.
- Frontends consume the op catalog; no local op lists, aliases, retired names,
  or compatibility vocabularies.
- No direct MLIR string printers or raw AIR builders in TypeScript or Python.
  Rust lowers verified FrontendGraph through `apxm_program::lower`.
- Keep graph DTOs plain JSON: `name`, `nodes`, `edges`, `parameters`,
  `metadata`; node attributes are JSON-plain values.
- Handler sidecars are metadata comments only at the process boundary; strip
  them before MLIR parse and store them in artifact sections.
- Do not add compatibility/fallback paths for old source formats. Migrate the
  caller to Rust/TS/Python frontend source or canonical `.air`.
- Do not mention removed planning labels, stale acronyms, or prior migration
  terms in code comments. State the runtime invariant directly.

## Standard workflow

1. Identify which frontend surface owns the change:
   - Rust validation/lowering: `crates/machine/program/src/{frontend_graph,lower}.rs`
   - TypeScript frontend: `crates/compiler/frontend/typescript/**`
   - Python frontend: `crates/compiler/frontend/python/**`
   - CLI source lowering: `crates/tools/cli/src/commands/compile.rs`
2. If the AIS op surface changes, invoke `ais-op-design` first, then run
   `dekk agents build-dialect` and `dekk agents codegen`.
3. Preserve the shared graph DTO contract across Rust, TypeScript, and Python.
   Add mirror tests when one language's serialized shape changes.
4. Route AIR generation through the Rust printer. Delete local printer code
   once callers have moved.
5. Keep public commands under Dekk; add a `.dekk.toml` command if a new normal
   workflow is needed.

## Verification

- Rust FrontendGraph lowering changed: `dekk agents test -p apxm-program`.
- TypeScript frontend changed:
  `npm --prefix crates/compiler/frontend/typescript run typecheck` and
  `npm --prefix crates/compiler/frontend/typescript test`.
- Python frontend changed: `dekk agents test-python-frontend`.
- Op catalog/codegen changed: `dekk agents check-frontend-codegen`.
- CLI frontend lowering changed: `dekk agents test-cli`.

## Anti-patterns

- A second TS/Python AIR printer that formats `ais.*` MLIR directly.
- A downstream-only AIR emitter or private bridge that bypasses
  `@apxm/frontend`.
- JSON input treated as executable graph source; JSON is data/DTO, not a
  runtime input format.
- Duplicating operation metadata because importing generated metadata is
  inconvenient.
- Comments that explain history instead of the invariant the code enforces.

## See also

- `ais-op-design` for operation changes.
- `mlir-pass-development` for compiler pass changes.
- `compile-and-execute` for `.air`/`.apxmobj` workflows.
