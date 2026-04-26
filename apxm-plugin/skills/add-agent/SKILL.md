---
name: add-agent
description: Add a new builtin agent template to APXM
user-invocable: false
---

# Add Agent Template

How to add a new builtin agent template to APXM. Agent templates define how to spawn and communicate with external AI agents (e.g., Claude, Codex, Gemini).

## Procedure

### 1. Add to the registry

Edit `crates/orchestration/apxm-acp/src/registry.rs` and add an entry to the templates array:

```rust
("my-agent".to_string(), AgentProfile {
    command: "my-agent --acp".to_string(),
    default_mode: Some("code".to_string()),
    default_model: Some("my-model-v1".to_string()),
}),
```

### 2. Regenerate codegen

```bash
dekk apxm codegen frontend      # Updates agents.py
dekk apxm codegen typescript     # Updates generated.ts (ALL_AGENTS)
```

### 3. Verify

```bash
# CLI shows the new agent
dekk apxm agent templates

# Python binding tests still pass
dekk apxm test-python-frontend

# Register and test
dekk apxm agent add my-agent
dekk apxm agent test my-agent
```

## Key Files

| File | Role |
|------|------|
| `crates/orchestration/apxm-acp/src/registry.rs` | Agent template definitions |
| `crates/compiler/apxm-frontend/python/apxm/_generated/agents.py` | Generated Python agent refs |
| `crates/tools/apxm-gui/frontend/src/types/generated.ts` | Generated TypeScript ALL_AGENTS |

## Agent Protocol

Agents communicate over JSON-RPC 2.0 on stdin/stdout. The ACP (Agent Client Protocol) lifecycle:

1. `SPAWN_AGENT` → process starts with `--acp` flag
2. JSON-RPC handshake (capabilities exchange)
3. `ASK` / `COMMUNICATE` → JSON-RPC method calls
4. Process terminates when workflow completes
