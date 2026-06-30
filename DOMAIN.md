# agents — domain overview

A one-page orientation for coding agents and humans dropping into this
repository for the first time. For the agent-facing SSOT, read
[`AGENTS.md`](AGENTS.md) or [`CODEX.md`](CODEX.md) (same body; portable /
Codex-named) or [`CLAUDE.md`](CLAUDE.md) (Claude Code). They are generated
from [`.agents/project.md`](.agents/project.md).

## What agents is

The APXM abstract-machine repo: AIS dialect, compiler, runtime, capability
contracts, context handling, permissions, orchestration, CLI, and the
profile-backed agent execution path. vLLM scheduling remains one backend path,
not the repo identity. This repo is not the `apxm` coordinator, `server`,
`os`, `auth`, or `studio`.

## Where to read more

- [`VISION.md`](VISION.md) — project positioning.
- [`docs/README.md`](docs/README.md) — docs landing.
- [`docs/pxm/readme.md`](docs/pxm/readme.md) — PXM theory and history.
- [`docs/pxm/foundations.md`](docs/pxm/foundations.md),
  [`docs/pxm/processes.md`](docs/pxm/processes.md),
  [`docs/pxm/scheduling.md`](docs/pxm/scheduling.md) — design layers.
- [`docs/compiler/pipeline.md`](docs/compiler/pipeline.md) — the pass
  pipeline.
- [`docs/backends/storage-layout.md`](docs/backends/storage-layout.md)
  — where APXM puts large files (HF cache, image store, artifacts).
- [`docs/backends/model-zoo.md`](docs/backends/model-zoo.md) — vLLM
  zoo reference;
  [`docs/backends/model-zoo-quickstart.md`](docs/backends/model-zoo-quickstart.md)
  for the 15-minute walkthrough.
- [`docs/vllm-fork.md`](docs/vllm-fork.md) — the
  fork integration contract.
- `apxm-project/eval` — preregistrations, evaluation harness, claim
  cards, paper drafts, and paper-bound evidence.

## Phase status

Current phase status is tracked in agent memory
(`apxm_phase_status`), not here, because it changes monthly. As of
this writing: Phase 1 IR is shipped (with caveats); Phase 2+ not
started; capability negotiation is cosmetic.

## Lifecycle for agent contributions

Run the 6 lifecycle skills in order — see [`AGENTS.md`](AGENTS.md)
§3:

`context → plan → execute-plan → simplify →
finish → commit`
