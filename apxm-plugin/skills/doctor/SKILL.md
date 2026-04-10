---
name: doctor
description: Diagnose environment health and dependency status
user-invocable: true
---

# Doctor

Checks the health of your APXM environment — whether MLIR/LLVM is available, credentials are registered, required environment variables are set, and configuration files exist. This is the first thing to run when something isn't working.

## Commands

```bash
dekk apxm doctor                   # human-readable health report
dekk apxm doctor --json            # machine-readable JSON report
```

## What Gets Checked

- **MLIR/LLVM**: Whether the MLIR toolchain is found, its resolved prefix path, and version (21+ required for compilation)
- **Credentials**: Number of registered LLM credentials and which providers are configured
- **Environment variables**: `APXM_BACKEND`, `MLIR_DIR`, `LLVM_DIR` — whether they're set
- **Config file**: Whether `~/.apxm/config.toml` exists, and how many LLM backends are configured

## JSON Output

```json
{
  "mlir": {"available": true, "prefix": "/path/to/env", "version": "22.0.0"},
  "credentials": {"count": 2, "providers": ["openai", "ollama"]},
  "environment": {"APXM_BACKEND": null, "MLIR_DIR": "/path", "LLVM_DIR": "/path"},
  "config": {"found": true, "path": "/home/user/.apxm/config.toml", "backends": 1}
}
```

## When to Use

- After `dekk apxm install` to verify everything was set up correctly
- When `compile` or `execute` fails with mysterious errors
- When moving to a new machine or environment
- To check if credentials are registered before running a graph
