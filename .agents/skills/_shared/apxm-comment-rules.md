# Shared rule — APXM comment conventions

Load this file before writing or reviewing code comments in any language.
These conventions are de-facto across the stack and enforced by reviewer
and agent discipline (no lint forces most of them). The
`apxm-comment-audit` self-hosted workflow machine-checks the
greppable subset.

## All languages

- **Doc comments are prose only.** No `@param` / `@returns` / `@note` /
  `@warning` tag soup anywhere — the codebase deliberately avoids it.
- **Inline comments justify *why*, never restate *what*.** Order,
  invariants, concurrency visibility, security rationale, guards. If the
  identifier name carries the meaning, no comment is needed.
- **Zero `TODO` / `FIXME` / `XXX` / `HACK` in first-party code.**
  Work-in-progress goes in a module-doc `Status` / `Future work` prose
  section. Vendored trees (`tools/external/*`, `external/vllm`) are
  exempt — never edit upstream markers.
- **No license / SPDX / copyright headers.**
- **No referential comments.** Never point at plans, tickets, prior
  conversations, audit artifacts, or `// see X.md`. The commit message
  owns "why now". (Worked violations to avoid: `// (see compiler-audit.md)`,
  `// see ../../SECURITY.md`.)
- **Cross-language mirror notes are required, not banned.** When a
  constant or name is duplicated across a language boundary, mark it
  (`// Mirror of <path>`); these are drift-detectable contracts.

## Rust

- Module-level `//!` header on crate roots and entry points stating
  role, contracts, and (for security-relevant modules) the threat +
  lifecycle model. This is the dominant convention, not universal —
  tests, `build.rs`, and generated files are exempt.
- `///` item docs on private as well as public items. `missing_docs` is
  not linted; this is discipline.
- `// SAFETY:` on every `unsafe` site (`unsafe_code = "deny"`).
- Line-style `//!` / `///`; block `/** */` docs are not used.

## C++ / MLIR

- `/** @file <basename> @brief <one-line> */` header — the only Doxygen
  vocabulary in use is `@file` + `@brief`. `@file` basename must match
  the filename.
- Follow the header with prose rationale and, for passes, a before/after
  IR example.
- LLVM-style `///` on internal/static helpers; `//===---===//` banner
  dividers for section breaks.
- TableGen `.td`: every `Pass<>` / `AIS_Op<>` carries a terse
  `let summary` and a multi-paragraph `let description = [{ }]` with
  numbered steps and ```mlir``` fenced examples.

## TypeScript / React

- `//` module-summary comment on line 1 describing the file's role and
  why it exists.
- `/** one-line prose */` JSDoc on exports (description-only, no tags).
- Inline `//` justifying domain guards.
