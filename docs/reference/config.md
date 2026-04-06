# APXM Configuration Reference

Complete reference for `~/.apxm/config.toml` — the single source of truth for APXM configuration.

---

## File Locations

| Path | Purpose |
|------|---------|
| `~/.apxm/config.toml` | User-level configuration (backends, routing, tools) |
| `.apxm/config.toml` | Project-level overrides (merged with user config) |

**Load order:** Project config takes precedence over user config.

---

## Backends (`[[backends]]`)

Backends define WHERE inference runs. This is the core abstraction.

### Required Fields

```toml
[[backends]]
name = "anthropic"          # Unique identifier
type = "cloud"              # cloud | onprem | local
protocol = "anthropic"      # Wire protocol (see below)
```

### Optional Fields

```toml
endpoint = "https://api.anthropic.com"    # API URL (has defaults for cloud)
api_key = "env:ANTHROPIC_API_KEY"         # Direct value or "env:VAR_NAME"

[backends.headers]                         # Custom HTTP headers
X-Custom-Gateway-Key = "env:KEY"
```

### Backend Types

| Type | Description | `api_key` required? | Docker lifecycle? |
|------|-------------|---------------------|-------------------|
| `cloud` | SaaS providers (Anthropic, OpenAI, Google) | Yes | No |
| `onprem` | Enterprise gateways (Enterprise API, Azure OpenAI) | Yes | No |
| `local` | Self-hosted (vLLM, Ollama) | Usually no | Yes |

### Protocols

| Protocol | For | Default endpoint |
|----------|-----|------------------|
| `anthropic` | Claude models | None (must specify or use `env:` var) |
| `openai` | GPT + OpenAI-compatible | None (must specify or use `env:` var) |
| `google` | Gemini models | None (must specify or use `env:` var) |
| `ollama` | Local Ollama | `http://localhost:11434` |
| `vllm` | vLLM (+ APXM graph hints) | `http://localhost:8000` |

### Example: Ollama (Local)

```toml
[[backends]]
name = "ollama"
type = "local"
protocol = "ollama"
# api_key not needed for local Ollama
endpoint = "http://localhost:11434"  # default, can omit

[[backends.models]]
id = "llama3.3"
aliases = ["llama", "local-fast"]
context_window = 128000
supports_functions = true   # llama3.1+ supports tool calling

[[backends.models]]
id = "qwen2.5:72b"
aliases = ["qwen", "local-large"]
context_window = 128000
supports_functions = true

# Optional: Ollama runtime options
[backends.options]
num_ctx = "128000"    # context window (overrides model default)
num_gpu = "1"         # GPU layers to offload
temperature = "0.8"
```

#### Routing to Ollama

```toml
[chat.routing.operation_routes.ask]
backend = "ollama"
model = "llama3.3"

[chat.routing.operation_routes.think]
backend = "ollama"
model = "qwen2.5:72b"
```

#### Auto-discover installed models

```bash
apxm backend list-models ollama
```

### Example: Cloud Backend

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

### Example: On-Prem Backend

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

### Example: Local Backend with Docker

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

---

## Docker Configuration (`[backends.docker]`)

For `type = "local"` backends, Docker config enables lifecycle management via `apxm backend start/stop/status/logs/restart`.

```toml
[backends.docker]
image = "gpu/vllm:latest"              # Docker image (required)
model_path = "/models/my-model"         # Host path to model weights
tensor_parallel = 4                     # GPU count for TP
args = ["--device", "/dev/kfd"]         # Extra docker run args
command = ["vllm", "serve", "..."]      # Override entrypoint (optional)

[backends.docker.env]                   # Container environment
HIP_VISIBLE_DEVICES = "0,1,2,3"
```

---

## Models (`[[backends.models]]`)

Models are nested under backends. A model describes WHAT you're calling.

### Fields

```toml
[[backends.models]]
id = "claude-sonnet-4-5"              # Sent to API (required)
aliases = ["sonnet", "fast"]          # Alternative routing names
context_window = 200000               # Max tokens (0 = unknown)
cost_per_1k_input = 0.003             # USD per 1K input tokens
cost_per_1k_output = 0.015            # USD per 1K output tokens
supports_vision = true                # Image input support
supports_functions = true             # Tool/function calling
supports_thinking = false             # Extended reasoning (o1, Claude)
tags = ["production", "smart"]        # For routing rules
```

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

---

## Chat Configuration (`[chat]`)

Controls runtime behavior and routing.

```toml
[chat]
providers = ["anthropic", "corp-gateway"]  # Backend whitelist (empty = all)
default_backend = "anthropic"            # Fallback backend
default_model = "claude-sonnet-4-5"      # Fallback model
planning_model = "claude-sonnet-4-5"     # Used for PLAN operations
max_context_tokens = 8192                # Session context limit
session_storage = "/path/to/sessions"    # Session persistence directory
system_prompt = "You are a helpful AI."  # Default system prompt
```

---

## Routing (`[chat.routing]`)

Fine-grained control over which backend/model handles each request.

### Operation Routes

Route specific AIS operations to specific backends or models:

```toml
[chat.routing.operation_routes.plan]
backend = "anthropic"
model = "claude-sonnet-4-5"

[chat.routing.operation_routes.think]
backend = "local-gpu"

[chat.routing.operation_routes.ask]
backend = "corp-gateway"

[chat.routing.operation_routes.reason]
model = "claude-sonnet-4-5"
```

**Routable operations:** `plan`, `think`, `reason`, `reflect`, `ask`, `verify`

### Model Aliases

Create shortcuts for commonly-used models:

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

Define what happens when a backend becomes unhealthy:

```toml
[[chat.routing.fallback_chains]]
backend = "anthropic"
fallbacks = ["corp-gateway", "local-gpu"]

[[chat.routing.fallback_chains]]
backend = "corp-gateway"
fallbacks = ["local-gpu"]
```

### Routing Resolution Order

1. Explicit backend selection (request specifies backend/model directly)
2. Model-based routing (model alias resolution via `[chat.routing.model_aliases]`)
3. Operation-specific default (`[chat.routing.operation_routes]`)
4. Global default backend (`chat.default_backend` / `chat.default_model`)
5. Strategy-based selection (`FirstHealthy` / `RoundRobin` / `LowLatency`)
6. Fallback chain (if primary is unhealthy)

---

## Instruction Prompts (`[instruction]`)

System prompts for different AIS operation types:

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

Control tool behavior and permissions.

### Bash Tool

```toml
[tools.bash]
enabled = true
timeout_secs = 120
max_output_bytes = 100000
working_directory = "/home/user/project"
blocked_commands = ["rm -rf", "sudo", "su "]
# allowed_commands = ["git", "cargo"]    # Whitelist mode (if set)
```

### Read Tool

```toml
[tools.read]
enabled = true
max_file_size = 1048576                   # 1MB
max_default_lines = 200
working_directory = "/home/user/project"
allowed_extensions = ["rs", "py", "js", "toml", "md", "txt"]
blocked_paths = [".env", "credentials", ".git/config"]
# allowed_paths = ["/repo"]              # Whitelist mode
```

### Write Tool

```toml
[tools.write]
enabled = true
max_file_size = 1048576
working_directory = "/home/user/project"
create_directories = true
overwrite_existing = true
blocked_extensions = ["exe", "sh", "bat", "ps1", "dll", "so"]
# allowed_paths = ["/repo/src"]          # Whitelist mode
```

### Search Web Tool

```toml
[tools.search_web]
enabled = true
max_results = 10
safe_search = true
search_depth = "basic"                    # basic | advanced
include_answer = false
blocked_domains = ["malware.com"]
blocked_queries = ["illegal"]
# allowed_domains = ["docs.rs", "github.com"]  # Whitelist mode
```

### Presets

> **Note:** Preset names (like `bash_safe`, `read_source`, etc.) are just config section names with no built-in behavior. They serve as organizational conventions — the user must fill in the actual configuration values (timeouts, blocked commands, allowed paths, etc.) for each preset.

Use preset names for common security profiles:

```toml
[tools.bash_safe]       # Blocks dangerous commands
enabled = true

[tools.bash_build]      # Allows build commands, 10-min timeout
enabled = true

[tools.bash_git]        # Git commands only (whitelist)
enabled = true

[tools.read_source]     # Code files only, blocks secrets
enabled = true

[tools.write_safe]      # Blocks executables
enabled = true

[tools.search_docs]     # Documentation sites only
enabled = true

[tools.search_research] # Deep search enabled
enabled = true
```

---

## CLI Reference

### Backend Management

```bash
apxm backend list                  # List all backends
apxm backend add <name>            # Add backend (flags: --type, --protocol, --endpoint, --api-key)
apxm backend remove <name>         # Remove backend
apxm backend test [name]           # Test connectivity
apxm backend migrate               # Import from legacy credentials.toml
```

### Local Backend Lifecycle

```bash
apxm backend start <name>          # Start Docker container
apxm backend stop <name>           # Stop container
apxm backend status [name]         # Check container status
apxm backend logs <name>           # Tail logs (--tail N)
apxm backend restart <name>        # Restart container
```

### Model Inspection

```bash
apxm models list                   # List all models
apxm models health                 # ModelRouter health status
```

---

## Environment Variable References

Any `api_key`, `endpoint`, or header value can reference an environment variable:

```toml
api_key = "env:ANTHROPIC_API_KEY"     # Reads $ANTHROPIC_API_KEY at startup
endpoint = "env:CUSTOM_ENDPOINT"       # Reads $CUSTOM_ENDPOINT
```

If the referenced variable is not set, APXM returns an error at startup (fail-fast).

---

## Complete Example

```toml
# ~/.apxm/config.toml

# ════════════════════════════════════════════════════════════════════
# BACKENDS
# ════════════════════════════════════════════════════════════════════

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

# ════════════════════════════════════════════════════════════════════
# ROUTING
# ════════════════════════════════════════════════════════════════════

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

# ════════════════════════════════════════════════════════════════════
# INSTRUCTION PROMPTS
# ════════════════════════════════════════════════════════════════════

[instruction]
ask = "You are a helpful AI assistant. Be concise."
think = "Think step by step."
plan = "Create actionable plans with clear milestones."

# ════════════════════════════════════════════════════════════════════
# TOOLS
# ════════════════════════════════════════════════════════════════════

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

## Migration

If you have a legacy `~/.apxm/credentials.toml`:

```bash
apxm backend migrate
```

This reads your old credentials and creates corresponding `[[backends]]` entries in `config.toml`.

> **Note:** After migration, the runtime no longer falls back to `credentials.toml`. The new config is the only source of truth.
