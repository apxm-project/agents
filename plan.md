# ⚠️ COMPLETED — All phases implemented as of April 6, 2026

# Plan: `apxm-frontend`

## Objective

Make Python the primary authoring frontend for APXM, generated from Rust-owned registries where practical, and retire the `.ais` DSL only after Python reaches feature parity for first-party workflows.

This version of the plan is intentionally narrower and more executable than the original draft. The original direction is good, but it bundled four separate efforts into one document:

1. Frontend product design
2. Python package migration
3. Runtime feature work
4. `.ais` removal

Those should not ship as one indivisible change.

## Product Thesis

The core bet still holds:

- LLM-authored workflows should be written in Python, not a custom DSL.
- Rust remains the source of truth for graph semantics and runtime behavior.
- The Python frontend should expose typed references for graph attributes, operations, and agent/profile metadata instead of relying on ad hoc string constants.
- The compiled graph format and runtime stay stable while the frontend evolves.

## Decisions

- Python package name: `apxm`
- New crate location: `crates/compiler/apxm-frontend/`
- Frontend layout: `rust/` for registry/codegen support, `python/` for the user-facing package
- `.ais` deprecation: yes
- `.ais` deletion: only after parity gates pass
- Generated Python modules live under `apxm/_generated/`
- Function signatures should define workflow parameters for `@compile` flows
- `g.spawn()` returning an `AgentHandle` is the right direction
- `g.team()` should be workflow-local sugar, not global config

## Non-Goals For The First Ship

These are valuable, but they should not block the initial frontend launch:

- Node-level live token streaming to `response.txt`
- Role-scoped skill injection redesign
- Workflow-local skill packaging
- Custom agent profile loading from new file formats
- Deleting every last hardcoded human-readable string in the Python package

The initial target is a typed, usable, maintainable frontend. Runtime ergonomics can follow.

## Current Facts To Plan Around

- `plan.md` is currently a draft file, not yet tracked by git.
- This checkout has `51` `.ais` examples under `examples/`, not `54`.
- `external/agentmate/` is large and appears to be its own embedded repository, so migration/deletion needs to be treated carefully.
- The workspace already has clean ownership boundaries across `apxm-core`, `apxm-cli`, `apxm-compiler`, `apxm-driver`, and `apxm-runtime`.

## Architecture Boundaries

Keep these boundaries explicit:

- `apxm-core`: shared constants, profiles, graph-facing types
- `apxm-frontend/rust`: registry facade and code generation only
- `apxm-frontend/python`: user-facing Python API
- `apxm-cli`: codegen command and packaging hooks
- `apxm-compiler` and `apxm-driver`: unchanged unless required for removing `.ais`
- `apxm-runtime`: only frontend-enabling fixes in the first pass

If a change is not required to make Python graphs compile and execute, it should not be in the critical path.

## Phased Plan

### Phase 0: Inventory And Parity Targets

Define exactly what the Python frontend must replace before `.ais` can be removed.

Tasks:

- Inventory the public surface currently used from `external/agentmate/python/`
- Inventory all first-party `.ais` examples and group them by feature
- Identify the minimum Python API needed for parity:
  - graph construction
  - parameters
  - operation nodes
  - spawn/communicate patterns
  - flow composition
- Decide whether a temporary `agentmate -> apxm` compatibility shim is needed

Exit criteria:

- A parity matrix exists for first-party examples and tests
- The migration scope is based on observed usage, not aspirational API design

### Phase 1: Create `crates/compiler/apxm-frontend/`

Stand up the crate and make code generation real before moving Python code.

Tasks:

- Add `crates/compiler/apxm-frontend/` to the workspace
- Create `rust/` crate with:
  - `lib.rs`
  - `registry.rs`
  - `codegen.rs`
- Implement a small registry facade over existing Rust-owned data:
  - graph attribute constants
  - operation specs
  - built-in agent/profile templates
- Generate Python modules into `python/apxm/_generated/`

Initial generated targets:

- `_generated/constants.py`
- `_generated/operations.py`
- `_generated/agents.py`

Recommendation:

- Defer `_generated/env.py` until the reproducibility story is clear. Generating from `~/.apxm/config.toml` makes builds user-specific and non-hermetic.

Exit criteria:

- `apxm codegen frontend` produces importable Python modules
- Generated files are deterministic for repo-owned inputs

### Phase 2: Migrate The Python Package

Move the usable Python frontend into the new crate without redesigning everything at once.

Tasks:

- Copy the Python package from `external/agentmate/` into `crates/compiler/apxm-frontend/python/apxm/`
- Rename imports from `agentmate` to `apxm`
- Replace duplicated constants with imports from `apxm._generated`
- Keep behavior stable where possible

Recommendation:

- Add a short-lived compatibility layer only if it materially lowers migration cost. Do not keep two frontends alive indefinitely.

Exit criteria:

- The package imports cleanly from its new location
- Existing Python-side tests or smoke workflows still run
- Public constants and operation names come from generated modules

### Phase 3: Add Frontend Ergonomics

After the migrated package works, add the API improvements that justify the rewrite.

Tasks:

- Make `@compile` derive parameters from the Python function signature
- Allow named parameter placeholders like `{topic}` in frontend authoring
- Lower named parameter placeholders to positional placeholders in the emitted graph
- Add `AgentHandle` as sugar over `SPAWN_AGENT + COMMUNICATE`
- Add workflow-local `Team` sugar over multiple `spawn()` calls
- Add auto-naming only for structural nodes where names are not semantically important

Guardrails:

- These features must lower to the existing graph/runtime model
- The frontend should not invent new runtime semantics in this phase

Open question:

- Named node references in templates are attractive, but they also hide edge creation. Treat `{node_name}` auto-wiring as a separate sub-phase and prove it with tests before making it the default pattern.

Exit criteria:

- Representative workflows compile to valid graphs
- New sugar produces the same graph shape as manual construction
- Failure modes are explicit for unknown placeholder names

### Phase 4: CLI And Build Integration

Only integrate into the main build once codegen is stable.

Tasks:

- Add `apxm codegen frontend` to `apxm-cli`
- Decide whether `dekk apxm build` should always run codegen or only when asked
- Document the bootstrap flow for local development and CI

Recommendation:

- Start with an explicit codegen command.
- Add automatic build hooks only after idempotence and failure behavior are well understood.

Exit criteria:

- A fresh checkout can generate and import the frontend reliably
- CI behavior is deterministic

### Phase 5: Migrate Examples And Docs

Do not delete `.ais` assets until the Python path is demonstrated on real examples.

Tasks:

- Convert a representative subset of `.ais` examples first
- Cover the feature matrix, not just the easiest examples
- Update docs to teach Python first
- Keep `.ais` examples temporarily as a validation oracle

Recommendation:

- Convert the examples in tiers:
  - Tier 1: core graph construction and parameters
  - Tier 2: multi-agent and ACP flows
  - Tier 3: composition and advanced patterns

Exit criteria:

- Python replacements exist for all important example categories
- Docs no longer rely on `.ais` for the primary user journey

### Phase 6: Remove `.ais`

Delete the DSL only after the Python frontend is established.

Tasks:

- Remove the C++ parser under `crates/compiler/apxm-compiler/mlir/lib/Parser/`
- Remove DSL C API entry points
- Remove Rust DSL entry points and FFI exports
- Remove `.ais` dispatch from the driver
- Remove `.ais` examples once Python replacements are present
- Clean up docs and tests

Exit criteria:

- No first-party docs, tests, CLI flows, or examples depend on `.ais`
- The workspace builds and tests pass without parser code

### Phase 7: Runtime Follow-Ups

These should be tracked as follow-on work items, not prerequisites for the frontend migration.

Candidate items:

- Auto-detect ACP protocol for spawned agents
- Wire `ProfileRouter` if that path is already implemented but dormant
- Stream node output incrementally to `response.txt`
- Add role-aware skill resolution and workflow-local skill copying

These are worthwhile, but they belong in separate design docs or follow-up tickets because they change runtime behavior independently of the frontend migration.

## Key Risks

### Risk 1: Deleting `.ais` Too Early

If `.ais` is removed before Python parity is proven, the repo loses its fallback authoring path and the migration becomes harder to validate.

Mitigation:

- Keep deletion at the end
- Require example and doc parity before removal

### Risk 2: Non-Hermetic Codegen

Generating frontend artifacts from `~/.apxm/config.toml` or other user-local state makes builds differ by machine.

Mitigation:

- Split generated artifacts into:
  - repo-owned deterministic outputs
  - optional local overlays
- Do not make local config generation a required build step until CI semantics are defined

### Risk 3: Overreaching On “Zero Hardcoded Strings”

That slogan is directionally good but technically too broad. Some strings are identifiers, some are user-facing labels, and some are unavoidable protocol values.

Mitigation:

- Narrow the requirement:
  - no duplicated graph attribute keys
  - no duplicated operation identifiers
  - no duplicated built-in agent/profile identifiers in the public API

### Risk 4: Hiding Too Much Graph Structure

Named template references that auto-create edges improve ergonomics, but they can also make graph construction less explicit and harder to debug.

Mitigation:

- Ship parameter-name lowering first
- Ship node-name auto-wiring only after tests and clear diagnostics exist

### Risk 5: Treating Runtime Enhancements As Frontend Work

Streaming, skills isolation, and routing changes expand the blast radius and make the project harder to land.

Mitigation:

- Split them into follow-up tracks unless a concrete frontend requirement proves otherwise

## Acceptance Criteria

The frontend migration is successful when all of the following are true:

- `crates/compiler/apxm-frontend/` exists and is part of the workspace
- `apxm codegen frontend` generates importable Python modules for Rust-owned registries
- `apxm` replaces the needed `agentmate` functionality for first-party workflows
- Function-signature parameters work for `@compile` flows
- `AgentHandle` and `Team` compile to the existing graph/runtime model
- Python examples cover the same major feature categories currently exercised by `.ais`
- Docs teach Python as the primary path
- `.ais` can be removed without leaving first-party gaps

## Verification

### Frontend

1. `dekk apxm build`
2. `python -c "from apxm._generated.constants import MODEL; print(MODEL)"`
3. `python -c "from apxm._generated.operations import ASK; print(ASK.op)"`
4. `python -c "from apxm._generated.agents import claude; print(claude.name)"`

### Graph Construction

5. A minimal `@compile` workflow with `topic: str` emits a graph parameter named `topic`
6. Named parameter placeholders lower to positional placeholders in emitted graph JSON
7. `g.spawn(...).ask(...)` emits the expected `SPAWN_AGENT` and `COMMUNICATE` nodes
8. Repeated `handle.ask(...)` calls create the expected control sequencing

### Migration

9. Representative Python examples execute successfully through the normal CLI path
10. The parity matrix for first-party examples is complete

### Removal

11. No `.ais` examples remain once Python replacements are present
12. Workspace builds and tests pass after parser removal

## Bottom Line

The strategy is sound, but the order matters.

The right sequence is:

1. stand up codegen
2. migrate Python
3. add ergonomics
4. prove parity
5. delete `.ais`
6. treat runtime enhancements as separate follow-up work

That turns the plan from a broad manifesto into something the repo can absorb incrementally.
