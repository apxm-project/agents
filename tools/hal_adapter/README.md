# HAL adapter — Hosted-API-Like shim for external agent benchmarks

Status: **scaffold only (2026-05-18)**.

## Purpose

External agentic benchmark harnesses (τ²-bench, GAIA, AppWorld,
SWE-bench Verified Lite) expect an OpenAI-chat-completions endpoint.
This shim fronts APXM behind that endpoint so the runners can use APXM
as their model backend without modification, while preserving the
per-arm switch needed for paired A/B (`apxm-on` vs `flat-http` on the
same vLLM-fork build).

## Surface (target)

```
tools/hal_adapter/server.py --port 18080 \
  --backend apxm-on \
  --model gpt-oss-120b \
  [--graph-context <id>]
```

Listens on `127.0.0.1:18080`, accepts OpenAI `POST /v1/chat/completions`
requests, forwards to the registered APXM backend (`vllm-gptoss` /
similar zoo service), returns the response verbatim.

- `--backend apxm-on`: requests go through APXM's graph-aware dispatch
  with `vllm_xargs.apxm` hints injected (default).
- `--backend flat-http`: requests skip hint injection (sets
  `APXM_DISABLE_HINTS=1` in the per-request env). Equivalent to the
  `--no-apxm-hints` arm of `concurrent_matrix.py`.
- `--model <bare-name>`: bare served-model name (e.g. `gpt-oss-120b`,
  not `openai/gpt-oss-120b`); the bare-name discipline is documented
  in the memory `apxm-config-model-id-must-match-served`.
- `--graph-context <id>` (optional): groups all requests with this
  context-id into a single APXM graph, so the benchmark's per-task
  multi-call sequence becomes one graph for pin / cohort scheduling.
  When absent, every request is a single-node graph.

## Implementation gates

- [ ] FastAPI / aiohttp server with `/v1/chat/completions` POST handler.
- [ ] Forwarding path that uses `apxm-runtime` via PyO3 or via a
      subprocess call to `dekk apxm execute` with a synthesized
      single-node graph per request.
- [ ] Graph-context grouping that maps the optional `--graph-context`
      argument to a stable graph_id across a benchmark task's multi-call
      sequence.
- [ ] Streaming support (SSE) — required by τ²-bench and AppWorld
      runners.
- [ ] Integration test: invoke τ²-bench's tool-call loop through the
      shim against the live `vllm-gptoss` zoo service, expect a
      non-zero pass rate on at least the trivial tool-use task.
- [ ] Manifest line per launch: `arm`, `model`, `graph_context_count`,
      `apxm_sha`, `vllm_fork_sha`, `port`. Written to a configurable
      log so EVAL claims can cite the exact shim configuration.

## Non-goals

- A persistent multi-process supervisor. The shim is one process per
  launch; benchmark drivers handle their own concurrency.
- LLM-judge fallback. If a runner cannot operate without LLM-judge
  grading, it is out of scope for the task-quality claim.
- A new agent framework. The shim is a pass-through; the agent loop
  lives in the external runner.

## Manifest + claim contract

Every shim launch writes a manifest line that any downstream claim
file MUST cite. The manifest schema mirrors the bench-harness
`EvidenceManifest` v2, with the additional
`hal_adapter_sha` field. Without a committed manifest line, a claim
that cites this shim is non-defensible.

## Why the scaffold ships before the implementation

This work is referenced from MASTER but had no implementation
directory until 2026-05-18. The scaffold + this README give the next
agent (human or otherwise) a concrete attach point: the surface is
named, the gates are listed, the manifest contract is fixed. The
implementation is intentionally NOT included in this scaffold —
shipping a stub server would tempt premature claims against an
unproven shim. Implementation work begins when an operator explicitly
takes this work as their goal.
