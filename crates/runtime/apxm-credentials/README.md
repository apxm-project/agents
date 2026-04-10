# apxm-credentials

Backend persistence and Docker lifecycle management for APXM.

## Overview

`apxm-credentials` manages LLM backend configuration stored in `~/.apxm/config.toml` and provides Docker container lifecycle operations for local backends (Ollama, vLLM).

## Module Structure

| Module | Description |
|--------|-------------|
| `backend` | `BackendStore` for reading/writing backend entries in `config.toml` |
| `docker` | Docker container start/stop/status for local LLM backends |
| `mask` | API key masking for display (`sk-...abc`) |
| `validate` | Backend entry validation (required fields, endpoint format) |

## Key Exports

- `BackendStore` -- CRUD operations on `~/.apxm/config.toml` backend entries
- `BackendError` -- error type for config I/O and validation failures

## Configuration

Backends are stored in `~/.apxm/config.toml` with `0o600` file permissions. Writes are atomic (temp file + rename). API keys never appear in event payloads or session traces.

```toml
[[backends]]
name = "openai"
type = "cloud"
protocol = "openai"
api_key = "sk-..."
models = ["gpt-4o"]
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-core | Error types |
