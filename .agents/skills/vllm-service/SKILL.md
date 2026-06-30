---
name: vllm-service
group: Domain
description: Use when building, launching, probing, or running APXM workloads against the Dockerized APXM-vLLM backend, especially on Slurm compute nodes. Enforces Dekk as the authority CLI, persistent service allocations, image-store reuse, and service-exec for commands that need the vLLM endpoint.
user-invocable: true
---

# APXM-vLLM Service

Load `_shared/apxm-development-rules.md`,
`_shared/apxm-storage-layout-rules.md` before broad work.

Use the Dekk-controlled APXM-vLLM service path for graph-aware vLLM
work. Docker isolates the vLLM server process; APXM communicates with
it over HTTP. Slurm owns GPU allocation and accounting.

## Operating rules

- `dekk agents` is the authority CLI. No raw `docker`, `srun`, or ad-hoc
  Python vLLM commands except for debugging the controller itself.
- Build the APXM-vLLM image once, save it into `.apxm/vllm-images`,
  then launch or reuse a persistent service. Do not rebuild or
  reinstall vLLM inside every Slurm allocation.
- Prefer an existing service. Start a new service only when changing
  image, model, context length, GPU allocation, or server startup
  flags.
- Run APXM commands that need the service from the service allocation
  with `dekk agents vllm service-exec <name> -- <command>`.
- Generated artifacts belong under `.apxm/`, not `examples/`.
- HF cache + `.apxm/vllm-images/` must be visible from every Slurm
  compute node at the same path. See
  `_shared/apxm-storage-layout-rules.md`. Run `dekk agents vllm doctor`
  to print the resolved layout.
- Never `scancel` a Slurm job owned by `apxm`. Always allocate
  fresh.

## Standard flow (zoo path)

```bash
dekk agents vllm doctor
dekk agents vllm docker-load                       # from .apxm/vllm-images/
dekk agents vllm zoo-cache-warm                    # without GPU allocation
dekk agents vllm zoo-apply                         # provision per manifest
dekk agents vllm zoo-status                        # probe everything

dekk agents vllm service-list                      # what's registered
dekk agents vllm service-status <name> --probe     # one service
dekk agents vllm service-exec <name> -- <command>  # run inside allocation
```

To build a new image:

```bash
APXM_COMMIT="$(git rev-parse --short HEAD)"
VLLM_COMMIT="$(git -C external/vllm rev-parse --short HEAD)"
IMAGE="apxm-vllm-runtime:${APXM_COMMIT}-${VLLM_COMMIT}"

dekk agents vllm docker-build --image "$IMAGE" --base-image <UPSTREAM_TAG_OR_DIGEST>
dekk agents vllm docker-save  --image "$IMAGE"
```

## Communication contract

Inside the service allocation, APXM uses the registered endpoint
(port allocated by `_allocate_port()`, not literal `8916`). Required
routes for graph-aware APXM-vLLM:

- `/v1/models`
- `/v1/chat/completions`
- `/v1/apxm/graphs/register`
- `/v1/apxm/graphs/{graph_id}`
- `/v1/apxm/scheduler`

Priority-latency claims require `/v1/apxm/scheduler` to report
`policy = "priority"`.

## Diagnostics

- `dekk agents vllm service-status <name> --probe`
- `dekk agents vllm service-exec <name> -- dekk agents vllm docker-logs`
- `dekk agents vllm service-exec <name> -- curl -s http://127.0.0.1:<port>/v1/models`
- `dekk agents vllm service-stop <name>` — only when intentionally
  releasing the service allocation.

## Anti-patterns

- A new Slurm job per benchmark iteration.
- Graph-aware claims from a stock vLLM server, or from an APXM-vLLM
  server whose scheduler probe is missing.
- CSVs, sessions, compiler diagnostics, evidence manifests, or
  `.apxmobj` artifacts under `examples/`.
- Hardcoding port `8916` in code (lints as `hardcoded-port-8916`).
- Re-introducing BaseHTTPMiddleware in the vLLM fork — silently
  breaks chat completions. See `feedback_basehttpmiddleware_breaks_chat`.
