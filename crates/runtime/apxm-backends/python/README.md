# APXM Python vLLM Prototype

This directory contains an older Python prototype for APXM graph-aware vLLM
behavior. It is useful for local experimentation and tests, but it is not the
canonical runtime contract for the current external fork.

## Status

Treat this package as historical or prototype code.

The current graph-aware vLLM integration used by APXM is centered on the fork in:

- `external/vllm`

The canonical repo-level documentation for that integration is:

- [`docs/external-vllm-fork.md`](../../../../docs/external-vllm-fork.md)

## Why This README Changed

Older APXM docs and helpers described a four-endpoint design around:

- `POST /v1/apxm/graphs/register`
- `GET /v1/apxm/graphs/{graph_id}/priority/{node_id}`
- `DELETE /v1/apxm/graphs/{graph_id}`
- `GET /v1/apxm/metrics`
- plus separate pin-management endpoints

That is not the contract exposed by the live fork in `external/vllm`.

The current fork exposes:

- `POST /v1/apxm/graphs/register`
- `GET /v1/apxm/graphs/{graph_id}`
- `DELETE /v1/apxm/graphs/{graph_id}`

and consumes per-request APXM hints through request bodies sent to the forked
vLLM server.

## If You Are Trying To Run The Real Fork

Do not start from this package.

Start from:

1. `external/vllm/AGENTS.md`
2. [`docs/external-vllm-fork.md`](../../../../docs/external-vllm-fork.md)
3. the fork source under `external/vllm/vllm/...`

## If You Are Working On This Prototype

Be explicit in code reviews and docs that changes here affect the prototype
package only unless they are also reflected in the external fork and the Rust
integration.
