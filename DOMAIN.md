# APXM — domain overview

A one-page orientation for coding agents and humans dropping into this
repository for the first time. For the agent-facing SSOT, read
[`AGENTS.md`](AGENTS.md) or [`CODEX.md`](CODEX.md) (same body; portable /
Codex-named) or [`CLAUDE.md`](CLAUDE.md) (Claude Code). They are generated
from [`.agents/project.md`](.agents/project.md).

## What APXM is

A graph-aware **dispatch + scheduling layer** for vLLM, with an
AMD-aligned **CPU/GPU split**. Planning, validation, and analysis run
on CPU; inference runs on GPU. The public surface is an MLIR dialect
(AIS) plus a Rust runtime plus a vLLM fork that accepts dispatch
hints. Not an "agent framework", not an "LLM orchestrator".

## Where to read more

- [`VISION.md`](VISION.md) — project positioning.
- [`docs/README.md`](docs/README.md) — docs landing.
- [`docs/pxm/readme.md`](docs/pxm/readme.md) — PXM theory and history.
- [`docs/pxm/foundations.md`](docs/pxm/foundations.md),
  [`processes.md`](docs/pxm/processes.md),
  [`scheduling.md`](docs/pxm/scheduling.md) — design layers.
- [`docs/compiler/pipeline.md`](docs/compiler/pipeline.md) — the pass
  pipeline.
- [`docs/backends/storage-layout.md`](docs/backends/storage-layout.md)
  — where APXM puts large files (HF cache, image store, artifacts).
- [`docs/backends/model-zoo.md`](docs/backends/model-zoo.md) — vLLM
  zoo reference;
  [`docs/backends/model-zoo-quickstart.md`](docs/backends/model-zoo-quickstart.md)
  for the 15-minute walkthrough.
- [`docs/external-vllm-fork.md`](docs/external-vllm-fork.md) — the
  fork integration contract.
- `apxm-project/apxm-eval` — preregistrations, evaluation harness, claim
  cards, paper drafts, and paper-bound evidence.

## Phase status

Current phase status is tracked in agent memory
(`apxm_phase_status`), not here, because it changes monthly. As of
this writing: Phase 1 IR is shipped (with caveats); Phase 2+ not
started; capability negotiation is cosmetic.

## Lifecycle for agent contributions

Run the 6 lifecycle skills in order — see [`AGENTS.md`](AGENTS.md)
§3:

`apxm-context → apxm-plan → apxm-execute-plan → apxm-simplify →
apxm-finish → apxm-commit`
