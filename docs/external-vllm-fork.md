# External vLLM Fork

**Why the fork:** stock vLLM does not consume APXM's vLLM extension hints,
so requests sent to a non-fork server execute without graph-aware scheduling,
KV-retention pinning, or critical-path priority.
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

OpenAI-compatible clients supply per-request APXM scheduling hints through
`extra_body`; the fork reads the nested `vllm_xargs.apxm` object and also
accepts related legacy flattened `apxm_*` fields. The fork uses those request
hints directly for graph-aware scheduling and KV-retention behavior.

## Build And Install Rules

APXM never installs precompiled vLLM wheels. Precompiled wheels come from
upstream source and would not contain the apxm patches in this fork. The
install path always builds from the visible `external/vllm` checkout.

APXM exposes a maintained top-level entrypoint:

- `dekk apxm vllm install`
- `dekk apxm vllm help [subcommand]`
- `dekk apxm vllm doctor`
- `dekk apxm vllm download <HF_MODEL_ID>`
- `dekk apxm vllm serve <MODEL_REF>`
- `dekk apxm vllm start <MODEL_REF>`
- `dekk apxm vllm status`
- `dekk apxm vllm logs`
- `dekk apxm vllm probe`
- `dekk apxm vllm enable <SERVED_MODEL_ID>`
- `dekk apxm vllm stop`

The fork remains a visible checkout under `external/vllm`, but operators should
not invoke its private Python environment directly. The controller owns that
implementation detail, verifies that imports resolve to this checkout, and
keeps model download, process control, probing, and APXM registration behind
the same `dekk apxm vllm` surface.

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

The current documentation contract is:

- removing the historical Python prototype at
  `crates/runtime/apxm-backends/python/apxm_vllm/`
- removing obsolete killer-demo planning docs that described a 4-endpoint
  `/v1/apxm/pins*` design instead of the live 3-endpoint graph contract
- pointing `.dekk.toml`'s `vllm` install component at the maintained
  `tools/scripts/vllm.py install` controller
- centralizing the maintained fork commands in `tools/scripts/vllm.py`
  (`dekk apxm vllm install`, `doctor`, `download`, `serve`, `start`,
  `status`, `logs`, `probe`, `enable`, and `stop`)

Operator-facing docs should use the `dekk apxm vllm` commands. Direct fork
interpreter paths, ad hoc `nohup`/PID management, and manual `lsof`/`kill`
flows belong in troubleshooting notes only when the maintained command cannot
express the operation.

## Integration Contract

The Rust backend in
`crates/runtime/apxm-backends/src/llm/backends/vllm/backend.rs` is aligned to
the live fork contract:

- graph registration via `POST /v1/apxm/graphs/register`
- graph inspection via `GET /v1/apxm/graphs/{graph_id}`
- graph release via `DELETE /v1/apxm/graphs/{graph_id}`
- per-request `vllm_xargs.apxm.pin_policy` hints supplied through OpenAI
  `extra_body` (no out-of-band pin endpoint)

`health_check()` probes `/v1/apxm/graphs/__apxm_probe__` and hard-fails on 404
by default — stock vLLM does not consume APXM's vLLM extension hints, so APXM
refuses to run against it unless `require_apxm_endpoints = false` is set on the
backend.

Any future runtime or benchmark work should align to that contract.

## Canonical Operator Path

For the simplest repo-local operator flow:

1. Install APXM with Dekk.
2. Install the visible fork: `dekk apxm vllm install`
3. Verify the fork and host: `dekk apxm vllm doctor`
4. Choose the model reference to serve. vLLM has no APXM default model.
5. Start the server: `dekk apxm vllm start <MODEL_REF> --wait`
6. Probe the OpenAI and APXM graph endpoints: `dekk apxm vllm probe`
7. Enable APXM routing metadata:
   `dekk apxm vllm enable <SERVED_MODEL_ID>`

Keep these responsibilities separate:

- install/bootstrap prepares the visible `external/vllm` fork
- `download` prepares Hugging Face model weights in the configured cache; skip
  it for an already-present local model path
- `start` and `stop` own server process lifecycle
- `probe` validates both `/v1/models` and the APXM graph control plane
- `enable` records APXM routing metadata only; it does not
  download model weights

## Bringing Up Any Model

APXM is model-agnostic. The fork serves any vLLM-supported model reference,
and APXM registers the served model id under a `vllm` backend so graph nodes
can route to it. The example below uses `google/gemma-4-31B-it`, but the same
steps apply to Hugging Face ids, fine-tunes, and local model directories that
vLLM can load.
There is no default APXM/vLLM model; the operator must choose and start one.

Use the terms consistently:

- `<MODEL_REF>` is what vLLM loads. It can be a Hugging Face id or a local
  model path.
- `<SERVED_MODEL_ID>` is what the running OpenAI-compatible server exposes in
  `/v1/models`, and what APXM stores for routing. It defaults to
  `<MODEL_REF>` unless you pass `--served-model-name`.
- `<HF_MODEL_ID>` is only for `dekk apxm vllm download`, which pre-populates a
  Hugging Face cache. Skip `download` for already-present local model paths.

Model weights, model licenses, cache placement, and accelerator drivers are
external to APXM.
Dekk is the supported entrypoint because it activates the repo environment
before invoking APXM and the fork controller.

The generic shape is:

```sh
dekk apxm vllm doctor --hf-home /path/to/hf-cache --port 8916
dekk apxm vllm start <MODEL_REF> \
  --hf-home /path/to/hf-cache \
  --served-model-name <SERVED_MODEL_ID> \
  --port 8916 \
  --wait
dekk apxm vllm probe --port 8916
dekk apxm vllm enable <SERVED_MODEL_ID> --port 8916
```

If `<MODEL_REF>` is a Hugging Face id and you want to pre-populate the cache
before serving, run:

```sh
dekk apxm vllm download <HF_MODEL_ID> --hf-home /path/to/hf-cache
```

`enable` is a convenience wrapper. It verifies `/v1/models`, verifies the APXM
graph route, adds the backend if needed, adds the served model id if needed,
and runs `dekk apxm backend test` unless `--skip-test` is set. It writes APXM's
normal backend store (`~/.apxm/config.toml`), and it does not start vLLM,
download weights, or rewrite graph/chat routing policy. For authenticated
servers, keep `VLLM_API_KEY` set or pass `--api-key-env <ENV_VAR>` so APXM
stores an environment reference instead of a literal secret. To make a workload
use the backend, route that workload to the backend/model you registered.

For a local model path, do not enable the filesystem path unless vLLM exposes
that exact string from `/v1/models`. Prefer an explicit served name:

```sh
dekk apxm vllm start /models/my-model \
  --served-model-name my-model-local \
  --port 8916 \
  --wait
dekk apxm vllm probe --port 8916
dekk apxm vllm enable my-model-local --port 8916
```

Gemma 4 is the worked example we validated; it is not a special APXM backend
or a required model.

## Lab Validation: Gemma 4

This is one validated bring-up path for the repo-local APXM vLLM fork using
Gemma 4. Substitute any vLLM-supported model reference that fits the machine.
Run every command from the APXM repo root unless the command says otherwise.

### 1. Stop or inspect stale servers

```sh
dekk apxm vllm status --port 8916
dekk apxm vllm stop --port 8916
```

### 2. Verify the visible fork and host

This must report that the editable install resolves to `external/vllm`, not a
wheel or hidden checkout:

```sh
dekk apxm vllm doctor --hf-home /path/to/hf-cache --port 8916
```

`doctor` also prints package versions, GPU visibility, the configured
Hugging Face cache, and any process currently bound to the APXM vLLM port.

### 3. Download Gemma 4 once

Use local SSD for the Hugging Face cache. The 31 B checkpoint is roughly
60 GiB on disk, so keep at least 80 GiB free:

```sh
dekk apxm vllm download google/gemma-4-31B-it \
  --hf-home /path/to/hf-cache
```

### 4. Start Gemma 4

The background controller writes logs and PID state under `.apxm/vllm-logs/`.
Use one GPU first unless you intentionally set tensor parallelism across
multiple devices.

```sh
dekk apxm vllm start google/gemma-4-31B-it \
  --served-model-name google/gemma-4-31B-it \
  --hf-home /path/to/hf-cache \
  --gpus 0 \
  --tensor-parallel-size 1 \
  --gpu-memory-utilization 0.90 \
  --max-model-len 8192 \
  --port 8916 \
  --wait
```

Use more GPUs only when you also increase tensor parallelism to the same
count, for example `--gpus 6,7 --tensor-parallel-size 2`.

### 5. Watch logs and probe the fork

```sh
dekk apxm vllm logs --port 8916 --lines 120
dekk apxm vllm probe --port 8916
```

`probe` checks `/v1/models`, confirms that `/v1/apxm/graphs/__apxm_probe__`
exists, and round-trips a temporary graph registration/status/release. A 404
from the APXM graph route means the process is not the APXM fork.

### 6. Enable APXM routing

Enable APXM routing only after the server is reachable:

```sh
dekk apxm vllm enable <SERVED_MODEL_ID> --port 8916 --alias smoke
```

### 7. Run an APXM smoke with metrics

Use the checked-in self-hosted smoke graph so the example does not assume a
benchmark-specific backend name. The graph resolves the registered
`smoke` alias with `select_backend(...)`; `<SERVED_MODEL_ID>` can be any model
served by your vLLM endpoint.

```sh
APXM_METRICS_DIR="$(mktemp -d)"
dekk apxm execute \
  --emit-session "${APXM_METRICS_DIR}/session" \
  --emit-metrics "${APXM_METRICS_DIR}/metrics.json" \
  examples/python/self-hosted/vllm_graph_smoke.py
```

Check that backend graph telemetry is present:

```sh
jq '.backends.graphs' "${APXM_METRICS_DIR}/metrics.json"
```

If that array is missing, APXM did not capture pre-release graph status.

## General Model Notes

### 1. Choose where weights live

`HF_HOME` controls where Hugging Face caches model blobs. Set it once and use
the same value for download and serve so vLLM finds the cached weights:

```sh
dekk apxm vllm doctor --hf-home /path/to/hf-cache
```

Sizing rule of thumb (bf16): ~2 bytes per parameter. A 31 B model is ~60 GB
on disk; a 7 B model is ~14 GB. Add headroom if you plan to keep multiple
checkpoints on the same volume.

### 2. Authenticate with the model provider, when required

```sh
huggingface-cli login                # writes ~/.cache/huggingface/token
export HF_TOKEN=$(cat ~/.cache/huggingface/token)
```

For Hugging Face-hosted gated models, log in and accept the model license with
the same account before downloading. For local model directories or non-Hugging
Face sources, follow that source's access flow instead.

### 3. Pre-download Hugging Face weights when using an HF model id

Pre-downloading separates "did the download fail" from "did the serve fail":

```sh
dekk apxm vllm download <HF_MODEL_ID> \
  --hf-home /path/to/hf-cache
```

`dekk apxm vllm start` will also let vLLM download on first run;
pre-downloading just makes the first serve fast and keeps download errors out
of the serve log.
Skip this step for a local model directory that already contains the weights
and tokenizer files.

### 4. Make sure `transformers` knows the model architecture

Cutting-edge models often need a newer `transformers` than the one vLLM
installed by default. If `transformers` does not recognize the
`config.json#model_type`, vLLM exits during model-config validation with
something like:

```
Value error, The checkpoint you are trying to load has model type `gemma4`
but Transformers does not recognize this architecture.
```

Run `doctor` first. If the installed fork environment is too old for a model,
rebuild through the APXM controller:

```sh
dekk apxm vllm doctor --hf-home /path/to/hf-cache
dekk apxm vllm install
```

Pick the minimum version listed in the model's release notes. If a brand-new
model needs a dependency newer than the installer provides, update the fork
requirements and rebuild through `dekk apxm vllm install` rather than
documenting one-off environment mutation.

### 5. Serve the model

```sh
dekk apxm vllm start <MODEL_REF> \
  --hf-home /path/to/hf-cache \
  --served-model-name <SERVED_MODEL_ID> \
  --gpus 6,7 \
  --tensor-parallel-size 2 \
  --gpu-memory-utilization 0.9 \
  --max-model-len 8192 \
  --port 8916 \
  --wait
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

```sh
dekk apxm vllm logs --port 8916 --lines 120
dekk apxm vllm probe --port 8916
```

If `/v1/apxm/graphs/__apxm_probe__` returns connection-refused, the server is
not up yet. If it returns 404, you are talking to stock vLLM (no apxm router) —
reinstall with `dekk apxm vllm install` and confirm the verifier prints
*"OK: editable install resolves to external/vllm fork"*.

### 7. Enable the backend and model with APXM

```sh
dekk apxm vllm enable <SERVED_MODEL_ID> --port 8916
```

The default endpoint is `http://127.0.0.1:8916/v1`, matching step 5. For a new
backend, pass `--endpoint` if the server is not at that address. If the backend
name already exists with a different endpoint, `enable` refuses to attach the
model to that backend; update/remove the backend first, or pass a new
`--backend-name`.

`enable` records routing metadata in APXM config. It does not download or
serve anything; those are steps 3 and 5.
It verifies the served model id and APXM graph route before writing config
unless `--skip-test` is set. It also runs `dekk apxm backend test`, which is a
basic backend connectivity check.

### Multiple models on one backend

Repeat step 7's registration for each model id you want APXM to route to the
same backend. One APXM backend entry represents one endpoint. Register multiple
model ids under the same backend only when that endpoint's `/v1/models` reports
each served id. If models run on different ports or hosts, use different
backend names so each endpoint remains unambiguous.

### Common failure modes

| Symptom | Cause | Fix |
|---|---|---|
| `model type X but Transformers does not recognize this architecture` | the fork environment is older than the model needs | update the fork requirements or installer, then run `dekk apxm vllm install` |
| `OSError: [model] is not a local folder and is not a valid model identifier` | weights not downloaded; Hugging Face cache mismatch between download and serve | re-run step 3 with the same `--hf-home` used in step 5 |
| `RuntimeError: ... requires a GPU with compute capability ...` | `--gpus` selects a device the build does not support | pin to a supported device or rebuild the fork for that platform |
| `health_check` returns *"does not expose /v1/apxm/* endpoints"* | server is stock vLLM, not the fork | `dekk apxm vllm install` to rebuild the editable fork install |
| Repeated `address already in use` on the serve port | previous serve still running | `dekk apxm vllm status --port 8916`, then `dekk apxm vllm stop --port 8916`; use `--port` to pick another port |

## Where The Backend Runbook Lives

The maintained operator-facing backend runbook now lives in:

- `docs/backends/vllm.md`

The deck directory is speaker-facing rehearsal material. It may reference this
backend runbook, but it should not own the durable backend setup path.

This document exists so the main repo has a stable pointer to the current
truth.

## Recommended Working Rule

For source-level contract questions, prefer `external/vllm` source code and
`external/vllm/AGENTS.md`.

For operator workflow, prefer the maintained `dekk apxm vllm` commands first.
Direct fork-local commands belong only in troubleshooting or source-level
diagnosis.

## Backend And Model Naming Rule

Keep backend identity and model identity separate:

- `vllm` is the backend protocol and serving implementation
- `gemma` is a model family, not a backend name

For a fork-local self-hosted setup, prefer a backend name such as:

- `vllm-fork`

Then register one or more concrete served model IDs under that backend, for
example:

- `Qwen/Qwen2.5-7B-Instruct`
- `local-instruct-finetune`

If official Google Gemma checkpoints are unavailable to the current Hugging Face
token, do not relabel another checkpoint as `google/gemma-...` just to satisfy
an example. Keep the exact served model ID visible in APXM config, examples, and
operator commands.

## Observing Pin Behavior

When `--emit-metrics` (or `--emit-session`) is active, `metrics.json` reports
two complementary metric layers:

- `runtime.token_accounting` contains aggregate token usage plus `per_node`,
  `per_flow`, and `per_agent` token accounting for LLM calls that report usage
  through APXM.
- `backends.graphs[]` contains graph-aware backend snapshots for every backend
  kind, including vLLM.

Each graph snapshot includes `backend_kind`, `backend_name`, `graph_id`,
`pinned_blocks`, `pinned_handles`, `critical_path_length`, and `node_count`.
This is the canonical place to observe whether APXM captured graph status from
the fork. It is not a KV-cache hit-rate or speedup metric; those require
separate measurement.

Spawned coding agents such as Codex or Claude are ACP subprocesses. APXM can
measure their node lifecycle, latency, outputs, session files, and child
execution links. Provider token usage belongs in `runtime.token_accounting`
only when the spawned agent or ACP adapter reports it back to APXM; otherwise
do not infer tokens or cost from wall-clock time.

## Boundary Vocabulary Rule

At the APXM to vLLM integration boundary, the correct term is:

- `graph`

Do not describe that boundary as `workflow_id` or a workflow registration API.
The current fork exposes graph registration and graph status endpoints:

- `POST /v1/apxm/graphs/register`
- `GET /v1/apxm/graphs/{graph_id}`
- `DELETE /v1/apxm/graphs/{graph_id}`
