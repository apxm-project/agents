# Backends and Models

> **Full configuration reference:** [config.md](../reference/config.md) covers every field, default, and example for `~/.apxm/config.toml`.

## The Hierarchy

APXM organizes LLM infrastructure into three levels:

```
BACKEND  -- where inference physically runs
  +-- MODEL  -- a specific LLM deployed on that backend
        +-- ENDPOINT  -- how to reach it (protocol + URL + auth)
```

A single `~/.apxm/config.toml` file is the only source of truth.

---

## Backend Types

APXM supports three backend categories, each with a distinct lifecycle and cost model.

### Cloud

SaaS APIs from major LLM providers. You pay per token; the provider manages infrastructure.

**Examples:** Anthropic (Claude), OpenAI (GPT), Google (Gemini)

```toml
[[backends]]
name = "anthropic"
type = "cloud"
protocol = "anthropic"
api_key = "env:ANTHROPIC_API_KEY"

[[backends.models]]
id = "claude-sonnet-4-5"
context_window = 200000
tags = ["production", "smart"]
```

### On-Prem

Enterprise-managed endpoints with internal billing. You have an API but someone else manages the GPUs.

**Examples:** Enterprise LLM gateway, Azure OpenAI, AWS Bedrock, private vLLM clusters

```toml
[[backends]]
name = "corp-gateway"
type = "onprem"
protocol = "openai"
endpoint = "https://llm.example.com/v1"
api_key = "env:OCP_APIM_KEY"

[backends.headers]
X-Custom-Gateway-Key = "env:OCP_APIM_KEY"
user = "env:USER"

[[backends.models]]
id = "GPT-oss-20B"
tags = ["onprem", "internal"]
```

### Local

You own the full stack: download the model, allocate GPUs, start and stop the server.

**Examples:** vLLM on GPU, Ollama, llama.cpp

```toml
[[backends]]
name = "local-gpu"
type = "local"
protocol = "vllm"
endpoint = "http://localhost:8000"

[backends.docker]
image = "gpu/pytorch-private:vllm-v0.14.0"
model_path = "/shared_inference/models/Google/Gemma-3-27b-it"
tensor_parallel = 4

[backends.docker.env]
HIP_VISIBLE_DEVICES = "0,1,2,3"
HSA_OVERRIDE_GFX_VERSION = "9.4.2"

[[backends.models]]
id = "/shared_inference/models/Google/Gemma-3-27b-it"
aliases = ["gemma3", "local"]
context_window = 8192
supports_vision = true
tags = ["local", "free"]
```

---

## Protocols

The `protocol` field selects the HTTP wire format APXM uses to talk to the backend:

| Protocol | Description | Graph Hints |
|----------|-------------|-------------|
| `openai` | OpenAI Chat Completions API (also used by OpenRouter, Together, etc.) | No |
| `anthropic` | Anthropic Messages API | No |
| `google` | Google Gemini API | No |
| `ollama` | Ollama local API (`/api/chat`, streaming) | No |
| `vllm` | OpenAI-compatible + APXM graph extensions | Yes |

### vLLM and Graph Hints

When `protocol = "vllm"`, APXM sends additional scheduling metadata with each request:

- `POST /v1/apxm/graphs/register` -- Register a graph for priority scheduling
- `DELETE /v1/apxm/graphs/{graph_id}` -- Release KV-cache for a completed graph
- Requests include hints via the `extra_body.apxm` JSON field (graph ID, critical-path flag, etc.)

This allows the vLLM scheduler (APXM branch) to prioritize critical-path operations, reducing end-to-end latency. See [vLLM Integration](../integrations/vllm.md) for setup details.

---

## Routing

Routing determines which backend and model handle each request. APXM resolves routing through a priority chain -- see [Routing Resolution Order](../reference/config.md#routing-resolution-order) in the config reference for the full sequence.

Key concepts:

- **Operation routes** direct specific AIS operations (e.g., `plan`, `think`, `ask`) to specific backends or models.
- **Model aliases** create shortcuts like `fast` or `smart` that map to concrete model/backend pairs.
- **Fallback chains** redirect traffic when a backend becomes unhealthy.

All routing configuration lives under `[chat.routing]` in `config.toml`. See [config.md](../reference/config.md) for the full specification and examples.

---

## Health Monitoring

APXM's `ModelRouter` continuously monitors backend health:

| State | Meaning | Behavior |
|-------|---------|----------|
| `Unknown` | Not enough requests recorded | Accept requests |
| `Healthy` | Normal operation | Accept requests |
| `Degraded` | High latency or elevated error rate | Accept with warning |
| `Unhealthy` | Failed health check | Trigger fallback chain |

The circuit breaker (in `apxm-runtime::ModelRouter`) is a separate mechanism: it opens after N consecutive failures (default: 5), half-opens after a timeout (default: 30s) to test recovery, and closes after a successful request.

---

## Docker Lifecycle (Local Backends)

Local backends with `[backends.docker]` config support full container lifecycle:

```bash
apxm backend start local-gpu    # Launch container
apxm backend status local-gpu   # Check if running
apxm backend logs local-gpu     # Tail logs
apxm backend stop local-gpu     # Shut down
apxm backend restart local-gpu  # Restart
```

The `DockerManager` translates `[backends.docker]` config into `docker run` commands with the correct GPU device mappings, environment variables, model mounts, and tensor-parallel settings.

---

## See Also

- [Configuration Reference](../reference/config.md) -- Complete `config.toml` field reference, routing resolution order, model metadata, and CLI commands
- [Multi-Model Routing](multi-model.md) -- Per-node model selection and cost/quality trade-offs
- [vLLM Integration](../integrations/vllm.md) -- Graph-aware scheduling with vLLM
- [Getting Started](../getting-started/installation.md) -- Initial setup and backend registration
