# External vLLM Fork

**Why the fork:** stock vLLM silently ignores APXM's `extra_body.apxm`
scheduling hints, so requests sent to a non-fork server execute without
graph-aware scheduling, KV-retention pinning, or critical-path priority.
APXM-aware behavior requires the fork at `external/vllm/` (branch `apxm`).

This document captures the current APXM vLLM integration reality in this
repository.

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

APXM never installs precompiled vLLM wheels. Precompiled wheels come from
upstream source and would not contain the apxm patches in this fork. The
install path always builds from the visible `external/vllm` checkout:

```
uv pip install --python .venv/bin/python --torch-backend=auto -r requirements/build.txt
uv pip install --python .venv/bin/python -e . --torch-backend=auto --no-build-isolation
```

The first line pre-populates `.venv` with the hardware-correct torch
(`--torch-backend=auto` resolves GPU runtime vs CUDA vs CPU per host). The second
line builds the editable install against that same venv via
`--no-build-isolation`, so `setup.py`'s device auto-detect (`torch.version.hip`
etc.) sees the right torch and selects the right `VLLM_TARGET_DEVICE`.

APXM exposes a maintained top-level entrypoint:

- `dekk apxm vllm install`
- `dekk apxm vllm serve <HF_MODEL_ID>`

The fork remains an independently managed visible checkout with its own
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

## Documentation Drift

No outstanding drift. The previous mismatches were resolved by:

- removing the historical Python prototype at
  `crates/runtime/apxm-backends/python/apxm_vllm/`
- removing the obsolete killer-demo plans (`docs/planning/specs/...-killer-demo-design.md`,
  `docs/planning/plans/...-killer-demo.md`) that described a 4-endpoint
  `/v1/apxm/pins*` design instead of the live 3-endpoint graph contract
- pointing `.dekk.toml`'s `vllm` install component at
  `tools/scripts/install_external_vllm.sh`
- centralising the maintained fork commands in `tools/scripts/vllm.py`
  (`dekk apxm vllm install`, `dekk apxm vllm serve <HF_MODEL_ID>`)

## Integration Contract

The Rust backend in
`crates/runtime/apxm-backends/src/llm/backends/vllm/backend.rs` is aligned to
the live fork contract:

- graph registration via `POST /v1/apxm/graphs/register`
- graph inspection via `GET /v1/apxm/graphs/{graph_id}`
- graph release via `DELETE /v1/apxm/graphs/{graph_id}`
- per-request `extra_body.apxm.pin_policy` hints (no out-of-band pin endpoint)

`health_check()` probes `/v1/apxm/graphs/__probe__` and hard-fails on 404 by
default — stock vLLM silently drops `extra_body.apxm`, so APXM refuses to run
against it unless `require_apxm_endpoints = false` is set on the backend.

Any future runtime or benchmark work should align to that contract.

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

## Bringing Up A Model (User Guide)

APXM is model-agnostic. The fork serves any vLLM-supported HuggingFace model;
APXM simply registers the model id under a `vllm` backend and routes graph
nodes to it. The example below uses `google/gemma-4-31B-it`, but the same
steps apply to any model id (Qwen, Llama, Mistral, fine-tunes, etc.).

Model weights, HF licenses, and accelerator drivers are external to APXM.

### 1. Choose where weights live

`HF_HOME` controls where HuggingFace caches model blobs. Set it once and use
the same value for download and serve so vLLM finds the cached weights:

```sh
export HF_HOME=/var/tmp/hf-cache    # local SSD; faster cold start than NFS
mkdir -p "$HF_HOME"
df -h "$HF_HOME"                     # confirm enough free space for the model
```

Sizing rule of thumb (bf16): ~2 bytes per parameter. A 31 B model is ~60 GB
on disk; a 7 B model is ~14 GB. Add headroom if you plan to keep multiple
checkpoints on the same volume.

### 2. Authenticate with HuggingFace (if the model is gated)

```sh
huggingface-cli login                # writes ~/.cache/huggingface/token
export HF_TOKEN=$(cat ~/.cache/huggingface/token)
```

For gated models (Llama, some Gemma releases), accept the license on the
model page in a browser using the same HF account before downloading.
Gemma 4 is currently Apache 2.0 and not gated, so license acceptance is not
required for `google/gemma-4-*`.

### 3. Pre-download the weights (recommended)

Pre-downloading separates "did the download fail" from "did the serve fail":

```sh
HF_HOME=/var/tmp/hf-cache external/vllm/.venv/bin/python - <<'PY'
from huggingface_hub import snapshot_download
snapshot_download(
    "google/gemma-4-31B-it",                       # substitute any model id
    allow_patterns=["*.safetensors", "*.json", "tokenizer*", "*.model"],
    max_workers=8,
)
PY
```

`dekk apxm vllm serve` will also download on first run; pre-downloading just
makes the first serve fast and keeps download errors out of the serve log.

### 4. Make sure `transformers` knows the model architecture

Cutting-edge models often need a newer `transformers` than the one vLLM
installed by default. If `transformers` does not recognize the
`config.json#model_type`, vLLM exits during model-config validation with
something like:

```
Value error, The checkpoint you are trying to load has model type `gemma4`
but Transformers does not recognize this architecture.
```

Upgrade in the fork's venv (always via `uv`, never bare `pip`):

```sh
cd external/vllm
uv pip install --python .venv/bin/python -U 'transformers>=5.5.0'
```

Pick the minimum version listed in the model's release notes (Gemma 4
needs `transformers >= 5.5.0`). For brand-new models you may need
`transformers` from main.

### 5. Serve the model

```sh
HF_HOME=/var/tmp/hf-cache \
dekk apxm vllm serve google/gemma-4-31B-it \
  --gpus 6,7 \
  --tensor-parallel-size 2 \
  --gpu-memory-utilization 0.9 \
  --max-model-len 8192 \
  --port 8916
```

Knob guide:

- `--gpus` — comma-separated device ids; sets both `HIP_VISIBLE_DEVICES`
  (GPU runtime) and `CUDA_VISIBLE_DEVICES` (NVIDIA). Pin only to GPUs you own on a
  shared host.
- `--tensor-parallel-size` — must equal the number of devices in `--gpus`.
  Use TP > 1 for models that don't fit in one GPU's VRAM, or to lower
  per-GPU memory pressure for very long contexts.
- `--gpu-memory-utilization` — fraction of each GPU's VRAM vLLM may claim
  for weights + KV cache. 0.9 is a safe default.
- `--max-model-len` — caps the context window the server advertises. Lower
  values reduce KV-cache memory (useful when sharing GPUs).

These are vLLM serving knobs; APXM does not dictate them.

### 6. Probe the running server

Wait for `Application startup complete` in the serve log, then verify:

```sh
# Model is reachable
curl -s http://localhost:8916/v1/models | jq -r '.data[].id'
# Expect: google/gemma-4-31B-it

# Fork APXM router is mounted (200 with registered:false confirms the route exists)
curl -s http://localhost:8916/v1/apxm/graphs/__probe__ | jq .
# Expect: {"object":"apxm.graph.status","graph_id":"__probe__","registered":false,...}

# Round-trip a chat request
curl -s -X POST http://localhost:8916/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"google/gemma-4-31B-it","messages":[{"role":"user","content":"Say hi in 5 words."}]}' \
  | jq -r '.choices[0].message.content'

# Full round-trip of the APXM graph contract
curl -s -X POST http://localhost:8916/v1/apxm/graphs/register \
  -H 'content-type: application/json' \
  -d '{"graph_id":"smoke-1","execution_id":"e1","nodes":[{"node_id":1,"name":"hello","downstream_nodes":[]}]}'
curl -s http://localhost:8916/v1/apxm/graphs/smoke-1
curl -s -X DELETE http://localhost:8916/v1/apxm/graphs/smoke-1
```

If `/v1/apxm/graphs/__probe__` returns connection-refused, the server is not
up yet. If it returns 404, you are talking to stock vLLM (no apxm router) —
reinstall with `dekk apxm vllm install` and confirm the verifier prints
*"OK: editable install resolves to external/vllm fork"*.

### 7. Register the backend and model with APXM

```sh
dekk apxm backend add vllm-fork --type onprem --protocol vllm
dekk apxm backend add-model vllm-fork google/gemma-4-31B-it
dekk apxm backend test vllm-fork
```

The default endpoint is `http://localhost:8916/v1`, matching step 5. Use
`--endpoint` only if the server runs on a different host or port.

`backend test` calls `health_check()`, which probes the fork's APXM router
and hard-fails on stock vLLM. If it fails with *"does not expose /v1/apxm/*
endpoints"*, you are pointed at a stock-vLLM server — restart the fork or
fix the endpoint url. Override only if you accept silent loss of
`extra_body.apxm` hints by setting `require_apxm_endpoints = false` on the
backend in `~/.apxm/config.toml`.

`backend add-model` registers routing metadata in APXM config. It does not
download or serve anything — those are steps 3 and 5.

### Multiple models on one backend

Repeat step 7's `add-model` line for each model id you want APXM to route to
the same backend. Each model id must be served on the backend's endpoint
(either by relaunching `vllm serve` with a different model or by running
multiple backends on different ports).

### Common failure modes

| Symptom | Cause | Fix |
|---|---|---|
| `model type X but Transformers does not recognize this architecture` | `transformers` in the fork venv is older than the model needs | step 4: `uv pip install --python .venv/bin/python -U 'transformers>=N'` |
| `OSError: [model] is not a local folder and is not a valid model identifier` | weights not downloaded; `HF_HOME` mismatch between download and serve | re-run step 3 with the same `HF_HOME` exported in step 5 |
| `RuntimeError: ... requires a GPU with compute capability ...` | `--gpus` selects a device the build does not support | pin to a supported device or rebuild the fork for that platform |
| `health_check` returns *"does not expose /v1/apxm/* endpoints"* | server is stock vLLM, not the fork | `dekk apxm vllm install` to rebuild the editable fork install |
| Repeated `address already in use` on the serve port | previous serve still running | `lsof -i :8916` and stop it; `--port` to pick another port |

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

## Observing Pin Behavior

When `--emit-metrics` (or `--emit-session`) is active, the `backends.vllm.graphs[]`
array in `metrics.json` reports per-graph pin telemetry collected from the fork
just before each graph is released. Each entry includes `pinned_blocks`,
`pinned_handles`, `critical_path_length`, and `node_count`. This is the canonical
place to observe whether the fork's KV-cache pinning is active and how many blocks
are being retained for a given execution.

## Boundary Vocabulary Rule

At the APXM to vLLM integration boundary, the correct term is:

- `graph`

Do not describe that boundary as `workflow_id` or a workflow registration API.
The current fork exposes graph registration and graph status endpoints:

- `POST /v1/apxm/graphs/register`
- `GET /v1/apxm/graphs/{graph_id}`
- `DELETE /v1/apxm/graphs/{graph_id}`
