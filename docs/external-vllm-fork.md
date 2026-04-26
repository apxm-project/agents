# External vLLM Fork

This note documents the source-level contract between APXM and the repo-local
graph-aware vLLM fork. Operator setup lives in
[`backends/vllm.md`](backends/vllm.md); do not duplicate bring-up steps here.

## Scope

APXM is backend-agnostic. vLLM is an optional LLM backend implemented under the
normal backend registry. The fork lives at `external/vllm` and adds graph-aware
OpenAI-compatible endpoints that stock vLLM does not expose.

Use the APXM controller for normal operations:

```bash
dekk apxm vllm install
dekk apxm vllm doctor
dekk apxm vllm start <MODEL_REF> --wait
dekk apxm vllm probe
dekk apxm vllm enable <SERVED_MODEL_ID>
```

Direct calls into the fork's private Python environment are troubleshooting
steps, not the supported workflow.

## Fork Source

- Submodule path: `external/vllm`
- Expected fork branch: `apxm`
- Operator runbook: `docs/backends/vllm.md`
- Rust backend: `crates/runtime/apxm-backends/src/llm/backends/vllm/backend.rs`

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

Spawned coding agents such as Claude Code or Codex are ACP subprocesses. APXM
can record their node lifecycle, latency, output, session files, and child
execution links. Token usage for those subprocesses belongs in
`runtime.token_accounting` only when the ACP adapter reports it back to APXM.
Do not infer tokens or cost from wall-clock time.

## Hard Stop Conditions

Do not treat a backend as graph-aware when any of these are true:

- `dekk apxm vllm doctor` resolves imports outside `external/vllm`.
- `dekk apxm vllm probe` cannot reach the APXM graph endpoints.
- `dekk apxm vllm enable <SERVED_MODEL_ID>` cannot find the served id in
  `/v1/models`.
- The workload routes to a different backend or model than the one enabled.

At the APXM to vLLM boundary, use the term `graph`, not `workflow`.
