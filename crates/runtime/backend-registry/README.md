# apxm-backend-registry

Backend registration and API-key reference management for APXM.

## Overview

`apxm-backend-registry` manages LLM backend configuration stored in
`$APXM_HOME/config.toml` (default `~/.apxm/config.toml`). It records backend
protocols, endpoints, model metadata, and API-key or `env:VAR` references.

This crate is not the APXM credential custody plane. Provider OAuth tokens,
sealed connections, webhook secrets, and tenant/user key hierarchy belong to
`auth`.

Container lifecycle is not part of this crate. The graph-aware vLLM backend has
its own operator path under `deploy/vllm/` and `dekk agents vllm ...`.

## Module Structure

| Module | Description |
|--------|-------------|
| `backend` | `BackendStore` for reading/writing backend entries in `config.toml` |
| `mask` | API key masking for display |
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
api_key = "env:OPENAI_API_KEY"
models = ["<SERVED_MODEL_ID>"]
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-core | Error types |
