# APXM Backends — Quick Reference

> Full reference: [`docs/reference/config.md`](config.md)

## The Hierarchy

```
~/.apxm/config.toml  ← single source of truth

[[backends]]         ← WHERE inference runs
  type               : cloud | onprem | local
  protocol           : openai | anthropic | google | ollama | vllm
  endpoint           : URL
  api_key            : value or "env:VAR"
  [backends.docker]  : lifecycle config (local only)

  [[backends.models]]  ← WHAT you're calling
    id               : model identifier sent to API
    aliases          : routing shortcuts
    context_window   : max tokens
    tags             : routing labels
```

## Backend Types at a Glance

| Type | Examples | Lifecycle | Cost | Graph Hints |
|------|----------|-----------|------|-------------|
| `cloud` | Anthropic, OpenAI, Google | Always on | Per-token | ❌ |
| `onprem` | Enterprise API, Azure OpenAI | IT-managed | Internal | ✅ only if `protocol = "vllm"` |
| `local` | vLLM on GPU, Ollama | **You run it** | Hardware only | ✅ only if `protocol = "vllm"` |

## Protocols

| Protocol | Use for |
|----------|---------|
| `anthropic` | Claude models |
| `openai` | GPT models + anything OpenAI-compatible |
| `google` | Gemini models |
| `ollama` | Local Ollama server |
| `vllm` | Local/on-prem vLLM (+ APXM graph hints) |

## Minimal Config

```toml
[[backends]]
name = "anthropic"
type = "cloud"
protocol = "anthropic"
api_key = "env:ANTHROPIC_API_KEY"

[[backends.models]]
id = "claude-sonnet-4-5"
tags = ["production"]

[chat]
default_backend = "anthropic"
default_model = "claude-sonnet-4-5"
```

## Routing

```toml
[chat.routing.operation_routes.think]
backend = "local-gpu"        # Route THINK ops to local GPU

[chat.routing.operation_routes.ask]
backend = "corp-gateway"           # Route ASK ops to on-prem

[chat.routing.model_aliases.fast]
model = "claude-haiku-4-5"       # "fast" resolves to haiku

[[chat.routing.fallback_chains]]
backend = "anthropic"
fallbacks = ["corp-gateway", "local-gpu"]
```

## Routing Strategy

When no explicit backend or operation route matches, APXM uses the configured `RoutingStrategy`:

| Strategy | Behavior |
|----------|----------|
| `FirstHealthy` | Pick the first healthy backend (default) |
| `RoundRobin` | Cycle through healthy backends |
| `LowLatency` | Pick the backend with lowest observed latency |

## CLI Commands

```bash
# Backend management
apxm backend list / add / remove / test / migrate

# Local backend lifecycle
apxm backend start / stop / status / logs / restart <name>

# Model inspection
apxm models list
apxm models health
```

## `env:` Prefix

Any `api_key`, `endpoint`, or header value can reference an env var:

```toml
api_key = "env:ANTHROPIC_API_KEY"   # reads $ANTHROPIC_API_KEY at runtime
```

## Routable AIS Operations

`plan` · `think` · `reason` · `reflect` · `ask` · `verify`
