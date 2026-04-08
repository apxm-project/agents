# Configuration Reference

Complete reference for `~/.apxm/config.toml` -- the single source of truth for APXM configuration.

For conceptual background on backends and models, see the [Backends Guide](../guides/backends.md).

---

## File Locations

| Path | Purpose |
|------|---------|
| `~/.apxm/config.toml` | User-level configuration (backends, routing, tools) |
| `.apxm/config.toml` | Project-level overrides (merged with user config) |

**Load order:** Project config takes precedence over user config. Fields in project config overwrite the same fields in user config; array sections like `[[backends]]` are merged by `name`.

---

## Backends (`[[backends]]`)

Each `[[backends]]` entry defines a target for LLM inference.

### Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Unique identifier for this backend |
| `type` | string | One of `cloud`, `onprem`, `local` |
| `protocol` | string | Wire protocol (see [Protocols](#protocols)) |

### Optional Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `endpoint` | string | Protocol-dependent | API base URL |
| `api_key` | string | -- | Direct value or `env:VAR_NAME` reference |
| `headers` | table | `{}` | Custom HTTP headers (values support `env:VAR_NAME`) |

```toml
[[backends]]
name = "anthropic"
type = "cloud"
protocol = "anthropic"
api_key = "env:ANTHROPIC_API_KEY"

[backends.headers]
X-Custom-Header = "env:SOME_VAR"
```

### Backend Types

| Type | Description | `api_key` required? | Docker lifecycle? |
|------|-------------|---------------------|-------------------|
| `cloud` | SaaS providers (Anthropic, OpenAI, Google) | Yes | No |
| `onprem` | Enterprise gateways (Azure OpenAI, corporate proxies) | Yes | No |
| `local` | Self-hosted (vLLM, Ollama) | Usually no | Yes |

### Protocols

| Protocol | For | Default endpoint |
|----------|-----|------------------|
| `anthropic` | Claude models | -- |
| `openai` | GPT + OpenAI-compatible | -- |
| `google` | Gemini models | -- |
| `ollama` | Local Ollama | `http://localhost:11434` |
| `vllm` | vLLM (+ APXM graph hints) | `http://localhost:8000` |

For vLLM graph-aware scheduling details, see the [vLLM Integration](../integrations/vllm.md).

---

## Models (`[[backends.models]]`)

Models are nested under backends. Each model describes a specific LLM deployed on that backend.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `id` | string | **(required)** | Model identifier sent to the API |
| `aliases` | string[] | `[]` | Alternative routing names |
| `context_window` | integer | `0` | Maximum token window (`0` = unknown) |
| `cost_per_1k_input` | float | `0.0` | USD per 1K input tokens |
| `cost_per_1k_output` | float | `0.0` | USD per 1K output tokens |
| `supports_vision` | bool | `false` | Image input support |
| `supports_functions` | bool | `false` | Tool/function calling |
| `supports_thinking` | bool | `false` | Extended reasoning (o1-style, Claude extended thinking) |
| `tags` | string[] | `[]` | Labels for routing rules |

### Tag Conventions

| Tag | Meaning |
|-----|---------|
| `production` | Approved for production |
| `development` | Dev/testing only |
| `smart` | High capability |
| `fast` | Low latency |
| `cheap` | Cost-optimized |
| `local` | Self-hosted |
| `vision` | Supports images |
| `thinking` | Extended reasoning |
| `onprem` | Enterprise internal |

### Example

```toml
[[backends]]
name = "anthropic"
type = "cloud"
protocol = "anthropic"
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
```

---

## Docker Configuration (`[backends.docker]`)

For `type = "local"` backends only. Enables container lifecycle management via `apxm backend start/stop/status/logs/restart`.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `image` | string | **(required)** | Docker image |
| `model_path` | string | -- | Host path to model weights (bind-mounted) |
| `tensor_parallel` | integer | `1` | GPU count for tensor parallelism |
| `args` | string[] | `[]` | Extra `docker run` arguments |
| `command` | string[] | -- | Override container entrypoint |
| `env` | table | `{}` | Container environment variables |

### Example

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
args = ["--device", "/dev/kfd", "--device", "/dev/dri", "--ipc", "host"]

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

For a complete vLLM deployment walkthrough, see the [vLLM Integration](../integrations/vllm.md).

---

## Chat Configuration (`[chat]`)

Controls runtime behavior and model routing.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `providers` | string[] | `[]` (all) | Backend whitelist; empty means all registered backends |
| `default_backend` | string | -- | Fallback backend name |
| `default_model` | string | -- | Fallback model ID |
| `planning_model` | string | -- | Model used for PLAN operations |
| `max_context_tokens` | integer | `8192` | Session context limit |
| `session_storage` | string | -- | Session persistence directory |
| `system_prompt` | string | -- | Default system prompt |

```toml
[chat]
providers = ["anthropic", "corp-gateway"]
default_backend = "anthropic"
default_model = "claude-sonnet-4-5"
planning_model = "claude-sonnet-4-5"
max_context_tokens = 8192
```

---

## Routing (`[chat.routing]`)

Fine-grained control over which backend and model handles each request. The router resolves requests through a fixed priority order (see [Resolution Order](#routing-resolution-order)).

### Operation Routes

Route specific AIS operations to specific backends or models.

| Field | Type | Description |
|-------|------|-------------|
| `backend` | string | Backend name to use for this operation |
| `model` | string | Model ID to use for this operation |

**Routable operations:** `plan`, `think`, `reason`, `reflect`, `ask`, `verify`

```toml
[chat.routing.operation_routes.plan]
backend = "anthropic"
model = "claude-sonnet-4-5"

[chat.routing.operation_routes.think]
backend = "local-gpu"

[chat.routing.operation_routes.ask]
backend = "corp-gateway"
```

### Model Aliases

Shortcuts for commonly-used models. Use the alias name anywhere a model ID is accepted.

```toml
[chat.routing.model_aliases.fast]
model = "claude-haiku-4-5"
backend = "anthropic"

[chat.routing.model_aliases.smart]
model = "claude-sonnet-4-5"

[chat.routing.model_aliases.local]
model = "/shared_inference/models/Google/Gemma-3-27b-it"
backend = "local-gpu"
```

### Fallback Chains

Define what happens when a backend becomes unhealthy.

```toml
[[chat.routing.fallback_chains]]
backend = "anthropic"
fallbacks = ["corp-gateway", "local-gpu"]

[[chat.routing.fallback_chains]]
backend = "corp-gateway"
fallbacks = ["local-gpu"]
```

### Routing Resolution Order

The router resolves each request through this fixed priority:

1. **Explicit selection** -- request specifies backend/model directly
2. **Model alias** -- `[chat.routing.model_aliases]` resolution
3. **Operation route** -- `[chat.routing.operation_routes]` for the AIS op type
4. **Global default** -- `chat.default_backend` / `chat.default_model`
5. **Strategy** -- `FirstHealthy` / `RoundRobin` / `LowLatency`
6. **Fallback chain** -- if the resolved backend is unhealthy

---

## Instruction Prompts (`[instruction]`)

System prompts injected for each AIS operation type. These are used when no explicit `system_prompt` attribute is set on a graph node.

| Field | Type | Description |
|-------|------|-------------|
| `ask` | string | System prompt for ASK operations |
| `think` | string | System prompt for THINK operations |
| `reason` | string | System prompt for REASON operations |
| `plan` | string | System prompt for PLAN operations |
| `reflect` | string | System prompt for REFLECT operations |

```toml
[instruction]
ask = "You are a helpful AI assistant. Be concise."
think = "Think step by step. Show your reasoning process."
reason = "Provide structured reasoning with clear logical steps."
plan = "Create actionable plans with concrete milestones."
reflect = "Analyze execution patterns and suggest improvements."
```

---

## Tool Configuration (`[tools.*]`)

Control tool behavior and permissions for agent sandboxes.

### Bash Tool (`[tools.bash]`)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the bash tool |
| `timeout_secs` | integer | `120` | Command execution timeout |
| `max_output_bytes` | integer | `100000` | Maximum stdout+stderr capture |
| `working_directory` | string | -- | Override working directory |
| `blocked_commands` | string[] | `[]` | Deny-list of command prefixes |
| `allowed_commands` | string[] | -- | Allow-list (if set, only these are permitted) |

### Read Tool (`[tools.read]`)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the read tool |
| `max_file_size` | integer | `1048576` | Maximum file size in bytes |
| `max_default_lines` | integer | `200` | Default line limit per read |
| `working_directory` | string | -- | Override working directory |
| `allowed_extensions` | string[] | -- | Restrict to these file extensions |
| `blocked_paths` | string[] | `[]` | Deny-list of path patterns |
| `allowed_paths` | string[] | -- | Allow-list (if set, only these paths are readable) |

### Write Tool (`[tools.write]`)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the write tool |
| `max_file_size` | integer | `1048576` | Maximum file size in bytes |
| `working_directory` | string | -- | Override working directory |
| `create_directories` | bool | `true` | Auto-create parent directories |
| `overwrite_existing` | bool | `true` | Allow overwriting existing files |
| `blocked_extensions` | string[] | `[]` | Deny-list of file extensions |
| `allowed_paths` | string[] | -- | Allow-list (if set, only these paths are writable) |

### Search Web Tool (`[tools.search_web]`)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable web search |
| `max_results` | integer | `10` | Maximum search results |
| `safe_search` | bool | `true` | Enable safe search filtering |
| `search_depth` | string | `"basic"` | `basic` or `advanced` |
| `include_answer` | bool | `false` | Include generated answer |
| `blocked_domains` | string[] | `[]` | Deny-list of domains |
| `blocked_queries` | string[] | `[]` | Deny-list of query patterns |
| `allowed_domains` | string[] | -- | Allow-list (if set, only these domains are queried) |

### Presets

Preset names (like `bash_safe`, `read_source`) are organizational conventions -- they are just TOML section names with no built-in behavior. Define the actual field values for each preset yourself.

```toml
[tools.bash_safe]
enabled = true
timeout_secs = 120
blocked_commands = ["rm -rf", "sudo", "su "]

[tools.bash_build]
enabled = true
timeout_secs = 600

[tools.read_source]
enabled = true
max_default_lines = 200
allowed_extensions = ["rs", "py", "js", "toml", "md", "txt"]
blocked_paths = [".env", "credentials", ".git/config"]

[tools.write_safe]
enabled = true
blocked_extensions = ["exe", "sh", "bat", "ps1", "dll", "so"]
```

---

## Environment Variable References

Any `api_key`, `endpoint`, or header value supports `env:VAR_NAME` syntax. The referenced environment variable is read at startup; if it is unset, APXM exits with an error (fail-fast).

```toml
api_key = "env:ANTHROPIC_API_KEY"     # reads $ANTHROPIC_API_KEY
endpoint = "env:CUSTOM_ENDPOINT"       # reads $CUSTOM_ENDPOINT
```

---

## CLI Quick Reference

### Backend Management

```bash
apxm backend list                  # list all backends
apxm backend add <name>            # add (flags: --type, --protocol, --endpoint, --api-key)
apxm backend remove <name>         # remove
apxm backend test [name]           # test connectivity
apxm backend migrate               # import from legacy credentials.toml
```

### Local Backend Lifecycle

```bash
apxm backend start <name>          # start Docker container
apxm backend stop <name>           # stop container
apxm backend status [name]         # check container status
apxm backend logs <name>           # tail logs (--tail N)
apxm backend restart <name>        # restart container
```

### Model Inspection

```bash
apxm models list                   # list all models across backends
apxm models health                 # ModelRouter health status
```

---

## Migration

If you have a legacy `~/.apxm/credentials.toml`:

```bash
apxm backend migrate
```

This reads old credential entries and creates corresponding `[[backends]]` entries in `config.toml`. After migration, the runtime no longer falls back to `credentials.toml`.

---

## Complete Example

```toml
# ~/.apxm/config.toml

# =====================================================================
# BACKENDS
# =====================================================================

# Cloud: Anthropic
[[backends]]
name = "anthropic"
type = "cloud"
protocol = "anthropic"
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
user = "env:USER"

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

# =====================================================================
# ROUTING
# =====================================================================

[chat]
providers = ["anthropic", "corp-gateway", "local-gpu"]
default_backend = "anthropic"
default_model = "claude-sonnet-4-5"
planning_model = "claude-sonnet-4-5"

[chat.routing.operation_routes.plan]
model = "claude-sonnet-4-5"

[chat.routing.operation_routes.think]
backend = "local-gpu"

[chat.routing.operation_routes.ask]
backend = "corp-gateway"

[chat.routing.model_aliases.fast]
model = "claude-haiku-4-5"
backend = "anthropic"

[chat.routing.model_aliases.smart]
model = "claude-sonnet-4-5"

[[chat.routing.fallback_chains]]
backend = "anthropic"
fallbacks = ["corp-gateway", "local-gpu"]

# =====================================================================
# INSTRUCTION PROMPTS
# =====================================================================

[instruction]
ask = "You are a helpful AI assistant. Be concise."
think = "Think step by step."
plan = "Create actionable plans with clear milestones."

# =====================================================================
# TOOLS
# =====================================================================

[tools.bash_safe]
enabled = true
timeout_secs = 120

[tools.read_source]
enabled = true
max_default_lines = 200

[tools.search_docs]
enabled = true
max_results = 10
```

---

## See Also

- [Backends Guide](../guides/backends.md) -- conceptual overview of the backend/model/endpoint hierarchy
- [vLLM Integration](../integrations/vllm.md) -- graph-aware scheduling with vLLM backends
- [Optimization Overview](../optimization/overview.md) -- compiler optimization levels (`-O0`, `-O2`)
- [Graph Format Reference](graph-format.md) -- the `.apxm` graph JSON contract
