# External vLLM Fork

This document captures the current APXM vLLM integration reality in this
repository. It exists because several older plans, helper scripts, and Python
prototype docs still describe an API shape that does not match the live fork in
`external/vllm`.

## Canonical Source Of Truth

The forked vLLM checkout in this repo is the canonical source of truth:

- Submodule path: `external/vllm`
- Remote: `https://github.com/randreshg/vllm.git`
- Expected branch: `apxm`

If APXM is meant to talk to graph-aware vLLM, it should talk to that checkout or
to a server launched from that checkout. Hidden copies under `/tmp` are useful
for diagnosis only; they are not the intended long-term operator workflow.

## Containerized Direct-Mount Rule

If you build or run the fork inside Docker, mount the full APXM repo and work
from the mounted `external/vllm` subdirectory.

Why:

- `external/vllm` is a git submodule, not a standalone repo checkout
- its `.git` metadata points back into the parent repo's `.git/modules/...`
- the fork uses `setuptools_scm` for versioning during editable installs

So this is the correct shape:

- mount `/path/to/apxm` into the container
- use `/mounted/apxm/external/vllm` as the working directory
- keep the editable install and `.venv` under that mounted checkout

This is the wrong shape for the final operator workflow:

- mounting only `external/vllm`
- copying the source into `/tmp`
- installing from a hidden container-only clone that the user cannot inspect

## What The Fork Actually Exposes

The current fork exposes three APXM-specific OpenAI-side endpoints:

- `POST /v1/apxm/graphs/register`
- `GET /v1/apxm/graphs/{graph_id}`
- `DELETE /v1/apxm/graphs/{graph_id}`

The router lives in `external/vllm/vllm/entrypoints/openai/apxm/api_router.py`
and is mounted from
`external/vllm/vllm/entrypoints/openai/api_server.py`.

Per-request APXM scheduling hints are sent inside OpenAI-compatible request
bodies via `extra_body.apxm` or related flattened `apxm_*` fields. The fork uses
those request hints directly for graph-aware scheduling and KV-retention
behavior.

## Build And Install Rules

When working inside the fork, follow `external/vllm/AGENTS.md`:

- use `uv`
- use `.venv/bin/python`
- do not use bare `pip`
- install with `uv pip install -e . --torch-backend=auto`
- if the change is Python-only, prefer
  `VLLM_USE_PRECOMPILED=1 uv pip install -e . --torch-backend=auto`

APXM now exposes a maintained top-level entrypoint for that install and serve
path:

- `dekk apxm vllm install`
- `dekk apxm vllm serve <HF_MODEL_ID>`

The fork still remains an independently managed visible checkout with its own
`uv` plus `.venv` lifecycle under `external/vllm`.

## APXM Toolchain Note

APXM itself also depends on the repo-local `.dekk/env` toolchain:

- `.dekk/env` is the canonical MLIR and LLVM prefix for this repo
- APXM compile and execute paths should discover that prefix even when the
  outer shell has not activated it manually
- if `apxm-compiler` was previously built before MLIR was visible, rebuild it
  from this repo after `.dekk/env` exists so it does not keep stale stub
  bindings

The intended operator path is still the visible repo:

- APXM compiler and runtime from this repo
- MLIR from this repo's `.dekk/env`
- graph-aware vLLM from this repo's `external/vllm`

## Current Documentation Drift

The following surfaces are stale and should not be treated as operational truth:

- `crates/runtime/apxm-backends/python/apxm_vllm/`
  - Historical Python prototype package, not the current `external/vllm` fork
    contract.

- `docs/planning/specs/2026-04-21-apxm-vllm-killer-demo-design.md`
- `docs/planning/plans/2026-04-21-apxm-vllm-killer-demo.md`
  - These documents describe an earlier four-endpoint design with
    `/v1/apxm/pins` and `/v1/apxm/pins/stats`.
  - The current fork does not expose those endpoints.

The following surfaces were updated to match the fork-local workflow:

- `.dekk.toml`
  - The optional `vllm` install component now runs
    `bash tools/scripts/install_external_vllm.sh`.
- `tools/scripts/vllm.py`
  - The maintained dekk-facing fork commands now live here:
    `dekk apxm vllm install` and `dekk apxm vllm serve <HF_MODEL_ID>`.

## Current Integration Mismatch

The Rust backend migration is now aligned to the live fork contract in
`crates/runtime/apxm-backends/src/llm/backends/vllm/backend.rs`:

- it registers graphs with `POST /v1/apxm/graphs/register`
- it probes graph-status availability with `GET /v1/apxm/graphs/{graph_id}`
- it releases graphs with `DELETE /v1/apxm/graphs/{graph_id}`
- it relies on per-request `extra_body.apxm.pin_policy` hints instead of an
  out-of-band pin endpoint

That matches the live fork, which drives graph-aware behavior from:

- graph registration via `POST /v1/apxm/graphs/register`
- graph inspection via `GET /v1/apxm/graphs/{graph_id}`
- graph release via `DELETE /v1/apxm/graphs/{graph_id}`
- per-request `extra_body.apxm.pin_policy` hints

The main remaining drift is documentation, benchmark planning language, and the
historical Python prototype package under
`crates/runtime/apxm-backends/python/apxm_vllm/`.

Any future runtime or benchmark work should be aligned to that contract.

## Canonical Operator Path

For the simplest repo-local operator flow:

1. Install APXM with dekk.
2. Install the visible fork: `dekk apxm vllm install`
3. Serve the exact model you want: `dekk apxm vllm serve <HF_MODEL_ID>`
4. Register the endpoint in APXM, for example:
   `dekk apxm backend add vllm-fork --type onprem --protocol vllm --endpoint http://127.0.0.1:8916/v1`
5. Register the exact served model id:
   `dekk apxm backend add-model vllm-fork <HF_MODEL_ID>`

Keep these responsibilities separate:

- install/bootstrap prepares the visible `external/vllm` fork
- `serve` chooses and downloads the model if needed
- `backend add-model` registers APXM routing metadata only; it does not
  download model weights

## Where The Runbook Lives

The maintained operator-facing rehearsal docs now live in:

- `docs/talks/2026-apxm-vllm/`
  - current talk bundle
  - keeps the external fork visible
  - avoids claiming unsupported Timeline or composite-demo assets

The older scratch worktree notes are still useful as supplementary operational
history:

- `.claude/worktrees/vllm-killer-demo/SESSION_STATUS.md`
- `.claude/worktrees/vllm-killer-demo/FORK_BUILDOUT_PLAN.md`

This document exists so the main repo has a stable pointer to the current
truth.

## Recommended Working Rule

If there is a conflict between:

1. `external/vllm` source code,
2. `external/vllm/AGENTS.md`, and
3. older APXM plans/helpers/docs,

prefer the fork source and fork-local instructions.

## Backend And Model Naming Rule

Keep backend identity and model identity separate:

- `vllm` is the backend protocol and serving implementation
- `gemma` is a model family, not a backend name

For a fork-local self-hosted setup, prefer a backend name such as:

- `vllm-fork`

Then register one or more concrete model IDs under that backend, for example:

- `DataPilot/ArrowMint-Gemma3-4B-ChocoMint-instruct-v0.2`

If official Google Gemma checkpoints are unavailable to the current Hugging Face
token, do not relabel another checkpoint as `google/gemma-...` just to satisfy
an example. Keep the exact served model ID visible in APXM config, examples, and
operator commands.

## Boundary Vocabulary Rule

At the APXM to vLLM integration boundary, the correct term is:

- `graph`

Do not describe that boundary as `workflow_id` or a workflow registration API.
The current fork exposes graph registration and graph status endpoints:

- `POST /v1/apxm/graphs/register`
- `GET /v1/apxm/graphs/{graph_id}`
- `DELETE /v1/apxm/graphs/{graph_id}`
