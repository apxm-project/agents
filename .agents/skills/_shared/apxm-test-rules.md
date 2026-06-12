# Shared rule — APXM test authoring

Load this file before adding or changing tests. This rule owns *how to
write* tests; `_shared/apxm-development-rules.md` owns *how to run* them
(authority CLI, build env). Run targeted tests first, the full suite
before claiming done.

## Placement & naming

- **Rust unit tests** live in a `#[cfg(test)] mod tests` block beside the
  code under test. **Integration tests** live in the crate's `tests/`
  dir, one file per surface.
- **Smoke tests follow the established shape.** A new backend adds
  `tests/backends/<provider>_smoke.rs`; a new op/pass adds a focused
  integration test next to its siblings — generalize the existing
  pattern, do not invent a new layout.
- **Python tests** live in `tests/` beside the package
  (`tools/<pkg>/tests/`), named `test_<unit>.py`.
- **Mirror contracts get a mirror test.** Anything marked
  `// Mirror of <path>` needs a test that fails when the two sides drift
  (e.g. `test_keys_match_rust.py`).

## What to pin

- When you add an **AIS op**, **backend**, or **MLIR pass**, pin: the
  happy-path lowering/execution, one rejection path, and the
  serialized-shape stability (round-trip parse) where applicable.
- Pin observable contracts (env-var names, route paths, response
  markers) against the constant, never a string literal.

## Conventions

- Test files carry the same module-doc / docstring discipline as
  non-test code (`_shared/apxm-comment-rules.md`): a one-line statement
  of what surface the file covers.
- Standardize the Python test bootstrap in a per-package `conftest.py`
  rather than scattering `sys.path.insert` / `# noqa: E402` across test
  files.
- No network or GPU in unit tests; gate those behind integration markers
  the suite can skip.

## Dogfood

Test-failure triage is a self-hosted workflow: the audit graph runs the
suite, clusters failures, and fans out a fixer per cluster
(`_shared/apxm-self-host-rules.md`). Prefer running that over hand-triaging
a large failure set.
