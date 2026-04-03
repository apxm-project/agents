# Backends and Models Architecture

This document defines the conceptual hierarchy for LLM infrastructure in APXM.

## Core Concepts

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              BACKEND                                     │
│  The infrastructure where inference runs                                 │
│  Examples: Anthropic Cloud, vendor On-Prem, Local vLLM on GPU           │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│   ┌─────────────────────────────────────────────────────────────────┐   │
│   │                           MODEL                                  │   │
│   │  A specific LLM deployed on this backend                        │   │
│   │  Examples: claude-sonnet-4-5, Gemma-3-27B-it, GPT-4o            │   │
│   ├─────────────────────────────────────────────────────────────────┤   │
│   │                                                                  │   │
│   │   ┌─────────────────────────────────────────────────────────┐   │   │
│   │   │                      ENDPOINT                            │   │   │
│   │   │  How to reach this model                                 │   │   │
│   │   │  • Protocol: OpenAI, Anthropic, vLLM                    │   │   │
│   │   │  • URL: https://api.anthropic.com                       │   │   │
│   │   │  • Auth: API key, headers                               │   │   │
│   │   └─────────────────────────────────────────────────────────┘   │   │
│   │                                                                  │   │
│   │   Metadata:                                                      │   │
│   │   • Context window: 200,000 tokens                              │   │
│   │   • Capabilities: vision, function calling                      │   │
│   │   • Cost: $0.003/1K input, $0.015/1K output                    │   │
│   │   • Tags: [production, smart]                                   │   │
│   │                                                                  │   │
│   └─────────────────────────────────────────────────────────────────┘   │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

## Backend Types

APXM supports three categories of backends, each with different characteristics:

### 1. Cloud Backends

**What:** SaaS APIs from major LLM providers. Pay-per-token pricing.

**Examples:**
- **Anthropic** — Claude models (Sonnet, Opus, Haiku)
- **OpenAI** — GPT models (GPT-4o, GPT-4o-mini, o1)
- **Google** — Gemini models (Gemini Pro, Gemini Flash)

**Characteristics:**
| Property | Value |
|----------|-------|
| Lifecycle | Always available (managed by provider) |
| Cost Model | Per-token (input + output) |
| Reliability | High SLA (99.9%+) |
| Latency | Variable (network + queue) |
| APXM Graph Hints | ❌ Not supported |
| Setup | API key only |

**Configuration:**
```toml
[[backends]]
name = "anthropic"
type = "cloud"
protocol = "anthropic"
endpoint = "https://api.anthropic.com"
api_key = "env:ANTHROPIC_API_KEY"

[[backends.models]]
id = "claude-sonnet-4-5"
context_window = 200000
cost_per_1k_input = 0.003
cost_per_1k_output = 0.015
tags = ["production", "smart"]
```

---

### 2. On-Prem / Managed Backends

**What:** Enterprise-managed LLM endpoints. Fixed cost or internal billing.

**Examples:**
- **enterprise LLM gateway** — Internal vendor model hosting
- **Azure OpenAI** — Azure-hosted OpenAI models
- **AWS Bedrock** — AWS-managed model access
- **Private vLLM clusters** — Company-managed GPU clusters

**Characteristics:**
| Property | Value |
|----------|-------|
| Lifecycle | Managed by IT/infra team |
| Cost Model | Internal billing / fixed allocation |
| Reliability | Depends on deployment |
| Latency | Often lower (same network) |
| APXM Graph Hints | ✅ If running APXM-patched vLLM |
| Setup | API key + custom headers |

**Configuration:**
```toml
[[backends]]
name = "corp-gateway"
type = "onprem"
protocol = "openai"  # OpenAI-compatible API
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

### 3. Local / Self-Hosted Backends

**What:** You control the infrastructure. Run your own inference server.

**Examples:**
- **vLLM on GPU** — High-performance vendor GPU inference
- **vLLM on NVIDIA** — CUDA-based inference
- **Ollama** — Easy local model running
- **llama.cpp** — CPU/Metal inference

**Characteristics:**
| Property | Value |
|----------|-------|
| Lifecycle | **You manage it** (start/stop) |
| Cost Model | Hardware cost only (no per-token) |
| Reliability | Depends on your setup |
| Latency | Lowest (local network) |
| APXM Graph Hints | ✅ Full support with APXM vLLM branch |
| Setup | Docker container, model download |

**Configuration:**
```toml
[[backends]]
name = "local-gpu"
type = "local"
protocol = "vllm"  # OpenAI-compatible + APXM graph hints
endpoint = "http://localhost:8000"

# Lifecycle management (optional)
[backends.docker]
image = "gpu/vllm:latest"
args = ["--device", "/dev/kfd", "--device", "/dev/dri"]
env = { HIP_VISIBLE_DEVICES = "0,1,2,3" }
model_path = "/models/Google/Gemma-3-27b-it"
tensor_parallel = 4

[[backends.models]]
id = "/models/Google/Gemma-3-27b-it"
context_window = 8192
tags = ["local", "free", "vision"]
```

---

## Protocol Types

The `protocol` field determines how APXM communicates with the backend:

| Protocol | Description | Streaming | Functions | Graph Hints |
|----------|-------------|-----------|-----------|-------------|
| `openai` | OpenAI Chat Completions API | ✅ | ✅ | ❌ |
| `anthropic` | Anthropic Messages API | ✅ | ✅ | ❌ |
| `google` | Google Gemini API | ✅ | ✅ | ❌ |
| `ollama` | Ollama native API | ✅ | ✅ | ❌ |
| `vllm` | OpenAI-compatible + APXM extensions | ✅ | ✅ | ✅ |

### vLLM Protocol

The `vllm` protocol extends OpenAI compatibility with APXM-specific features:

- **Graph Registration:** `POST /v1/apxm/graphs/` — Register a graph for priority scheduling
- **Graph Status:** `GET /v1/apxm/graphs/{id}` — Check registered graph
- **Graph Deletion:** `DELETE /v1/apxm/graphs/{id}` — Unregister graph
- **Priority Hints:** Requests include `x-apxm-graph-id` and `x-apxm-critical-path` headers

This enables the vLLM scheduler to prioritize critical-path operations in APXM graphs.

---

## Model Metadata

Each model has associated metadata for routing decisions:

```toml
[[backends.models]]
id = "claude-sonnet-4-5"           # Canonical model identifier
aliases = ["sonnet", "claude-4"]    # Alternative names
context_window = 200000             # Max tokens
cost_per_1k_input = 0.003          # USD per 1K input tokens
cost_per_1k_output = 0.015         # USD per 1K output tokens
supports_vision = true              # Can process images
supports_functions = true           # Supports tool/function calling
tags = ["production", "smart"]      # Routing tags
```

### Routing Tags

Tags enable policy-based model selection:

| Tag | Meaning |
|-----|---------|
| `production` | Approved for production workloads |
| `development` | Development/testing only |
| `smart` | High capability, higher cost |
| `fast` | Low latency, lower capability |
| `cheap` | Cost-optimized |
| `local` | Runs locally (no API costs) |
| `vision` | Supports image inputs |

---

## Routing Configuration

The `[routing]` section configures how APXM selects backends and models:

```toml
[routing]
# Global defaults
default_backend = "anthropic"
default_model = "claude-sonnet-4-5"

# Tag-based selection
prefer_tags = ["production", "smart"]   # First choice
fallback_tags = ["local", "cheap"]      # When preferred unavailable

# Operation-specific routing
[routing.operations]
plan = { model = "claude-sonnet-4-5" }      # Planning uses Sonnet
think = { backend = "local-gpu" }        # Thinking uses local Gemma
reason = { model = "claude-sonnet-4-5" }     # Reasoning uses Sonnet
ask = { backend = "corp-gateway" }             # Simple queries use on-prem

# Fallback chains
[[routing.fallbacks]]
backend = "anthropic"
fallbacks = ["corp-gateway", "local-gpu"]

[[routing.fallbacks]]
backend = "local-gpu"
fallbacks = ["corp-gateway"]
```

### Routing Resolution Order

When selecting a backend/model for a request:

1. **Explicit request** — If the request specifies a model, use it
2. **Operation route** — Check `routing.operations` for this AIS operation type
3. **Model alias** — Resolve aliases (e.g., "fast" → "gpt-4o-mini")
4. **Tag matching** — Select from models matching `prefer_tags`
5. **Default model** — Fall back to `routing.default_model`
6. **Default backend** — Fall back to `routing.default_backend`

If the selected backend is unhealthy, the circuit breaker triggers fallback chain resolution.

---

## Health Monitoring

APXM continuously monitors backend health:

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│  anthropic  │     │  corp-gateway │     │local-gpu│
│   HEALTHY   │     │   HEALTHY   │     │  DEGRADED   │
│  latency:   │     │  latency:   │     │  latency:   │
│   120ms     │     │    45ms     │     │   2100ms    │
└─────────────┘     └─────────────┘     └─────────────┘
```

### Health States

| State | Meaning | Behavior |
|-------|---------|----------|
| `Healthy` | Normal operation | Accept requests |
| `Degraded` | High latency or errors | Accept with warning |
| `Unhealthy` | Failed health check | Trigger fallback |
| `Unknown` | No health data yet | Treat as healthy |

### Circuit Breaker

Each backend has a circuit breaker that:
- **Opens** after N consecutive failures (default: 5)
- **Half-opens** after timeout (default: 30s) to test recovery
- **Closes** after successful request in half-open state

---

## CLI Commands

### Backend Management

```bash
# List all backends
apxm backend list

# Add a new backend
apxm backend add anthropic \
  --protocol anthropic \
  --api-key "$ANTHROPIC_API_KEY"

# Add local vLLM backend
apxm backend add gemma3-local \
  --protocol vllm \
  --endpoint http://localhost:8000

# Test backend connectivity
apxm backend test anthropic

# Check health of all backends
apxm backend health

# Start a local backend (if docker config exists)
apxm backend start local-gpu

# Stop a local backend
apxm backend stop local-gpu
```

### Model Management

```bash
# List all models across backends
apxm model list

# List models for a specific backend
apxm model list --backend anthropic

# Add a model to a backend
apxm model add claude-sonnet-4-5 \
  --backend anthropic \
  --context-window 200000 \
  --cost-input 0.003 \
  --cost-output 0.015 \
  --tags production,smart

# Show model details
apxm model show claude-sonnet-4-5
```

---

## Complete Example Configuration

```toml
# ~/.apxm/config.toml

# ═══════════════════════════════════════════════════════════════════════════
# BACKENDS
# ═══════════════════════════════════════════════════════════════════════════

# Cloud: Anthropic
[[backends]]
name = "anthropic"
type = "cloud"
protocol = "anthropic"
endpoint = "https://api.anthropic.com"
api_key = "env:ANTHROPIC_API_KEY"

[[backends.models]]
id = "claude-sonnet-4-5"
aliases = ["sonnet", "claude"]
context_window = 200000
cost_per_1k_input = 0.003
cost_per_1k_output = 0.015
supports_vision = true
supports_functions = true
tags = ["production", "smart"]

[[backends.models]]
id = "claude-haiku-4-5"
aliases = ["haiku", "fast"]
context_window = 200000
cost_per_1k_input = 0.00025
cost_per_1k_output = 0.00125
supports_functions = true
tags = ["fast", "cheap"]

# On-Prem: Enterprise Internal
[[backends]]
name = "corp-gateway"
type = "onprem"
protocol = "openai"
endpoint = "https://llm.example.com/v1"
api_key = "env:OCP_APIM_KEY"

[backends.headers]
X-Custom-Gateway-Key = "env:OCP_APIM_KEY"

[[backends.models]]
id = "GPT-oss-20B"
tags = ["onprem", "internal"]

# Local: vLLM on GPU
[[backends]]
name = "local-gpu"
type = "local"
protocol = "vllm"
endpoint = "http://localhost:8000"

[backends.docker]
image = "gpu/pytorch-private:vllm-v0.14.0_amd_dev_aiter_nixl_ravgupta"
args = [
  "--device", "/dev/kfd",
  "--device", "/dev/dri",
  "--ipc", "host",
  "--group-add", "video",
  "-v", "/models:/models"
]
env = { HIP_VISIBLE_DEVICES = "0,1,2,3" }
command = [
  "vllm", "serve", "/models/Google/Gemma-3-27b-it",
  "--tensor-parallel-size", "4",
  "--port", "8000"
]

[[backends.models]]
id = "/models/Google/Gemma-3-27b-it"
aliases = ["gemma3", "local"]
context_window = 8192
supports_vision = true
tags = ["local", "free", "vision"]

# ═══════════════════════════════════════════════════════════════════════════
# ROUTING
# ═══════════════════════════════════════════════════════════════════════════

[routing]
default_backend = "anthropic"
default_model = "claude-sonnet-4-5"
prefer_tags = ["production"]
fallback_tags = ["local"]

[routing.operations]
plan = { model = "claude-sonnet-4-5" }
think = { backend = "local-gpu" }
reason = { model = "claude-sonnet-4-5" }
reflect = { model = "claude-sonnet-4-5" }
ask = { backend = "corp-gateway" }

[[routing.fallbacks]]
backend = "anthropic"
fallbacks = ["corp-gateway", "local-gpu"]

[[routing.fallbacks]]
backend = "corp-gateway"
fallbacks = ["local-gpu"]
```

---

## Migration from credentials.toml

The legacy `~/.apxm/credentials.toml` format is still supported but deprecated:

```toml
# OLD: credentials.toml (deprecated)
[credentials.anthropic]
provider = "anthropic"
api_key = "sk-..."
model = "claude-sonnet-4-5"

# NEW: config.toml (recommended)
[[backends]]
name = "anthropic"
type = "cloud"
protocol = "anthropic"
api_key = "sk-..."

[[backends.models]]
id = "claude-sonnet-4-5"
```

Run `apxm config migrate` to convert automatically.
