# Backends and Models Architecture

> **Reference:** For the full configuration format, see [`docs/reference/config.md`](../reference/config.md).

## The Hierarchy

APXM has a three-level hierarchy for LLM infrastructure:

```
BACKEND  ── where inference physically runs
  └── MODEL  ── a specific LLM deployed on that backend
        └── ENDPOINT  ── how to reach it (protocol + URL + auth)
```

A single `~/.apxm/config.toml` is the only source of truth. There are no parallel systems, no `credentials.toml`, no `[[llm_backends]]`. Just `[[backends]]`.

---

## Backend Types

APXM supports three backend categories with different lifecycle, cost, and capability profiles:

### Cloud

SaaS APIs from major LLM providers. You pay per token; the provider manages everything else.

| Property | Value |
|----------|-------|
| Lifecycle | Always on (managed by provider) |
| Cost | Per-token |
| APXM Graph Hints | ❌ |
| Setup | API key only |

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

---

### On-Prem

Enterprise-managed endpoints. Fixed allocation or internal billing. You have an API but someone else manages the GPUs.

| Property | Value |
|----------|-------|
| Lifecycle | Managed by IT/infra |
| Cost | Internal billing |
| APXM Graph Hints | ✅ if running APXM-patched vLLM |
| Setup | API key + custom headers |

**Examples:** enterprise LLM gateway, Azure OpenAI, AWS Bedrock, private vLLM clusters

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

---

### Local

You own the full stack. You download the model, manage GPU allocation, start and stop the server.

| Property | Value |
|----------|-------|
| Lifecycle | **You manage it** |
| Cost | Hardware only |
| APXM Graph Hints | ✅ full support with APXM vLLM branch |
| Setup | Docker config + model path |

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

The `protocol` field determines the HTTP wire format APXM uses to talk to the backend:

| Protocol | Description | Graph Hints |
|----------|-------------|-------------|
| `openai` | OpenAI Chat Completions API (also used by OpenRouter, Together, etc.) | ❌ |
| `anthropic` | Anthropic Messages API | ❌ |
| `google` | Google Gemini API | ❌ |
| `ollama` | Ollama local API | ❌ |
| `vllm` | OpenAI-compatible + APXM graph extensions | ✅ |

### vLLM and Graph Hints

When `protocol = "vllm"`, APXM sends additional scheduling metadata with each request:

- `POST /v1/apxm/graphs/` — Register a graph for priority scheduling
- `GET /v1/apxm/graphs/{id}` — Check graph registration status
- Requests include `x-apxm-graph-id` and `x-apxm-critical-path` headers

This allows the vLLM scheduler (APXM branch) to prioritize critical-path operations in a running graph, reducing end-to-end latency.

---

## Model Metadata

Each model carries metadata used for routing, cost estimation, and capability checks:

```toml
[[backends.models]]
id = "claude-sonnet-4-5"        # Identifier sent to the API (required)
aliases = ["sonnet", "claude"]  # Alternative routing names
context_window = 200000         # Max tokens (0 = unknown)
cost_per_1k_input = 0.003       # USD per 1K input tokens
cost_per_1k_output = 0.015      # USD per 1K output tokens
supports_vision = true          # Image input support
supports_functions = true       # Tool/function calling
supports_thinking = false       # Extended reasoning (o1, Claude thinking)
tags = ["production", "smart"]  # Routing tags
```

### Tag Conventions

| Tag | Meaning |
|-----|---------|
| `production` | Approved for production workloads |
| `development` | Dev/testing only |
| `smart` | High capability, higher cost |
| `fast` | Low latency, lower capability |
| `cheap` | Cost-optimized |
| `local` | Self-hosted, no per-token cost |
| `vision` | Supports image inputs |
| `thinking` | Extended reasoning mode |
| `onprem` | Enterprise internal |

---

## Routing

Routing controls which backend and model handle each request. Configured under `[chat]` and `[chat.routing]`.

### Resolution Order

When selecting a backend/model for a request:

1. Explicit request override (request specifies backend/model directly)
2. `[chat.routing.operation_routes]` — per-AIS-operation routing rules
3. Model alias resolution (`[chat.routing.model_aliases]`)
4. `chat.default_model`
5. `chat.default_backend`
6. If selected backend is unhealthy → circuit breaker triggers fallback chain

### Operation Routes

Route specific AIS operations to specific backends:

```toml
[chat.routing.operation_routes.plan]
model = "claude-sonnet-4-5"     # Use this model for PLAN ops

[chat.routing.operation_routes.think]
backend = "local-gpu"       # Use local GPU for THINK ops

[chat.routing.operation_routes.ask]
backend = "corp-gateway"          # Cheap queries go on-prem
```

Routable operations: `plan`, `think`, `reason`, `reflect`, `ask`, `verify`

### Fallback Chains

When a backend becomes unhealthy, the circuit breaker redirects to the fallback chain:

```toml
[[chat.routing.fallback_chains]]
backend = "anthropic"
fallbacks = ["corp-gateway", "local-gpu"]
```

---

## Health Monitoring

APXM's ModelRouter continuously monitors backend health:

| State | Meaning | Behavior |
|-------|---------|----------|
| `Healthy` | Normal operation | Accept requests |
| `Degraded` | High latency or elevated error rate | Accept with warning |
| `Unhealthy` | Failed health check | Trigger fallback chain |

The circuit breaker opens after N consecutive failures (default: 5), half-opens after a timeout (default: 30s) to test recovery, and closes after a successful request.

---

## Docker Lifecycle (Local Backends)

Local backends with `[backends.docker]` config support full container lifecycle via CLI:

```bash
apxm backend start local-gpu    # Launch container
apxm backend status local-gpu   # Check if running
apxm backend logs local-gpu     # Tail logs
apxm backend stop local-gpu     # Shut down
apxm backend restart local-gpu  # Restart
```

The `DockerManager` translates the `[backends.docker]` config into `docker run` commands with the correct GPU device mappings, environment variables, model mounts, and tensor-parallel settings.

---

## CLI Reference

```bash
# Discovery
apxm backend list              # All registered backends
apxm models list               # All models across all backends
apxm models health             # ModelRouter health status

# Management
apxm backend add <name>        # Add a backend (interactive)
apxm backend remove <name>     # Remove a backend
apxm backend test [name]       # Test connectivity (all or specific)
apxm backend migrate           # Import from legacy credentials.toml

# Local backend lifecycle
apxm backend start <name>      # Start Docker container
apxm backend stop <name>       # Stop container
apxm backend status [name]     # Container status
apxm backend logs <name>       # Container logs (--tail N)
apxm backend restart <name>    # Restart container
```

---

## See Also

- [`docs/reference/config.md`](../reference/config.md) — Complete config file reference
- [`docs/architecture/backends-quickref.md`](backends-quickref.md) — One-page cheat sheet
