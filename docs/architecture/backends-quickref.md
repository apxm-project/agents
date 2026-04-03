# APXM Backends Quick Reference

## The Hierarchy

```
BACKEND (where inference runs)
│
├── type: cloud | onprem | local
├── protocol: openai | anthropic | google | ollama | vllm
├── endpoint: https://api.anthropic.com
├── auth: api_key, headers
│
└── MODELS (LLMs deployed here)
    │
    ├── id: claude-sonnet-4-5
    ├── aliases: [sonnet, claude]
    ├── context_window: 200000
    ├── cost: $0.003/$0.015 per 1K tokens
    ├── capabilities: vision, functions
    └── tags: [production, smart]
```

## Backend Types at a Glance

| Type | Examples | Lifecycle | Cost | Graph Hints |
|------|----------|-----------|------|-------------|
| **cloud** | Anthropic, OpenAI, Google | Always on | Per-token | ❌ |
| **onprem** | Enterprise API, Azure OpenAI | Managed | Internal | ✅ if vLLM |
| **local** | vLLM, Ollama | You run it | Hardware only | ✅ |

## Quick Commands

```bash
# Backends
apxm backend list              # Show all backends
apxm backend add <name>        # Add backend
apxm backend test <name>       # Test connectivity
apxm backend health            # Show health status
apxm backend start <name>      # Start local backend
apxm backend stop <name>       # Stop local backend

# Models
apxm model list                # Show all models
apxm model list -b anthropic   # Models on specific backend
apxm model show <id>           # Model details
```

## Routing Flow

```
Request → Operation Route? → Model Alias? → Tag Match? → Default
              │                   │              │           │
              ▼                   ▼              ▼           ▼
         plan→sonnet        fast→haiku    prefer_tags   default_model
                                          [production]   claude-sonnet
```

## Config Locations

| File | Purpose |
|------|---------|
| `~/.apxm/config.toml` | Main config (backends, models, routing) |
| `~/.apxm/credentials.toml` | Legacy credentials (deprecated) |
| `.apxm/config.toml` | Project-level overrides |

## Minimal Config Example

```toml
[[backends]]
name = "anthropic"
type = "cloud"
protocol = "anthropic"
api_key = "env:ANTHROPIC_API_KEY"

[[backends.models]]
id = "claude-sonnet-4-5"
tags = ["default"]

[routing]
default_backend = "anthropic"
default_model = "claude-sonnet-4-5"
```
