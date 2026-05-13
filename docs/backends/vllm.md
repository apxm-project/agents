# vLLM Backend

This is the Dekk-first operator path for the APXM graph-aware vLLM backend.
The fork lives under `external/vllm`; operators should use `dekk apxm vllm`
instead of calling the fork's private Python environment directly.

The canonical production shape is a pinned APXM-vLLM GPU runtime container image built
from `external/vllm` and launched as a persistent `dekk apxm vllm service-*`
job. For controlled Slurm/GPU evaluation, Slurm owns the node/GPU allocation
and Dekk owns the container lifecycle inside that allocation. Direct
`docker-*` commands are allocation-local primitives used by the service wrapper.

## Scope

APXM is model-agnostic. vLLM loads a model reference, APXM registers the served
model id that the running OpenAI-compatible server reports, and graph nodes
route to that backend/model pair.

Use these terms consistently:

- `<MODEL_REF>` is what vLLM loads. It can be a Hugging Face id, another
  provider-backed reference that vLLM supports, or a local model directory.
- `<SERVED_MODEL_ID>` is what `/v1/models` reports. APXM stores this value for
  routing.
- `<HF_MODEL_ID>` is only for Hugging Face-hosted models. Dockerized vLLM can
  populate the mounted cache on first start.

There is no APXM/vLLM default model. Examples must use placeholders unless they
are explicitly labeled as examples.

## Dekk Commands

Canonical commands:

- `dekk apxm vllm doctor` verifies the APXM fork source, Docker daemon, GPU runtime
  device nodes, Docker buildx, Slurm tools, cache settings, and port ownership.
- `dekk apxm vllm docker-build` builds a GPU runtime APXM-vLLM image from
  `external/vllm` with `docker buildx build --load` and records APXM/vLLM
  commit and dirty-tree labels. Docker's legacy builder is not a supported
  path.
- `dekk apxm vllm service-start <NAME> <MODEL_REF>` submits a persistent
  Slurm-owned APXM-vLLM service job.
- `dekk apxm vllm service-status <NAME> --probe` checks the Slurm job and
  probes the APXM-vLLM endpoint from inside that allocation.
- `dekk apxm vllm service-exec <NAME> -- <command>` runs APXM commands inside
  the service allocation through `srun --jobid <service-job> --overlap`.
- `dekk apxm vllm docker-start <MODEL_REF>` starts Dockerized vLLM only when
  the operator already owns the allocation.
- `dekk apxm vllm probe` checks `/v1/models`, the APXM graph routes, and
  requires the scheduler capability route to report `policy = "priority"`.
- `dekk apxm vllm enable <SERVED_MODEL_ID>` records the probed endpoint and
  served model id in APXM backend config.
- `dekk apxm vllm help [subcommand]` shows detailed controller options.

Dockerized vLLM communicates with APXM over HTTP. APXM stores an endpoint such
as `http://127.0.0.1:8916/v1`, uses OpenAI-compatible `/chat/completions` for
inference, and uses `/apxm/*` routes for graph-aware registration, status,
release, and scheduler capability checks. Docker is process isolation, not a
separate APXM backend protocol.

## Bring Up A Backend

Run these commands from the APXM repo root.

### 1. Verify The Host And Fork

```bash
dekk apxm vllm doctor --port 8916
```

`doctor` must report that `external/vllm` contains the APXM router, that the
OpenAI API server mounts it, that `/v1/apxm/scheduler` exists in the fork
source, and that Docker, Docker buildx, and the GPU runtime device nodes are
reachable. If
`external/vllm` is not at the expected APXM fork, fix the source before building
an image.

### 2. Build The APXM-vLLM Image

Use an explicit image tag or digest. `latest` is not part of the APXM
infrastructure path.

```bash
APXM_COMMIT="$(git rev-parse --short HEAD)"
VLLM_COMMIT="$(git -C external/vllm rev-parse --short HEAD)"
IMAGE="apxm-vllm-gpu:${APXM_COMMIT}-${VLLM_COMMIT}"

dekk apxm vllm docker-build --image "$IMAGE"
dekk apxm vllm docker-save --image "$IMAGE"
```

The build command fails early if the fork contract is absent. For final
evaluation, record the image id or registry digest after build. If Docker
buildx is unavailable, install the host's `docker-buildx` package/plugin rather
than using Docker's legacy builder.
`docker-save` writes the APXM-managed image artifact under
`.apxm/vllm-images`; Slurm launchers load that artifact and do not build images
inside allocations.

### 3. Choose A Model And Cache

For the first APXM claim-bearing readiness run on one regular `gpu` node,
use:

```text
MODEL_REF=openai/gpt-oss-120b
SERVED_MODEL_ID=gpt-oss-120b
BACKEND_NAME=vllm-fork
PORT=8916
HF_HOME_HOST=/home/raherrer/.cache/huggingface-apxm-vllm
```

This is an example model decision, not an APXM default. For local model
directories, set `MODEL_REF` to the filesystem path and set
`SERVED_MODEL_ID` to the id reported by `/v1/models`.
Use `openai/gpt-oss-20b` only for a fast smoke/debug run when image or
model-cache bring-up is the bottleneck.

### 4. Start The Dockerized Server

For graph-aware latency experiments, prefer a persistent Slurm service job over
a one-off model server:

```bash
dekk apxm vllm service-start gptoss120b "$MODEL_REF" \
  --image "$IMAGE" \
  --served-model-name "$SERVED_MODEL_ID" \
  --backend-name "$BACKEND_NAME" \
  --hf-home "$HF_HOME_HOST" \
  --port "$PORT" \
  --max-model-len 32768

dekk apxm vllm service-status gptoss120b --probe
dekk apxm vllm service-exec gptoss120b -- \
  python3 examples/python/benchmarks/benchmark_e2e.py --iterations 3
```

`service-exec` runs commands through `srun --jobid <service-job> --overlap`.
That keeps APXM on the same node as the Dockerized vLLM server, so the backend
endpoint can remain `http://127.0.0.1:$PORT/v1`. Reuse the same service for
multiple benchmark campaigns; start a new service only when changing the image,
model, context length, or server startup contract. A client in an arbitrary
different allocation should not assume the compute-node host port is reachable;
`service-exec` is the portable path because it runs the client on the service
node.

For a direct container start inside an already-owned allocation, launch with
priority scheduling, prefix caching, usage details, and constrained queue
pressure:

```bash
mkdir -p "$HF_HOME_HOST"

dekk apxm vllm docker-start "$MODEL_REF" \
  --image "$IMAGE" \
  --served-model-name "$SERVED_MODEL_ID" \
  --backend-name "$BACKEND_NAME" \
  --hf-home "$HF_HOME_HOST" \
  --port "$PORT" \
  --tensor-parallel-size 8 \
  --gpu-memory-utilization 0.90 \
  --max-model-len 32768 \
  --enable-prefix-caching \
  --scheduling-policy priority \
  --enable-prompt-tokens-details \
  --enable-force-include-usage \
  --reasoning-parser openai_gptoss \
  --tool-call-parser openai \
  --enable-auto-tool-choice \
  --enable \
  --alias benchmark \
  --max-num-seqs 4 \
  --attention-backend GPU_AITER_UNIFIED_ATTN \
  --container-env VLLM_GPU_USE_AITER=1
```

APXM can emit priority and reuse hints, but those hints only become scheduling
or cache behavior when the backend is configured to honor the corresponding
vLLM features. Treat prefix-cache counters as latency evidence, not
provider-visible token or dollar-cost reduction by themselves.

Unknown trailing flags are passed through to vLLM, so `--max-num-seqs 4` is
intentional. Dockerized vLLM binds inside the container to `0.0.0.0:$PORT`, uses
host networking, and is registered by APXM as `http://127.0.0.1:$PORT/v1`.

For reasoning-capable models, pass parser and template settings explicitly when
the model requires them. APXM does not infer these from the served model id:

```bash
dekk apxm vllm docker-start "$MODEL_REF" \
  --image "$IMAGE" \
  --served-model-name <SERVED_MODEL_ID> \
  --reasoning-parser <REASONING_PARSER> \
  --default-chat-template-kwargs '{"enable_thinking": true}' \
  --enable-prompt-tokens-details \
  --enable-force-include-usage \
  --port 8916 \
  --wait
```

The parser value is model-specific and must be selected from the model server's
supported reasoning parser list. `--enable-prompt-tokens-details` lets vLLM
include prompt-cache details such as `cached_tokens` in OpenAI-compatible usage
responses. Current vLLM chat-completion usage reports prompt-cache details, but
reasoning-token detail is only reported when the served API response includes a
provider detail field such as `completion_tokens_details.reasoning_tokens` or
`output_tokens_details.reasoning_tokens`.

### 5. Probe The Running Server

```bash
dekk apxm vllm probe --endpoint "http://127.0.0.1:${PORT}/v1"
```

`probe` checks `/v1/models`, confirms the APXM graph router is mounted, and
round-trips a temporary graph registration/status/release. A 404 from the APXM
graph route means the process is not the APXM fork.

The runtime backend also probes `GET /v1/apxm/scheduler` as capability evidence
for priority scheduling. Missing scheduler information does not invalidate graph
registration, but a priority-latency claim requires evidence that the scheduler
policy is `priority`.

### 6. Enable APXM Routing

```bash
dekk apxm vllm enable "$SERVED_MODEL_ID" \
  --backend-name "$BACKEND_NAME" \
  --endpoint "http://127.0.0.1:${PORT}/v1" \
  --alias benchmark
```

`enable` verifies that `/v1/models` reports the served model id, verifies the
APXM graph route, adds the backend if needed, adds the served model id if
needed, and runs `dekk apxm backend test`. It does not start vLLM, download
weights, or rewrite graph/chat routing policy.
For authenticated servers, keep `VLLM_API_KEY` set or pass
`--api-key-env <ENV_VAR>` so APXM stores an environment reference instead of a
literal secret. Unauthenticated local/on-prem vLLM is valid; no fake API key is
written.

The default backend name is `vllm-fork`. If a graph or config file expects a
different backend name, pass it explicitly:

```bash
dekk apxm vllm enable <SERVED_MODEL_ID> \
  --backend-name vllm-local \
  --port 8916
```

`enable` writes the normal global APXM backend store at `~/.apxm/config.toml`.
Project-level config files or an explicit `--config` may use a different backend
store, so keep the selected backend name and model id aligned with the workload
you are about to run.

Python graphs validate backend/model routes against the active APXM config when
they are built. Use `apxm.backends.select_backend(...)` to select registered
routes instead of reading ad hoc `APXM_VLLM_*` environment variables in graph
code.

## Example: Local Directory

```bash
dekk apxm vllm docker-start /models/my-model \
  --image "$IMAGE" \
  --served-model-name my-model-local \
  --port 8916 \
  --wait \
  --enable
dekk apxm vllm probe --port 8916
dekk apxm vllm enable my-model-local --port 8916
```

Do not enable the filesystem path unless `/v1/models` reports that exact string.

## Example: Hugging Face Cache

```bash
dekk apxm vllm docker-start <HF_MODEL_ID> \
  --image "$IMAGE" \
  --served-model-name <SERVED_MODEL_ID> \
  --hf-home /path/to/hf-cache \
  --port 8916 \
  --wait \
  --enable
dekk apxm vllm probe --port 8916
dekk apxm vllm enable <SERVED_MODEL_ID> --port 8916
```

This is a Hugging Face example only. Other vLLM-supported model sources should
follow their own authentication and storage rules.

## Run Graphs With Metrics

Use a checked-in self-hosted smoke graph for a fresh-checkout-safe metrics
test. Register a role alias on the served model, then let the Python frontend
resolve that alias from the backend registry:

```bash
dekk apxm vllm enable <SERVED_MODEL_ID> --port 8916 --alias smoke
APXM_METRICS_DIR="$(mktemp -d)"
dekk apxm execute \
  --emit-session "${APXM_METRICS_DIR}/session" \
  --emit-metrics "${APXM_METRICS_DIR}/metrics.json" \
  examples/python/self-hosted/vllm_graph_smoke.py
```

The `smoke` alias is an example role name, not a model default. If you prefer
not to use aliases, edit the example to call
`select_backend(protocol=VLLM.protocol, backend=<BACKEND_NAME>, model=<SERVED_MODEL_ID>)`.

Benchmark graphs can use different backend names and model ids. Before running
one, align its config with the backend you enabled:

```bash
dekk apxm execute \
  --emit-session "${APXM_METRICS_DIR}/priority-session" \
  --emit-metrics "${APXM_METRICS_DIR}/priority-metrics.json" \
  examples/python/benchmarks/stress/priority_scheduling.air
```

Session output creates a timestamped execution directory under the path you
pass. To inspect traces, use the concrete session directory printed by the
command, for example:

```bash
rg -n "memoization_hit|token_usage|scheduler_decision" \
  "${APXM_METRICS_DIR}/session"/*/trace.ndjson
```

Backend graph telemetry is emitted under `backends.graphs[]` in `metrics.json`
or the path passed to `--emit-metrics`. Each entry reports `backend_kind`,
`backend_name`, `graph_id`, `pinned_handles`, `pinned_blocks`,
`critical_path_length`, and `node_count`. Those fields prove APXM captured
graph status from the fork; they are not a KV-cache hit-rate or speedup metric
by themselves.

`runtime.token_accounting` is the per-node and aggregate token accounting
section. It contains:

- `total` for execution-wide LLM token usage
- `per_node` keyed by APXM node id
- `per_flow` keyed by flow name
- `per_agent` keyed by APXM agent name when the request runs inside an agent
  scope
- `cached_input_tokens` and `reasoning_output_tokens` when the backend reports
  those provider detail counts

Use both sections together. `runtime.token_accounting.per_node` answers which
APXM nodes spent tokens. `backends.graphs[]` answers whether the graph-aware
backend retained graph state and pin metadata.

Spawned coding agents such as Claude Code or Codex are ACP subprocesses. APXM can
measure the APXM node lifecycle, latency, output, session files, and child
execution links for those nodes. Provider token usage is only included in
`runtime.token_accounting` when the spawned agent or its ACP adapter reports
usage back to APXM. If the external agent does not expose token usage, keep the
metrics honest: report node duration and outputs, but do not invent token or
cost numbers. The long-term shape is for ACP adapters to normalize usage into
the same per-node accounting contract.

## Multiple Models

One APXM backend entry represents one endpoint. Register multiple model ids
under the same backend only when that endpoint's `/v1/models` reports each
served id. If models run on different ports or hosts, use different backend
names so each backend endpoint remains unambiguous.

## Hard Stop Conditions

Do not treat the backend as graph-aware if any of these are true:

- `doctor` does not resolve imports under `external/vllm`
- `probe` cannot reach the APXM graph endpoints
- `enable` cannot find `<SERVED_MODEL_ID>` in `/v1/models`
- the workload is routed to a different backend/model than the one you enabled

At the APXM to vLLM boundary, the correct term is `graph`, not `workflow`.
