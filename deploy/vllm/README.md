# APXM vLLM Deployment

Status: operational deployment plan, 2026-05-12.

This directory is the Dekk-controlled deployment surface for APXM graph-aware
vLLM.

## Builder Policy

`dekk apxm vllm docker-build` uses Docker BuildKit through `docker buildx build
--load`. Docker's legacy builder is not an allowed APXM path.
`dekk apxm vllm doctor` must report `docker_buildx_ready=true` before image
builds are considered ready.

## Communication Model

APXM does not call Docker directly during graph execution. Docker only owns the
server process. The runtime talks to vLLM over HTTP:

```text
APXM runtime / dekk
  -> http://127.0.0.1:8916/v1/models
  -> http://127.0.0.1:8916/v1/chat/completions
  -> http://127.0.0.1:8916/v1/apxm/graphs/register
  -> http://127.0.0.1:8916/v1/apxm/graphs/{graph_id}
  -> http://127.0.0.1:8916/v1/apxm/scheduler

Docker container
  runs python -m vllm.entrypoints.openai.api_server
  binds 0.0.0.0:8916 with --network host
```

The APXM backend registry stores `http://127.0.0.1:8916/v1` as the endpoint.
The graph-aware `vllm` backend is valid only when `/v1/apxm/graphs/*` exists.
Priority-latency claims additionally require `/v1/apxm/scheduler` to report
`policy = "priority"`.

## Dekk Is The Authority

Operators should use a persistent service for cluster work. The direct Docker
commands are allocation-local primitives used by the Slurm wrapper, not the
normal benchmark loop.

```bash
dekk apxm vllm docker-build \
  --image apxm-vllm-runtime:<tag> \
  --base-image <VLLM_IMAGE_TAG_OR_DIGEST>

dekk apxm vllm docker-save \
  --image apxm-vllm-runtime:<tag>

dekk apxm vllm service-start gptoss120b openai/gpt-oss-120b \
  --image apxm-vllm-runtime:<tag-or-digest> \
  --served-model-name gpt-oss-120b \
  --backend-name vllm-fork \
  --hf-home "$HOME/.cache/huggingface-apxm-vllm" \
  --max-model-len 32768

dekk apxm vllm service-status gptoss120b --probe
dekk apxm vllm service-exec gptoss120b -- dekk apxm execute <GRAPH.py>
```

The service wrapper loads the saved image, starts Docker with the APXM-vLLM
flags, waits for `/v1/models`, runs the APXM graph-route probe, and registers
the endpoint/model in the APXM backend registry.

## Slurm

Slurm must own GPU allocation and accounting. The wrapper does not build
images. It loads the exact APXM image archive from `.apxm/vllm-images` into the
allocated node's Docker daemon, then starts the container.

Create the image-store artifact once before submitting jobs:

```bash
APXM_COMMIT="$(git rev-parse --short HEAD)"
VLLM_COMMIT="$(git -C external/vllm rev-parse --short HEAD)"
IMAGE="apxm-vllm-runtime:${APXM_COMMIT}-${VLLM_COMMIT}"

dekk apxm vllm docker-build --image "$IMAGE" --base-image <VLLM_IMAGE_TAG_OR_DIGEST>
dekk apxm vllm docker-save --image "$IMAGE"
```

Then launch:

```bash
dekk apxm vllm service-start gptoss120b openai/gpt-oss-120b \
  --image "$IMAGE" \
  --served-model-name gpt-oss-120b \
  --hf-home "$HOME/.cache/huggingface-apxm-vllm" \
  --max-model-len 32768
```

Run commands against that persistent allocation:

```bash
dekk apxm vllm service-status gptoss120b --probe
dekk apxm vllm service-exec gptoss120b -- \
  python3 examples/python/benchmarks/benchmark_e2e.py \
    --iterations 3 \
    --precompile-artifacts \
    --emit-compiler-diagnostics
```

`service-exec` uses `srun --jobid <service-job> --overlap`, so APXM runs on
the same node as the container and `http://127.0.0.1:8916/v1` remains the
correct backend endpoint. Submit a new service job only when changing the
image, model, context length, or other server startup contract.

Override `APXM_VLLM_IMAGE_ARCHIVE` only when the archive is intentionally stored
outside the default APXM image store. Missing archives are hard failures.

## Image Policy

APXM-vLLM images must use explicit tags or digests. Runs must record:

- image tag and immutable image id/digest;
- APXM commit and dirty-worktree status;
- `external/vllm` commit;
- Slurm job id, node list, partition, and GRES;
- model ref, served model id, and model snapshot path/hash when available;
- exact Dekk command and vLLM logs.
