# vLLM Backend

This is the Dekk-first operator path for the APXM graph-aware vLLM backend.
The fork lives under `external/vllm`; operators should use `dekk apxm vllm`
instead of calling the fork's private Python environment directly.

## Scope

APXM is model-agnostic. vLLM loads a model reference, APXM registers the served
model id that the running OpenAI-compatible server reports, and graph nodes
route to that backend/model pair.

Use these terms consistently:

- `<MODEL_REF>` is what vLLM loads. It can be a Hugging Face id, another
  provider-backed reference that vLLM supports, or a local model directory.
- `<SERVED_MODEL_ID>` is what `/v1/models` reports. APXM stores this value for
  routing.
- `<HF_MODEL_ID>` is only for `dekk apxm vllm download`, which pre-populates a
  Hugging Face cache. Skip it for local model directories and non-Hugging Face
  model sources.

There is no APXM/vLLM default model. Examples must use placeholders unless they
are explicitly labeled as examples.

## Dekk Commands

- `dekk apxm vllm install` builds the repo-local fork environment.
- `dekk apxm vllm help [subcommand]` shows detailed controller options.
- `dekk apxm vllm doctor` verifies imports, package versions, GPU visibility,
  and port ownership.
- `dekk apxm vllm download <HF_MODEL_ID>` downloads Hugging Face weights into
  the configured cache. Pass `--hf-home`, set `APXM_VLLM_HF_HOME`, or let
  Hugging Face use its own default cache.
- `dekk apxm vllm start <MODEL_REF>` starts the forked server in the background.
- `dekk apxm vllm serve <MODEL_REF>` runs the forked server in the foreground.
- `dekk apxm vllm probe` checks `/v1/models` and the APXM graph endpoints.
- `dekk apxm vllm enable <SERVED_MODEL_ID>` records the running endpoint and
  served model id in APXM backend config.
- `dekk apxm vllm status`, `logs`, and `stop` manage the repo-local server.

## Bring Up A Backend

Run these commands from the APXM repo root.

### 1. Install And Verify The Fork

```bash
dekk apxm vllm install
dekk apxm vllm doctor --port 8916
```

`doctor` must report that the editable vLLM install and APXM router resolve
under `external/vllm`. If it points at a wheel, `/tmp` checkout, or unrelated
environment, stop and reinstall through Dekk.

### 2. Choose A Model Reference

If the model is hosted on Hugging Face and you want to pre-populate the cache:

```bash
dekk apxm vllm download <HF_MODEL_ID> --hf-home /path/to/hf-cache
```

For local model directories, skip `download` and pass the directory path to
`start`.

### 3. Start The Server

```bash
dekk apxm vllm start <MODEL_REF> \
  --port 8916 \
  --wait
```

Add `--served-model-name <SERVED_MODEL_ID>` when you want the server to expose
a stable model id different from `<MODEL_REF>`. Add `--hf-home /path/to/cache`
only for Hugging Face-backed model refs when you do not want the Hugging Face
default cache. GPU and memory flags such as `--gpus`,
`--tensor-parallel-size`, `--gpu-memory-utilization`, and `--max-model-len` are
passed through to vLLM.

For graph-aware latency experiments, launch the backend with vLLM prefix caching
and priority scheduling enabled:

```bash
dekk apxm vllm start <MODEL_REF> \
  --served-model-name <SERVED_MODEL_ID> \
  --enable-prefix-caching \
  --scheduling-policy priority \
  --port 8916 \
  --wait
```

APXM can emit priority and reuse hints, but those hints only become scheduling
or cache behavior when the backend is configured to honor the corresponding
vLLM features. Treat prefix-cache counters as latency evidence, not
provider-visible token or dollar-cost reduction by themselves.

For reasoning-capable models, pass the parser and template settings explicitly.
APXM does not infer these from the served model id:

```bash
dekk apxm vllm start <MODEL_REF> \
  --served-model-name <SERVED_MODEL_ID> \
  --reasoning-parser <REASONING_PARSER> \
  --default-chat-template-kwargs '{"enable_thinking": true}' \
  --enable-prompt-tokens-details \
  --enable-force-include-usage \
  --port 8916 \
  --wait
```

For the internal Gemma 4 run, `<REASONING_PARSER>` is `gemma4`. That is an
example parser selection, not an APXM default. `--enable-prompt-tokens-details`
lets vLLM include prompt-cache details such as `cached_tokens` in
OpenAI-compatible usage responses. Current vLLM chat-completion usage reports
prompt-cache details, but reasoning-token detail is only reported when the
served API response includes a provider detail field such as
`completion_tokens_details.reasoning_tokens` or
`output_tokens_details.reasoning_tokens`.

### 4. Probe The Running Server

```bash
dekk apxm vllm probe --port 8916
```

`probe` checks `/v1/models`, confirms the APXM graph router is mounted, and
round-trips a temporary graph registration/status/release. A 404 from the APXM
graph route means the process is not the APXM fork.

### 5. Enable APXM Routing

```bash
dekk apxm vllm enable <SERVED_MODEL_ID> --port 8916
```

`enable` verifies that `/v1/models` reports the served model id, verifies the
APXM graph route, adds the backend if needed, adds the served model id if
needed, and runs `dekk apxm backend test` unless `--skip-test` is set. It does
not start vLLM, download weights, or rewrite graph/chat routing policy.
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
dekk apxm vllm start /models/my-model \
  --served-model-name my-model-local \
  --port 8916 \
  --wait
dekk apxm vllm probe --port 8916
dekk apxm vllm enable my-model-local --port 8916
```

Do not enable the filesystem path unless `/v1/models` reports that exact string.

## Example: Hugging Face Cache

```bash
dekk apxm vllm download <HF_MODEL_ID> --hf-home /path/to/hf-cache
dekk apxm vllm start <HF_MODEL_ID> \
  --served-model-name <SERVED_MODEL_ID> \
  --hf-home /path/to/hf-cache \
  --port 8916 \
  --wait
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
  examples/python/_benchmarks/priority_scheduling.air
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
