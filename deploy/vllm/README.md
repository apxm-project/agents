# APXM vLLM deployment

This directory is the deployment surface for APXM graph-aware vLLM. **It is
not the operator runbook.** All operational procedures live in
[`docs/backends/model-zoo.md`](../../docs/backends/model-zoo.md).

## Layout

| File | Role |
|---|---|
| `zoo.example.toml` | Schema reference + 3-shape template. Operators copy to `zoo.toml` (gitignored) and edit. |
| `zoo.toml` | Operator's local manifest (gitignored). **Sole source of truth** for what runs once present. |
| `run-vllm.sh` | Slurm wrapper invoked by `_start_one_service` via `sbatch`. Single-node only; multi-node Ray (cross-node TP+PP) is out of scope — one service per node. |
| `Dockerfile.apxm` | Python-source-overlay Dockerfile that lays the `external/vllm/` fork on top of a pinned ROCm vLLM base image. |

## How a service starts

1. Operator copies `zoo.example.toml` to `zoo.toml` and edits.
2. `dekk apxm vllm zoo-cache-warm` pulls weights to the shared HF cache.
   CPU-only; refuses to start if WekaFS free < Σ(weights_gb) × 1.2.
3. `dekk apxm vllm zoo-apply` expands the manifest, calls
   `_start_one_service(...)` per replica, which submits a Slurm job that
   runs `run-vllm.sh`.
4. The wrapper loads the image (`docker-load`), starts the container, and
   blocks on `sleep infinity` until the Slurm job is cancelled.

Direct CLI invocations of `docker-*` are internal to the wrapper; operators
should not call them by hand.

## Builder policy

`dekk apxm vllm docker-build` uses Docker BuildKit through
`docker buildx build --load`. Docker's classic builder is not allowed.
`dekk apxm vllm doctor` must report `docker_buildx_ready=true` before
image builds are considered ready.

The Dockerfile takes a single required `--build-arg BASE_IMAGE=…` pinning
the ROCm vLLM base; image tags follow
`apxm-vllm-runtime:<APXM_SHA>-<VLLM_SHA>` by convention but are
operator-supplied — the controller does not synthesize a default tag, so
the tag can never silently drift with HEAD.

## Communication model

vLLM listens on a TCP port chosen by the zoo manifest (typically the
[8916, 8999] allocator range — but never assume any specific value;
allocator state lives in `.apxm/vllm-services/<name>.json`). The container
joins the host network so APXM clients reach the API server on
`http://<HOST>:<PORT>/v1/...`.

For local-only deployments the host is `127.0.0.1`. For shared/remote
hosts the `--host 0.0.0.0` bind is allowed only with an explicit API key
(`VLLM_API_KEY` or `--api-key`); without one, the controller refuses to
start (hard error, not a warning).

## Contract reference

The five `/v1/apxm/*` routes that distinguish the fork from stock vLLM are
documented in [`docs/backends/vllm.md`](../../docs/backends/vllm.md).
