# Changelog

All notable changes to APXM are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and APXM intends to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once a `1.0`
contract is committed to.

## [Unreleased]

_No unreleased changes._

## [0.0.1] - 2026-04-30 — Public release scrub

This is the first public-ready iteration of the repository.

### Added
- `VISION.md` at the repo root: APXM positioned as a library system for agent
  skills (compiled, versioned, linkable, governed), with three concrete
  proof-point workflows under `examples/python/demos/gemma4/`.
- MIT `LICENSE` at the repo root and across all crates and the Python
  frontend (the previous mixed Apache-2.0 / MIT-OR-Apache-2.0 signals are
  unified to MIT).
- `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `CHANGELOG.md`,
  `.detect-secrets.cfg`, and `.secrets.baseline` for the public release.
- Workspace `Cargo.toml` now sets `license`, `repository`, `homepage`, and
  `authors`; every crate inherits via `*.workspace = true`.

### Changed
- README and documentation lead with the fragmented-skills problem and the
  skills-as-libraries thesis. PXM theory remains first-class as the substrate
  but is now reached through `VISION.md` / `README.md` rather than being the
  entry point itself.
- `examples/python/demos/gemma4/` rewritten as "three skill-library proof
  points" — the workflow source files are unchanged in intent; only the
  framing, the README, and the `pyproject.toml` description are updated.
- `docs/design/apxm-aware-codex-skill-libraries.md` opens with the
  fragmented-skills problem before the integration scope.
