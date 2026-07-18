# vLLM Fork

This note documents the source-level contract between APXM and the APXM
graph-aware vLLM fork. In the APXM coordinator workspace, the source lives at
`workspace/vllm`; standalone APXM checkouts can set `APXM_VLLM_DIR`.
Claim-bearing evaluation may also run an equivalent container image built from that fork. Operator setup lives in
[`backends/vllm.md`](backends/vllm.md); do not duplicate bring-up steps here.

## Scope

APXM is backend-agnostic. vLLM is an optional LLM backend registered through
the normal backend registry. The fork is the `apxm-project/vllm` repo and adds
graph-aware OpenAI-compatible endpoints that stock vLLM does not expose.

Bring-up, container build, zoo reconciliation, and operator commands all live
in [`backends/vllm.md`](backends/vllm.md) and [`backends/model-zoo.md`](backends/model-zoo.md).
This page covers only the contract that any equivalent build must satisfy.

For containerized operation, the requirement is contract equivalence: the image
must expose the same HTTP routes and request-hint behavior as the source fork,
and APXM should register it through the Dekk service wrapper or, for an
already-owned allocation, `dekk agents vllm probe --endpoint ...` and
`dekk agents vllm enable --endpoint ...`.

## Fork Source

- Workspace path: `workspace/vllm`
- Override: `APXM_VLLM_DIR`
- Expected fork branch: `main`; commit and router verification are the source of
  truth for evaluation evidence.
- Operator runbook: `docs/backends/vllm.md`
- Rust backend: `crates/runtime/backends/src/llm/backends/vllm/backend.rs`

If APXM needs graph-aware vLLM behavior, it must talk to a server launched from
this fork or an equivalent build that exposes the same contract. A stock vLLM
server is not valid for the APXM graph-aware `vllm` backend because that backend
requires the `/v1/apxm/*` graph endpoints. Use another OpenAI-compatible backend
route if graph-aware behavior is intentionally not required.

## HTTP Contract

The fork exposes these APXM graph endpoints:

- `POST /v1/apxm/graphs/register`
- `GET /v1/apxm/graphs/{graph_id}`
- `DELETE /v1/apxm/graphs/{graph_id}`

The fork should also expose the scheduler-capability endpoint:

- `GET /v1/apxm/scheduler`

The scheduler endpoint returns the live vLLM scheduling policy. APXM treats the
graph endpoints and scheduler endpoint as required for the graph-aware `vllm`
backend. A response with `policy != "priority"` means APXM critical-path
priority hints can round-trip without reordering admission, so `dekk agents vllm
probe` and `enable` reject it.

Node inference still uses the OpenAI-compatible chat route:

- `POST /v1/chat/completions`

APXM scheduling hints travel through the OpenAI request body under
`extra_body.vllm_xargs.apxm`. The Rust backend should keep graph registration,
per-node hints, status capture, and release aligned to that request shape.
Reasoning parsers and chat-template kwargs are vLLM serve-time choices. APXM
operator tooling may pass them through, but APXM core/runtime must not infer
them from a model id.

## Naming Rules

Keep backend identity separate from model identity:

- `vllm` is the protocol and serving implementation.
- `<MODEL_REF>` is what the vLLM server loads.
- `<SERVED_MODEL_ID>` is what `/v1/models` reports and what APXM stores for
  routing.

There is no APXM/vLLM default model. Public docs should use `<MODEL_REF>` and
`<SERVED_MODEL_ID>` placeholders unless they are explicitly showing a local
operator example. Local model directories, Hugging Face ids, and other
vLLM-supported sources should keep their actual served ids visible in APXM
config and examples.

## Metrics Boundary

APXM metrics have two complementary layers:

- `runtime.token_accounting` records APXM-visible LLM token usage by graph,
  node, flow, and agent when providers report usage.
- `backends.graphs[]` records backend graph snapshots such as `backend_kind`,
  `backend_name`, `graph_id`, `pinned_handles`, `pinned_blocks`,
  `critical_path_length`, and `node_count`.

External coding agents use exact admitted profiles: Claude through the
`claude-agent-acp` adapter over the Claude Agent SDK, Codex through the admitted
ACP adapter over the Codex App Server, and other peers only for their proven
matrix. Their private work is one outer Capability NodeExecution with nested
attributed evidence, not APXM child executions. Peer-reported model/token/cost
data remains third-party evidence and never becomes authoritative
`runtime.token_accounting`. Do not infer tokens or cost from wall-clock time.

## Hard Stop Conditions

Do not treat a backend as graph-aware when any of these are true:

- `dekk agents vllm doctor` resolves imports outside the configured vLLM checkout.
- `dekk agents vllm probe` cannot reach the APXM graph endpoints.
- `dekk agents vllm enable <SERVED_MODEL_ID>` cannot find the served id in
  `/v1/models`.
- The workload routes to a different backend or model than the one enabled.

Do not claim priority-hint improvement when `/v1/apxm/scheduler` is missing or
does not report priority mode for the run.

At the APXM to vLLM boundary, use the term `graph`, not `workflow`.
