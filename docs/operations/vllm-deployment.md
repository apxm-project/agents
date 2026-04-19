# vLLM Deployment Runbook

## 1. Overview

APXM's vLLM backend works against any OpenAI-compatible vLLM server. Both
stock (upstream) vLLM and the APXM-fork vLLM are supported. The APXM-fork
adds extension endpoints under `/v1/apxm/*` for graph-aware KV-cache pinning;
stock vLLM does not expose these. At health-check time, the runtime probes
`GET /v1/apxm/pins/stats` and, if it receives a 404, flips an internal
`apxm_endpoints_available` flag to `false`
(`crates/runtime/apxm-backends/src/llm/backends/vllm/backend.rs:380`). From
that point on, `register_graph`, `pin_prefix`, and `release_graph` become
silent no-ops -- the rest of the pipeline (completions, priority, hints)
continues to work normally.

## 2. Server Requirements

The following flags are required (or strongly recommended) when launching the
vLLM server. Each flag is explained with the reason APXM depends on it.

### `--enable-prefix-caching`

APXM's compiler detects shared prompt prefixes across parallel ASK nodes
(via the `PromptCanonicalization` pass at `-O2`) and tags them with a
`shared_prefix_group`. At runtime, the vLLM backend sends these hints so
the server can reuse KV-cache blocks across requests in the same group.
Without `--enable-prefix-caching`, the server computes KV blocks from
scratch for every request, negating the compiler's prefix-folding work.

### `--enable-auto-tool-choice --tool-call-parser <PARSER>`

APXM auto-attaches all registered capabilities (tools) to every ASK
operation at execution time
(`crates/runtime/apxm-runtime/src/executor/handlers/llm.rs:624-636`).
The request includes `"tool_choice": "auto"`. Stock vLLM rejects this
with an HTTP 400 unless both `--enable-auto-tool-choice` and a matching
`--tool-call-parser` are set. See section 3 for the correct parser value
for each model family.

### `--max-model-len <N>`

Must be greater than or equal to APXM's `context_window` setting for the
model (configured in `~/.apxm/config.toml` under `[[backends.models]]`).
If the vLLM model length is smaller than what APXM expects, requests that
approach the context window will fail server-side.

## 3. Tool-Call Parser by Model Family

Select the `--tool-call-parser` value that matches the model you are serving.

| Model family              | `--tool-call-parser` value |
|---------------------------|----------------------------|
| Qwen 2.5 / Qwen 3        | `hermes`                   |
| Qwen3-Coder               | `qwen3_coder`              |
| Llama 3.1 / 3.2 / 3.3     | `llama3_json`              |
| Llama 4                   | `llama4_pythonic`           |
| Mistral / Mixtral          | `mistral`                  |
| DeepSeek V3               | `deepseek_v3`              |
| DeepSeek V3.1             | `deepseek_v31`             |
| GLM-4.5 (4 MoE)           | `glm45`                    |
| GLM-4.7                   | `glm47`                    |
| Nous Hermes 2 Pro+         | `hermes`                   |
| InternLM2                  | `internlm`                 |
| IBM Granite 3.x            | `granite`                  |
| AI21 Jamba 1.5             | `jamba`                    |
| xLAM                      | `xlam`                     |
| Phi-4 Mini                 | `phi4_mini_json`           |

If your model family is not listed, check the vLLM documentation or run
`ls` inside the container to discover available parsers:

```sh
docker exec <container> ls /usr/local/lib/python3.12/dist-packages/vllm/tool_parsers/*.py
```

## 4. Endpoint URL Convention

The endpoint string passed to `dekk apxm backend add` **must** include the
`/v1` suffix. The runtime builds the completions URL by concatenating the
configured `base_url` with `/chat/completions`
(`crates/runtime/apxm-backends/src/llm/backends/openai/backend.rs:360`,
constant at `crates/core/apxm-core/src/constants.rs:405`). There is no
automatic `/v1` insertion.

Correct:

```
http://localhost:8000/v1
```

Incorrect (will 404):

```
http://localhost:8000
http://localhost:8000/v1/
```

This matches how OpenAI's own API stores the base URL (`https://api.openai.com/v1`).

## 5. Reference Docker Invocation

### GPU runtime (vendor GPU)

```sh
docker run -d \
  --name vllm-apxm \
  --device /dev/kfd \
  --device /dev/dri \
  --group-add video \
  --ipc host \
  --security-opt seccomp=unconfined \
  -p 8000:8000 \
  -v $HOME/.cache/huggingface:/root/.cache/huggingface \
  vllm/vllm-openai:latest \
  --model Qwen/Qwen3.5-4B \
  --max-model-len 32768 \
  --enable-prefix-caching \
  --enable-auto-tool-choice \
  --tool-call-parser hermes \
  --trust-remote-code
```

### CUDA (NVIDIA GPU)

```sh
docker run -d \
  --name vllm-apxm \
  --gpus all \
  --ipc host \
  -p 8000:8000 \
  -v $HOME/.cache/huggingface:/root/.cache/huggingface \
  vllm/vllm-openai:latest \
  --model Qwen/Qwen3.5-4B \
  --max-model-len 32768 \
  --enable-prefix-caching \
  --enable-auto-tool-choice \
  --tool-call-parser hermes \
  --trust-remote-code
```

Wait for the server to be ready before proceeding:

```sh
until curl -sf http://localhost:8000/v1/models > /dev/null; do sleep 2; done
echo "vLLM is ready"
```

## 6. Quickstart: Register and Smoke Test

```sh
dekk apxm backend add vllm-bench --type local --protocol vllm \
    --endpoint http://localhost:8000/v1

dekk apxm backend test vllm-bench

dekk apxm execute -O2 examples/python/_benchmarks/chained_llm.py
```

The first command registers the backend in `~/.apxm/config.toml`. The
second verifies connectivity (models endpoint + health check). The third
compiles and runs a benchmark workflow end to end.

## 7. Troubleshooting

### `OpenAI API error (status 404)` from `/chat/completions`

**Cause:** The endpoint in `~/.apxm/config.toml` is missing the `/v1`
suffix. The runtime appends `/chat/completions` directly to the base URL,
so `http://localhost:8000/chat/completions` becomes the target -- which
does not exist.

**Fix:** Re-register the backend with the `/v1` suffix:

```sh
dekk apxm backend remove vllm-bench
dekk apxm backend add vllm-bench --type local --protocol vllm \
    --endpoint http://localhost:8000/v1
```

### `"auto" tool choice requires --enable-auto-tool-choice and --tool-call-parser to be set` (HTTP 400)

**Cause:** The vLLM server was launched without the `--enable-auto-tool-choice`
and `--tool-call-parser` flags, or the parser name does not match the model
family. APXM auto-attaches registered capabilities as tools to every ASK
node and sends `"tool_choice": "auto"`
(`crates/runtime/apxm-runtime/src/executor/handlers/llm.rs:635`).

**Fix:** Restart the vLLM server with the correct flags. Consult the table
in section 3 to pick the parser that matches your model:

```sh
docker stop vllm-apxm && docker rm vllm-apxm
# Re-run the docker command from section 5 with the correct --tool-call-parser
```

### `WARNING [protocol.py:51] The following fields were present in the request but ignored: {'apxm'}` in vLLM logs

**Cause:** This is expected behavior on stock (upstream) vLLM. APXM sends
graph-aware scheduling hints in the `extra_body.apxm` field of every
request. Stock vLLM does not recognize this field and logs the warning.
The warning is harmless -- the completion still succeeds.

**Fix:** No action needed. The APXM runtime detects stock vLLM via the
Step 1c health probe (`/v1/apxm/pins/stats` returning 404) and silently
skips the graph lifecycle calls. To actually consume the hints for
KV-cache pinning, deploy the APXM-fork vLLM (see section 8).

### `Backend 'vllm-bench': connection refused` on `dekk apxm backend test`

**Cause:** The vLLM server is not running, not yet ready, or listening on
a different port than configured.

**Fix:**

1. Check that the container is running: `docker ps | grep vllm`
2. Check the port mapping matches the endpoint in config
3. Wait for the server to finish loading the model (large models can take
   several minutes):

```sh
docker logs vllm-apxm --tail 20
# Look for "Uvicorn running on http://0.0.0.0:8000"
```

## 8. APXM-Fork vLLM (Optional)

The APXM-fork of vLLM adds extension endpoints under `/v1/apxm/` that
enable graph-aware KV-cache management. Specifically, it exposes
`/v1/apxm/graphs/register` (upload graph topology before execution),
`/v1/apxm/pins` (pin KV blocks for prefix reuse across sibling nodes),
`/v1/apxm/pins/stats` (cache statistics), and
`/v1/apxm/graphs/{graph_id}` (release pinned blocks after execution).
These endpoints are consumed by `GraphAwareVllmBackend`'s
register/pin/release flow
(`crates/runtime/apxm-backends/src/llm/backends/vllm/backend.rs:165-250`).
The fork is currently a work-in-progress. Without it, the runtime
detects the absence of these endpoints at health-check time (Step 1c)
and all lifecycle calls become no-ops -- completions, priority mapping,
and `extra_body.apxm` hints continue to flow normally.
