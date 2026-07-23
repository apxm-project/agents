# apxm-backends

- Status: pre-canonical provider/backend implementation inventory
- Canonical inference seam: [`apxm-inference`](../inference/)
- Canonical execution boundary:
  [ADR-0011](../../../docs/adr/0011-agent-program-execution-is-one-end-to-end-spine.md)

This crate contains the current OpenAI, Anthropic, Google, Ollama, vLLM, mock,
storage, prompt, registry, retry, and observability implementations. It is
useful migration and adapter evidence, but its unified registry/provider
routing model is not canonical Agent Program semantics.

Canonical v1 source contains a typed `Model` bound to one exact
`ModelTargetRef`. Server-owned admission materializes one exact
target/deployment/port binding. Runtime calls the injected
`ModelInferencePort`; it never searches this crate's registry, selects the
first healthy provider, substitutes a model, or falls back after failure.

Each production provider implementation must enter through its own exact
Implementation Descriptor and Port Contract. It must preserve request/effect
identity, typed Model Context, Tool schemas, streaming, cancellation, safe
retry/reconciliation, outcome-unknown, native usage, and evidence semantics.
Test mocks remain explicit dependencies and never become production fallbacks.

## APXM-vLLM

The current `llm/backends/vllm` module records graph-aware prefix and scheduling
experiments. The canonical first-party adapter is owned by the adapters plane
and implements `ModelInferencePort` for an exact APXM-vLLM binding. Agent
source/AIR remains provider-neutral; backend-specific hints are admitted
implementation metadata whose effect and telemetry must be observable.

See the [vLLM boundary](../../../docs/backends/vllm.md) and the
[source-first Agent frontend master plan](../../../docs/agents/simple-agent-authoring-frontend-plan.md).
