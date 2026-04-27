---
name: codegen
description: Regenerate downstream frontend projections from Rust contracts
user-invocable: false
---

# Codegen

How to regenerate downstream projections (Python, TypeScript) from Rust contracts. The codegen pipeline keeps frontend bindings aligned with AIS operations, shared constants, backend-owned provider/model catalogs, and ACP agent profiles.

## Commands

```bash
dekk apxm codegen frontend      # Python _generated/ → operations, constants, agents, providers, models, emission
dekk apxm codegen typescript     # TypeScript generated.ts → ops, attrs, providers, agents
```

## When to Run

Run codegen after modifying any of these source files:

| Source File | What Changed |
|-------------|--------------|
| `crates/core/apxm-ais/src/attrs.rs` | Attribute constants or ALL_ATTR_NAMES |
| `crates/core/apxm-ais/src/operations/definitions.rs` | Operation specs, fields, categories |
| `crates/runtime/apxm-backends/src/llm/catalog.rs` | Built-in provider or model metadata |
| `crates/runtime/apxm-backends/src/llm/protocol.rs` | ProviderProtocol variants or wire names |
| `crates/orchestration/apxm-acp/src/registry.rs` | Agent templates |
| `crates/tools/apxm-cli/src/frontend/registry.rs` | Codegen registry wrappers |

## Pipeline

```
Rust contracts (apxm-ais, apxm-core, apxm-backends, apxm-acp)
    ↓
registry.rs (wraps Rust contracts into Frontend* types)
    ↓
codegen.rs → Python _generated/
    ├── constants.py    (from ALL_ATTR_NAMES)
    ├── operations.py   (from OperationSpec — full projection with FieldSpec)
    ├── agents.py       (from AgentRegistry)
    ├── providers.py    (from backend-owned BUILTIN_PROVIDERS + ProviderProtocol)
    ├── models.py       (from backend-owned BUILTIN_MODELS)
    └── emission.py     (from MlirEmissionSpec)
    ↓
codegen_ts.rs → TypeScript generated.ts
    ├── OpSpec + FieldSpec types
    ├── ALL_OPERATIONS const
    ├── ATTR constants object
    ├── ProviderProtocol type + BUILTIN_PROVIDERS
    ├── ModelSpec + BUILTIN_MODELS
    └── AgentTemplate type + ALL_AGENTS
```

## Generated Output Files

| Target | Output Path |
|--------|-------------|
| Python | `crates/compiler/apxm-frontend/python/apxm/_generated/` |
| TypeScript | `crates/tools/apxm-gui/frontend/src/types/generated.ts` |

## Verification

```bash
# Python
python -c "from apxm._generated.operations import ALL_OPERATIONS; print(len(ALL_OPERATIONS))"
python -c "from apxm._generated.providers import BUILTIN_PROVIDERS; print(len(BUILTIN_PROVIDERS))"

# TypeScript (check it compiles)
cd crates/tools/apxm-gui/frontend && npx tsc --noEmit src/types/generated.ts
```
