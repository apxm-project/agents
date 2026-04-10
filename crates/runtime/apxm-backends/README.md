# apxm-backends

LLM providers, storage backends, and prompt templates.

## Overview

`apxm-backends` consolidates three backend systems used by the runtime: a unified LLM provider interface supporting 6 protocols, pluggable storage backends, and compile-time embedded prompt templates via MiniJinja.

## Module Structure

| Module | Description |
|--------|-------------|
| `llm/` | Unified LLM provider interface |
| `llm/backends/openai` | OpenAI-compatible API backend |
| `llm/backends/anthropic` | Anthropic Messages API backend |
| `llm/backends/google` | Google Gemini API backend |
| `llm/backends/ollama` | Ollama local inference backend |
| `llm/backends/vllm` | vLLM backend with graph-aware prefix hints |
| `llm/backends/mock` | Deterministic mock backend for testing and benchmarks |
| `llm/registry/` | `LLMRegistry` with health monitoring and model resolution |
| `llm/provider` | `Provider` enum and `ProviderId` factory |
| `llm/assembler` | Request assembly and message formatting |
| `llm/rate_limit` | Per-provider rate limiting |
| `llm/retry/` | Exponential backoff with jitter and error classification |
| `llm/schema/` | JSON schema validation and output parsing |
| `llm/observability/` | `MetricsTracker`, `RequestTracer`, aggregated metrics |
| `storage/` | Pluggable storage backends (SQLite, in-memory, KV, embedder) |
| `prompts/` | MiniJinja template rendering from embedded templates |

## Provider Protocols (6)

| Protocol | Description |
|----------|-------------|
| OpenAI | OpenAI Chat Completions API (and compatibles) |
| Anthropic | Anthropic Messages API |
| Google | Google Gemini API |
| Ollama | Ollama local inference |
| Vllm | vLLM with graph-aware prefix caching hints |
| Mock | Deterministic responses for testing |

## Key Exports

- `LLMRegistry` -- provider registry with health monitoring
- `Provider` / `ProviderId` -- provider instances and identifiers
- `LLMRequest` / `LLMResponse` -- request/response types
- `RequestBuilder` -- fluent request construction
- `GenerationConfig` -- temperature, max_tokens, top_p, etc.
- `StreamChunk` -- streaming response chunks
- `MetricsTracker` / `RequestTracer` -- observability
- `BackendFactory` -- creates backends from `ProviderProtocol`
- `StorageBackend` -- trait for pluggable storage
- `render_prompt` / `render_inline` -- template rendering

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-core | Error types, Value type, provider specs |
