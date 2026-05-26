# vLLM backend (concept)

This document defines the **contract** between APXM and the vLLM fork at
`external/vllm/`. It is intentionally not a runbook — operational procedures
(start, scale, log, probe) live in
[`docs/backends/model-zoo.md`](model-zoo.md).

## What APXM gets from the fork

The fork (`https://github.com/apxm-project/vllm`, branch `apxm`) is a
thin patch stack on upstream vLLM that adds the **graph-aware HTTP
surface** APXM dispatches against.

The patch set adds:

- The OpenAI-compatible API server mounts an `/v1/apxm/*` router.
- Each chat/completion/responses/speech-to-text request accepts a
  `vllm_xargs.apxm` field carrying per-request scheduling hints (graph id,
  execution id, priority class, pin policy).
- The V1 scheduler honours those hints: critical-path requests are admitted
  with priority -1 (only under `--scheduling-policy priority`, which APXM
  enables by default), and graph-registered prefix blocks survive eviction
  via a per-graph pin map.

## The five `/v1/apxm/*` routes

| Route | Method | Purpose |
|---|---|---|
| `/v1/apxm/graphs/register` | POST | Register a graph's nodes, prefix cohorts, and pin policy |
| `/v1/apxm/graphs/{graph_id}` | GET | Read pin telemetry (registered, pinned_handles, pinned_blocks, critical_path_length) |
| `/v1/apxm/graphs/{graph_id}` | DELETE | Release the pin map and reclaim blocks |
| `/v1/apxm/scheduler` | GET | Advertise scheduler policy (single-variant under APXM) |
| `/v1/apxm/admin/reset_prefix_cache` | POST | Drop the prefix cache for benchmark cell isolation |

A backend registered under the `vllm-fork` protocol **must** expose all of
these. `GraphAwareVllmBackend::health_check` synchronously probes
`/v1/apxm/graphs/{__apxm_probe__}` and `/v1/apxm/scheduler`; missing either
is a hard registration failure. There is no per-call capability flag
gating graph-aware behaviour on a cached probe result — a registered
backend is guaranteed to support the full graph contract.

## The `vllm_xargs.apxm` payload

Every chat/completion request that APXM emits against the fork carries a
`vllm_xargs.apxm` object inside `extra_body`. The schema (see
`crates/core/apxm-core/src/constants/llm.rs`):

```json
{
  "schema_version": 1,
  "graph_id": "graph-...",
  "execution_id": "exec-...",
  "node_id": 42,
  "node_name": "synthesize",
  "downstream_nodes": [43, 44],
  "priority_class": "critical_path",
  "pin_policy": "per_request"
}
```

The fork's scheduler reads the priority class to decide admission ordering,
and the pin policy to decide whether to retain the prefix block past the
request's completion.

## What runs the fork

Operationally: **the zoo manifest is the only way.** Operators write a
`[[deployment]]` entry in `deploy/vllm/zoo.toml` and call `dekk apxm vllm
zoo-apply`. See [`docs/backends/model-zoo.md`](model-zoo.md) for the full
runbook, manifest schema, the three deployment shapes, the port allocator,
and failure modes.

Stock (vanilla) vLLM is **not** registered under the `vllm-fork` backend
type — that registration would fail loudly because the graph routes are
absent. Operators wanting to A/B against vanilla vLLM should register it
under the `OpenAIBackend` protocol instead.

## The contract is the boundary

Everything inside APXM treats the fork as a black box behind the five
routes plus the `vllm_xargs.apxm` extension. The fork keeps OpenAI
wire-compatibility, so APXM never patches request payloads at the wire
level — every dispatcher change either rides `vllm_xargs.apxm` or extends
one of the `/v1/apxm/*` routes.

This invariant is what makes the rebase tractable: as long as upstream
preserves the per-class registration points in
`vllm/v1/core/sched/scheduler.py` and `vllm/v1/engine/core_client.py`, the
APXM patch stack continues to cherry-pick cleanly. Pre-rebase baseline
lives at `.apxm/vllm-baseline.json`.
