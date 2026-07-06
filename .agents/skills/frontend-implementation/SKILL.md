---
name: frontend-implementation
group: Domain
description: Use when changing APXM compiler frontends: Rust AirModule, TypeScript @apxm/frontend, Python apxm, frontend codegen, or Studio/source lowering into AIR.
user-invocable: true
---

# APXM Frontend Implementation

Load `_shared/apxm-development-rules.md` before broad work. When writing code
load `_shared/apxm-comment-rules.md`; when writing tests load
`_shared/apxm-test-rules.md`.

## Ownership

- The AIS op catalog is the single source of operations:
  `crates/machine/ais/generated/op-spec.v1.json`, generated from Rust-owned
  AIS definitions.
- The Rust compiler owns AIR text printing:
  `crates/compiler/pipeline/src/air_builder/`.
- TypeScript and Python frontends own authoring ergonomics, graph recording,
  validation, and handler sidecars. They must lower to the compiler
  `FrontendGraph`/`AirModule` shape before AIR text exists.
- Studio and other authoring surfaces generate TypeScript or Python frontend
  source, then invoke the frontend path. They do not keep private AIR emitters.

## Rules

- Every supported frontend must produce AIR: Rust, TypeScript, and Python.
- Frontends consume the op catalog; no local op lists, aliases, retired names,
  or compatibility vocabularies.
- No direct MLIR string printers in TypeScript or Python. The only AIR text
  assembly path is Rust `AirModule::to_air()` / `AirProgram::to_air()`.
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
   - Rust printer / validation: `crates/compiler/pipeline/src/air_builder/**`
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

- Rust AIR builder changed: `dekk agents test -p apxm-compiler`.
- TypeScript frontend changed:
  `npm --prefix crates/compiler/frontend/typescript run typecheck` and
  `npm --prefix crates/compiler/frontend/typescript test`.
- Python frontend changed: `dekk agents test-python-frontend`.
- Op catalog/codegen changed: `dekk agents check-frontend-codegen`.
- CLI frontend lowering changed: `dekk agents test-cli`.

## Anti-patterns

- A second TS/Python AIR printer that formats `ais.*` MLIR directly.
- A Studio-only AIR emitter or widget bridge that bypasses `@apxm/frontend`.
- JSON input treated as executable graph source; JSON is data/DTO, not a
  runtime input format.
- Duplicating operation metadata because importing generated metadata is
  inconvenient.
- Comments that explain history instead of the invariant the code enforces.

## See also

- `ais-op-design` for operation changes.
- `mlir-pass-development` for compiler pass changes.
- `compile-and-execute` for `.air`/`.apxmobj` workflows.
