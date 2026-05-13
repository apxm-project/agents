---
name: apxm-vllm-service
description: Use when building, launching, probing, or running APXM workloads against the Dockerized APXM-vLLM backend, especially on Slurm compute nodes. Enforces Dekk as the authority CLI, persistent service allocations, image-store reuse, and service-exec for commands that need the vLLM endpoint.
---

# APXM-vLLM Service

Use the Dekk-controlled APXM-vLLM service path for graph-aware vLLM work.
Docker isolates the vLLM server process; APXM communicates with it over HTTP.
Slurm owns GPU allocation and accounting.

## Operating Rules

- `dekk apxm` is the authority CLI. Do not bypass it with raw `docker`, `srun`,
  or ad hoc Python vLLM commands unless debugging the controller itself.
- Build the APXM-vLLM image once, save it into `.apxm/vllm-images`, then launch
  or reuse a persistent service. Do not rebuild or reinstall vLLM inside every
  Slurm allocation.
- Prefer an existing service. Start a new service only when changing image,
  model, context length, GPU allocation, or server startup flags.
- Run claim-bearing APXM commands from the service allocation with
  `dekk apxm vllm service-exec <name> -- <command>`.
- Generated benchmark/evaluation artifacts belong under `.apxm`, not under
  `examples/`.

## Standard Flow

From the APXM repo root:

```bash
dekk apxm vllm doctor

APXM_COMMIT="$(git rev-parse --short HEAD)"
VLLM_COMMIT="$(git -C external/vllm rev-parse --short HEAD)"
IMAGE="apxm-vllm-gpu:${APXM_COMMIT}-${VLLM_COMMIT}"

dekk apxm vllm docker-build --image "$IMAGE"
dekk apxm vllm docker-save --image "$IMAGE"

dekk apxm vllm service-start gptoss120b openai/gpt-oss-120b \
  --image "$IMAGE" \
  --served-model-name gpt-oss-120b \
  --backend-name vllm-fork \
  --hf-home "$HOME/.cache/huggingface-apxm-vllm" \
  --max-model-len 32768

dekk apxm vllm service-status gptoss120b --probe
```

Run APXM workloads inside the service allocation:

```bash
dekk apxm vllm service-exec gptoss120b -- \
  python3 examples/python/benchmarks/benchmark_e2e.py \
    --iterations 3 \
    --precompile-artifacts \
    --emit-compiler-diagnostics
```

## Communication Contract

Inside the service allocation, APXM uses the registered endpoint:

```text
http://127.0.0.1:8916/v1
```

Required routes for graph-aware APXM-vLLM:

- `/v1/models`
- `/v1/chat/completions`
- `/v1/apxm/graphs/register`
- `/v1/apxm/graphs/{graph_id}`
- `/v1/apxm/scheduler`

Priority-latency claims require `/v1/apxm/scheduler` to report
`policy = "priority"`.

## Diagnostics

- Service status and capability probe:
  `dekk apxm vllm service-status <name> --probe`
- Container logs:
  `dekk apxm vllm service-exec <name> -- dekk apxm vllm docker-logs`
- Run a one-off health command in the allocation:
  `dekk apxm vllm service-exec <name> -- curl -s http://127.0.0.1:8916/v1/models`
- Stop only when intentionally releasing the service allocation:
  `dekk apxm vllm service-stop <name>`

## Anti-Patterns

- Do not use deprecated `install`, `download`, `start`, `serve`, `status`, or
  `logs` legacy commands.
- Do not submit a new Slurm job for every benchmark iteration.
- Do not make graph-aware claims from a stock vLLM server or from an APXM-vLLM
  server whose scheduler probe is missing.
- Do not put CSVs, sessions, compiler diagnostics, evidence manifests, or
  `.apxmobj` artifacts under `examples/`.
