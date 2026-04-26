---
name: backend/add
description: Register and manage inference backends for graph execution
user-invocable: true
---

# Backend Management

Manages inference backends that APXM uses when executing graphs. Every ASK, THINK, and REASON node needs a backend to call.

Backends are stored in `~/.apxm/config.toml` with 0600 permissions.

## Commands

### Add a backend

```bash
dekk apxm backend add my-openai --type cloud --protocol openai --api-key <API_KEY>
dekk apxm backend add local --type local --protocol ollama --endpoint http://localhost:11434
dekk apxm backend add corp --type onprem --protocol openai \
  --endpoint https://gateway.company.com/v1 \
  --api-key <API_KEY> --header "X-Custom-Gateway-Key=<VALUE>"
```

### Add a model to a backend

```bash
dekk apxm backend add-model my-openai <SERVED_MODEL_ID> --context-window <TOKENS> --supports-functions --alias fast --tag production
```

### List, test, remove

```bash
dekk apxm backend list
dekk apxm backend test
dekk apxm backend test my-openai
dekk apxm backend remove my-openai
```

### Docker lifecycle (local backends)

```bash
dekk apxm backend start local-vllm
dekk apxm backend stop local-vllm
dekk apxm backend status
dekk apxm backend logs local-vllm
dekk apxm backend restart local-vllm
```

## References

- [vLLM Backend](docs/backends/vllm.md)
- Run `dekk apxm backend --help` for the current command surface
