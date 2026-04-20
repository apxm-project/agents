# vLLM Deployment Runbook

## 1. Overview

APXM's vLLM backend works against any OpenAI-compatible vLLM server. Both
stock (upstream) vLLM and the APXM-fork vLLM are supported. The APXM-fork
adds extension endpoints under `/v1/apxm/*` for graph-aware KV-cache pinning;
stock vLLM does not expose these. At health-check time, the runtime probes
`GET /v1/apxm/pins/stats` and, if it receives a 404, flips an internal
`apxm_endpoints_available` flag to `false`
(`crates/runtime/apxm-backends/src/llm/backends/vllm/backend.rs:401`). From
that point on, `register_graph`, `pin_prefix`, and `release_graph` become
silent no-ops -- the rest of the pipeline (completions, priority, hints)
continues to work normally.

The same `supports_graph_extensions()` typed capability
(`traits.rs:101`) drives lifecycle wiring: at `Runtime::execute()` time,
`find_graph_aware_backends()` (`registry/mod.rs:630`) discovers any
graph-aware backend and constructs a `VllmGraphLifecycle` guard
(`crates/runtime/apxm-runtime/src/vllm_lifecycle.rs`) that calls
`register_graph` once, then `release_graph` on either explicit
completion (`runtime.rs:317, :434`) or `Drop` (panic / future
cancellation). The Drop path uses a `tokio::spawn` best-effort detached
release; the happy-path explicit release is mandatory.

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
(`crates/runtime/apxm-runtime/src/executor/handlers/llm.rs:619-672`).
The request includes `"tool_choice": "auto"`. Stock vLLM rejects this
with an HTTP 400 unless both `--enable-auto-tool-choice` and a matching
`--tool-call-parser` are set. See section 3 for the correct parser value
for each model family.

As of 2026-04, the runtime does **not** wait for the 400 -- it
**pre-flights** the request via the typed `supports_auto_tool_choice()`
capability (`traits.rs:101`, `vllm/backend.rs:436`) and hard-fails with
an actionable error before the request leaves
(`handlers/llm.rs:653-665`). See `auto_tool_choice` below for the opt-out
path.

### `--max-model-len <N>`

Must be greater than or equal to APXM's `context_window` setting for the
model (configured in `~/.apxm/config.toml` under `[[backends.models]]`).
If the vLLM model length is smaller than what APXM expects, requests that
approach the context window will fail server-side.

### Per-backend `auto_tool_choice = false` opt-out

If you must run stock vLLM **without** `--enable-auto-tool-choice` (or
hit a server that rejects tool routing for any other reason), declare it
in `~/.apxm/config.toml` so the runtime stops sending
`tool_choice="auto"` for that backend:

```toml
[[backends]]
name = "vllm-bench"
type = "local"
protocol = "vllm"
endpoint = "http://localhost:8000/v1"
auto_tool_choice = false
```

The field lives at `BackendConfig.auto_tool_choice: Option<bool>`
(`crates/core/apxm-core/src/types/backend.rs:97`), is wired into
`GraphAwareVllmBackend::auto_tool_choice_supported`
(`vllm/backend.rs:128, :151-168`), and is what `supports_auto_tool_choice()`
returns. With this set, ASK nodes that have tools attached will fail at
`handlers/llm.rs:653` with the same actionable message — but you can
remove the tools from your graph (or the auto-attach in `[capabilities]`)
and the rest of the pipeline keeps working.

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
  --model Qwen/Qwen2.5-7B-Instruct \
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
  --model Qwen/Qwen2.5-7B-Instruct \
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

## 6. Quickstart: Register, Smoke Test, Run Benchmarks

### 6.1 Register the backend

```sh
dekk apxm backend add vllm-bench --type local --protocol vllm \
    --endpoint http://localhost:8000/v1

dekk apxm backend test vllm-bench
```

The first command writes an entry to `~/.apxm/config.toml`. The name
`vllm-bench` is **not arbitrary** — it must match
`examples/python/_benchmarks/_config.py::VLLM_BACKEND`, which is the
single source of truth for the registered name across all benchmarks.
The second command verifies connectivity (models endpoint + health
check, which also flips `apxm_endpoints_available` if the APXM-fork
endpoints are absent).

### 6.2 Run the demo benchmarks

The three reference benchmarks are pre-wired with typed routing
(`@compile(default_provider=VLLM, default_backend=VLLM_BACKEND,
default_model=Vllm.QWEN_2_5_7B)` — no string literals):

```sh
# Single fan-out (8 parallel ASK nodes sharing a 4 KB prefix)
dekk apxm execute -O2 examples/python/_benchmarks/prefix_fanout_large.py

# Smaller fan-out (4-way)
dekk apxm execute -O2 examples/python/_benchmarks/shared_prefix_fanout.py

# Sequential ASK→THINK→REASON chain (covers tool-call retry hint preservation)
dekk apxm execute -O2 examples/python/_benchmarks/chained_llm.py

# Three-graph suite (writes a markdown comparison report)
python3 scripts/benchmark_e2e.py --opt-level 2 \
    --graph shared_prefix_fanout --graph prefix_fanout_large --graph chained_llm \
    --real -o vllm_integration_results.md
```

`--emit-session` is **default-on** as of 2026-04 — every run writes a
manifest under `.apxm/sessions/{graph_name}-{YYYYMMDDTHHmmss}/`. Use
`--no-emit-session` to opt out (`apxm-cli/src/commands/cli.rs:84-110`).

### 6.3 What `-O2` adds

The `vllm_hints` compiler pass
(`crates/compiler/apxm-compiler/src/passes/vllm_hints.rs`, registered at
`pipeline.rs:112`) runs at `-O1` and above. It reads MLIR-stamped
attributes (`ais.shared_prefix_group`, `ais.shared_prefix_est_tokens`,
`ais.warmup_candidate`, `ais.downstream_nodes` — see
`apxm-ais/src/attrs.rs:154-168`) and writes runtime-consumable
`_vllm_*` attributes that the runtime materializes into the
`extra_body.apxm` field of every chat completion. Without `-O2`, these
attributes are absent and vLLM behaves as a vanilla OpenAI-compatible
endpoint.

The full chain MLIR ↔ Rust ↔ Python uses three constant registries that
are kept in sync by a build-time drift-detector test
(`apxm-ais/src/attrs.rs:342-360`):

| Layer  | File                                                                           |
|--------|--------------------------------------------------------------------------------|
| MLIR   | `crates/compiler/apxm-compiler/mlir/include/ais/Common/Constants.h:115-119`    |
| Rust   | `crates/core/apxm-ais/src/attrs.rs:154-168`                                    |
| Python | `crates/compiler/apxm-frontend/python/apxm/_generated/constants.py:116-127`    |

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

### `Backend 'X' does not support tool_choice="auto"` (APXM pre-flight error)

**Cause:** APXM auto-attaches registered capabilities as tools to every
ASK node and would have sent `"tool_choice": "auto"`
(`crates/runtime/apxm-runtime/src/executor/handlers/llm.rs:672`). A typed
capability check at `:653` saw `supports_auto_tool_choice() == false` and
hard-failed before the request left the runtime. The full message tells
you exactly which knobs to flip.

**Fix:** Choose one of the three options the error message offers:

1. **Restart vLLM with the right flags** (preferred for APXM-fork or
   any production deployment):

   ```sh
   docker stop vllm-apxm && docker rm vllm-apxm
   # Re-run the docker command from section 5 with --enable-auto-tool-choice
   # plus the correct --tool-call-parser from section 3
   ```

2. **Mark the backend as tool-choice-incapable** in `~/.apxm/config.toml`:

   ```toml
   [[backends]]
   name = "vllm-bench"
   auto_tool_choice = false
   ```

   The runtime stops sending `tool_choice="auto"` and your ASK nodes
   without tools continue to work. ASK nodes with tools attached still
   fail (with the same message) — those need either option 1 or 3.

3. **Remove the tool from this node** (or disable the auto-attach for
   this graph) so no tool routing is needed.

### `"auto" tool choice requires --enable-auto-tool-choice and --tool-call-parser to be set` (HTTP 400)

**Cause:** The pre-flight check above did not fire (e.g., the backend
was created before the capability was registered, or the field was set
to `true` in TOML against a server that doesn't actually support it),
and the request reached the server. The fix is the same — see the entry
above.

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
enable graph-aware KV-cache management. Specifically, it exposes:

| Endpoint                          | Purpose                                                        | Backend method                  |
|-----------------------------------|----------------------------------------------------------------|---------------------------------|
| `POST /v1/apxm/graphs/register`   | Upload graph topology before execution                          | `register_graph` (`backend.rs:186`) |
| `POST /v1/apxm/pins`              | Pin KV blocks for prefix reuse across sibling nodes             | `pin_prefix` (`backend.rs:255`)     |
| `GET  /v1/apxm/pins/stats`        | Cache statistics; also probed by `health_check` for capability  | `health_check` (`backend.rs:390`)   |
| `DELETE /v1/apxm/graphs/{id}`     | Release pinned blocks after execution                            | `release_graph` (`backend.rs:221`)  |

Lifecycle is owned by `VllmGraphLifecycle`
(`crates/runtime/apxm-runtime/src/vllm_lifecycle.rs`):

- **Construction** at `Runtime::execute()` (`runtime.rs:300, :424`) —
  picks any backend with `supports_graph_extensions()` via
  `find_graph_aware_backends()` (`registry/mod.rs:630`), calls
  `register_graph` once, returns a guard.
- **Explicit release** at `runtime.rs:317, :434` on the happy path
  (mandatory; see Rule 5 of the integration plan).
- **Drop guard** as panic / cancellation safety net — uses
  `tokio::spawn` for a best-effort detached `release_graph`. Will silently
  no-op if the runtime is shutting down (so the explicit release on the
  happy path is non-optional).

Three integration tests cover this surface against a `wiremock` HTTP
server (no live vLLM needed):

- `crates/runtime/apxm-backends/tests/vllm_lifecycle_http.rs` — full
  register/completions/release happy path
- `crates/runtime/apxm-runtime/tests/vllm_lifecycle_drop.rs` — Drop-guard
  fires release on (a) explicit happy path, (b) bare drop, (c)
  `tokio::spawn` task abort
- `crates/runtime/apxm-runtime/tests/tool_loop_hint_preservation.rs` —
  `apxm_hints` preserved across all N retries of a tool-using ASK node
  (`handlers/llm.rs:1101-1102`)

Run with: `cargo test -p apxm-backends --test vllm_lifecycle_http` and
`cargo test -p apxm-runtime --test vllm_lifecycle_drop`.

The fork itself is currently a work-in-progress. Without it, the runtime
detects the absence of these endpoints at health-check time and all
lifecycle calls become no-ops — completions, priority mapping, and
`extra_body.apxm` hints continue to flow normally (vLLM logs them as
ignored unknown fields; harmless).

## 9. Built-in vLLM Models

The Rust source-of-truth `BUILTIN_MODELS`
(`crates/core/apxm-core/src/types/model_spec.rs:170-188`) has four
vLLM-tagged entries that codegen into the typed Python class
`Vllm` (`crates/compiler/apxm-frontend/python/apxm/_generated/models.py`):

| Python alias              | Model id                              |
|---------------------------|---------------------------------------|
| `Vllm.QWEN_2_5_7B`        | `Qwen/Qwen2.5-7B-Instruct` (default)  |
| `Vllm.QWEN_2_5_14B`       | `Qwen/Qwen2.5-14B-Instruct`           |
| `Vllm.LLAMA_3_1_8B`       | `meta-llama/Llama-3.1-8B-Instruct`    |
| `Vllm.LLAMA_3_1_70B`      | `meta-llama/Llama-3.1-70B-Instruct`   |

Used in benchmarks like:

```python
from apxm import compile, GraphRecorder, Vllm
from ._config import VLLM, VLLM_BACKEND

@compile(default_provider=VLLM, default_backend=VLLM_BACKEND, default_model=Vllm.QWEN_2_5_7B)
def prefix_fanout_large(g: GraphRecorder):
    ...
```

To serve a different model, override `default_model` per `@compile()`,
or pass `model=...` per `g.ask()`. New models added to `BUILTIN_MODELS`
are picked up automatically by `dekk apxm codegen`.

## 10. Acceptance Checklist

When validating a fresh deployment, the following should all hold:

1. `dekk apxm backend test vllm-bench` succeeds (HTTP 200 on
   `/v1/models`); WARN logged once at first run if the APXM-fork
   endpoints are absent.
2. `dekk apxm execute -O2 examples/python/_benchmarks/prefix_fanout_large.py`
   completes; manifest written under `.apxm/sessions/`.
3. Server logs show `POST /v1/chat/completions × 8` with `extra_body.apxm.*`
   populated (`graph_id`, `node_id`, `priority_class`, `reuse_group`).
4. APXM-fork only: `POST /v1/apxm/graphs/register × 1` and
   `DELETE /v1/apxm/graphs/{id} × 1` (also fires on Ctrl-C via Drop
   guard).
5. `chained_llm.py`: every tool-call retry has `apxm_hints` populated
   (Step 3b regression test guarantees this in CI).
6. Lint: `grep -rE '"_?vllm[-_]?[a-z_]*"' crates/ examples/python/_benchmarks/`
   matches **only** inside the four constant registries (`Constants.h`,
   `apxm-ais/src/attrs.rs`, `_generated/constants.py`,
   `_benchmarks/_config.py`).
