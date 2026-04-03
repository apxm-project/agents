# LLM Backends

Practical guide to registering LLM backends and making them available to APXM workflows.

> **Full spec:** [config.md](../reference/config.md) |
> **Architecture:** [backends-and-models.md](../architecture/backends-and-models.md)

---

## Quick Start

```bash
# Register a cloud backend
apxm backend add my-openai --type cloud --protocol openai --api-key sk-...

# Verify connectivity
apxm backend test my-openai

# List what you have
apxm backend list
apxm models list
```

That is all you need. The backend is now available to every graph execution.

---

## Supported Protocols

| Protocol | Typical Use | Default Endpoint |
|----------|-------------|------------------|
| `openai` | GPT, OpenRouter, Azure OpenAI, any compatible API | `https://api.openai.com/v1` |
| `anthropic` | Claude models | `https://api.anthropic.com` |
| `google` | Gemini models | Google AI endpoints |
| `ollama` | Local Ollama server | `http://localhost:11434` |
| `vllm` | Self-hosted vLLM (+ APXM graph-hint extensions) | None (must specify) |

---

## Backend Types

- **`cloud`** -- SaaS providers; pay-per-token, always on.
- **`onprem`** -- Enterprise gateways; managed by IT, may need custom headers.
- **`local`** -- You own the GPUs; full Docker lifecycle via `apxm backend start/stop`.

---

## Config File

All backends live in `~/.apxm/config.toml` (the single source of truth). Minimal example:

```toml
[[backends]]
name = "my-openai"
type = "cloud"
protocol = "openai"
api_key = "env:OPENAI_API_KEY"
```

Use `"env:VAR_NAME"` to read secrets from the environment instead of storing them in plaintext.

See [config.md](../reference/config.md) for the full field reference.

---

## Models

Declare models under each backend with `[[backends.models]]`:

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
tags = ["production", "smart"]

[[backends.models]]
id = "claude-haiku-4-5"
aliases = ["haiku", "fast"]
tags = ["fast", "cheap"]
```

Aliases let you reference a model as `sonnet` instead of the full id.
Tags drive the routing system (see [config.md](../reference/config.md) for `[chat.routing]`).

---

## Docker Lifecycle (Local Backends)

Local backends with a `[backends.docker]` section support container management:

```bash
apxm backend start  local-gpu   # launch container
apxm backend status local-gpu   # check if running
apxm backend logs   local-gpu   # tail container logs
apxm backend stop   local-gpu   # shut down
apxm backend restart local-gpu  # restart
```

Example Docker config:

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
```

---

## Migration from `credentials.toml`

If you have backends registered under the legacy `apxm llm` system:

```bash
apxm backend migrate
```

This converts `~/.apxm/credentials.toml` entries into `[[backends]]` blocks in `config.toml`. After migration, `credentials.toml` is no longer consulted.

---

## CLI Reference

### `apxm backend`

| Command | Description |
|---------|-------------|
| `apxm backend list` | List all registered backends |
| `apxm backend add <name>` | Add a backend (supports `--type`, `--protocol`, `--api-key`, `--endpoint`, `--header`) |
| `apxm backend remove <name>` | Remove a backend |
| `apxm backend test [name]` | Test connectivity (all or one) |
| `apxm backend migrate` | Import legacy `credentials.toml` |
| `apxm backend start <name>` | Start Docker container (local only) |
| `apxm backend stop <name>` | Stop Docker container |
| `apxm backend status [name]` | Container status |
| `apxm backend logs <name>` | Tail container logs (`--tail N`) |
| `apxm backend restart <name>` | Restart container |

### `apxm models`

| Command | Description |
|---------|-------------|
| `apxm models list` | List all models across all backends |
| `apxm models health` | ModelRouter health status |

---

## See Also

- [config.md](../reference/config.md) -- complete config file reference
- [backends-and-models.md](../architecture/backends-and-models.md) -- architecture and routing details
