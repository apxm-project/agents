# APXM LLM Configuration Guide

All LLM configuration lives in `~/.apxm/config.toml`.

## Backend Architecture

APXM routes through the enterprise LLM gateway at `https://llm.example.com`.

```
~/.apxm/config.toml
       │
       ├── [[backends]]  → amd-gateway   (ALL models — Claude, GPT, Gemini, DeepSeek)
       ├── [[backends]]  → amd-openai    (GPT-5.x codex only, fallback)
       └── [[backends]]  → my-anthropic (DEAD — /Anthropic/messages returns 404)
```

**Rule: Use `amd-gateway` for everything.** It serves all models via OpenAI-compatible API.
The response format is normalized in `crates/apxm-backends/src/llm/backends/openai/backend.rs`.

## Available Models (tested 2026-04-06)

### Claude (via amd-gateway)
| Model ID | Alias | Use |
|---|---|---|
| `claude-sonnet-4-5@20250929` | `claude`, `smart`, `balanced` | General reasoning, THINK/PLAN/REFLECT |
| `claude-haiku-4-5@20251001` | `haiku`, `fast`, `cheap` | Quick ops, ASK |
| `claude-opus-4-5@20251101` | `opus`, `powerful`, `heavy` | Complex tasks |

### Gemini (via amd-gateway)
| Model ID | Alias | Use |
|---|---|---|
| `gemini-2.5-flash` | `gemini-fast` | Fast multimodal |
| `gemini-2.5-pro` | `gemini`, `gemini-pro` | Powerful multimodal, huge context |
| `gemini-3.1-pro-preview` | `gemini-frontier` | Cutting edge |

### OpenAI (via amd-gateway)
| Model ID | Alias | Use |
|---|---|---|
| `gpt-4.1` | `gpt41`, `gpt-balanced` | Balanced |
| `gpt-4.1-mini` | `gpt41-mini`, `gpt-fast` | Fast, cheap |
| `gpt-4.1-nano` | `gpt41-nano`, `gpt-nano` | Nano |
| `gpt-5` | `gpt5`, `gpt-frontier` | Frontier |
| `o4-mini` | `reasoner`, `reasoning-fast` | Reasoning |
| `o3` | `deep-reasoner`, `reasoning-heavy` | Heavy reasoning |
| `o3-mini` | `reasoning-balanced` | Balanced reasoning |

### Other (via amd-gateway)
| Model ID | Alias | Use |
|---|---|---|
| `DeepSeek-R1` | `deepseek`, `verifier` | Verification, reasoning |
| `meta-llama/Llama-4-Maverick-17B-128E-Instruct-FP8` | `llama`, `llama4` | On-prem multimodal |

## Per-Operation Routing

APXM routes different operations to different models:

```toml
[chat.routing.operation_routes.ask]    # Quick Q&A
backend = "amd-gateway"
model = "claude-haiku-4-5@20251001"   # Fast + cheap

[chat.routing.operation_routes.think]  # Deep reasoning
backend = "amd-gateway"
model = "claude-sonnet-4-5@20250929"  # Balanced

[chat.routing.operation_routes.verify] # True/false claims
backend = "amd-gateway"
model = "o4-mini"                      # Reasoning model
```

## Override Model Per-Node in AIS

```ais
// Use default (Haiku for ASK)
ask("What is the capital of France?") -> answer

// Override with alias
ask("Analyze this complex architecture") -> analysis {
    model = "smart"  // → claude-sonnet
}

// Or use model ID directly
think("Deep analysis...") -> result {
    model = "gemini-2.5-pro"
}
```

## Model Aliases

Aliases let you reference models symbolically:

```toml
[chat.routing.model_aliases.smart]      # claude-sonnet
[chat.routing.model_aliases.fast]       # claude-haiku
[chat.routing.model_aliases.powerful]   # claude-opus
[chat.routing.model_aliases.reasoner]   # o4-mini
[chat.routing.model_aliases.deep-reasoner] # o3
[chat.routing.model_aliases.verifier]   # DeepSeek-R1
[chat.routing.model_aliases.gemini]     # gemini-2.5-pro
[chat.routing.model_aliases.codex]      # gpt-5.4
```

## Fallback Chains

When a backend fails (circuit breaker trips), APXM falls back:

```
amd-gateway → amd-openai
amd-openai  → amd-gateway
```

Note: `my-anthropic` is NOT in any fallback chain. It returns 404 for all requests.

## Custom System Prompts

Override the system prompt per operation:

```toml
[chat.instructions]
think = "You are a deep reasoning expert. Think step by step."
ask = "You are a concise, helpful assistant. Answer in 1-2 sentences."
plan = "You are an expert planner. Always produce structured JSON output."
```

## Adding New Models

```bash
# Add to amd-gateway backend in ~/.apxm/config.toml:
[[backends.models]]
id = "new-model-id@version"
aliases = ["my-alias"]
context_window = 200000
supports_functions = true
tags = ["category", "speed"]

# Add alias:
[chat.routing.model_aliases.my-alias]
model = "new-model-id@version"
backend = "amd-gateway"
```

## Testing Model Connectivity

```bash
# Test all backends
dekk apxm backend test amd-gateway
dekk apxm backend test amd-openai

# Test specific model directly
curl -H "X-Custom-Gateway-Key: YOUR_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"claude-sonnet-4-5@20250929","messages":[{"role":"user","content":"hi"}],"max_tokens":10}' \
  "https://llm.example.com/api/chat/completions"

# Run a simple workflow to test routing
dekk apxm execute examples/basics/hello.ais
```
