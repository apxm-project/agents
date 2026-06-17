# APXM vLLM model zoo — operator guide

The zoo is the **sole** operator surface for deploying vLLM services
under APXM. Every running vLLM service is owned by an entry in your zoo
manifest.

This document is the getting-started guide. The conceptual contract
(`/v1/apxm/*` routes, the role of the fork) lives in
[`vllm.md`](vllm.md).

---

## What this gives you

- One vLLM container per service, scheduled by Slurm onto any node in
  the cluster.
- Model-agnostic: any model vLLM can load (HF id, local path, mirror)
  is a one-line entry in the manifest. APXM can search multiple HF cache
  roots and bind-mount shared local model roots without copying weights.
- Three deployment shapes out of the box: single-instance,
  multi-instance on one node (GPU groups), and multi-instance across
  nodes (replicas). Multi-node Ray (cross-node TP+PP for one model) is
  out of scope — keep one service per node.
- Round-robin dispatch across replicas of the same `served_model_name`,
  baked into the APXM resolver.

---

## Prerequisites (do once per cluster)

```bash
# 1. Put model storage on a shared filesystem visible from every Slurm
#    compute node. Prefer .apxm/config.toml so Slurm jobs and direct
#    docker-start calls resolve the same roots.
#
#    Minimal C42-style shape:
#      data.vllm.hf_cache = "/shared/models/cache/huggingface"
#      data.vllm.model_roots = ["/shared/models"]
#
#    APXM_VLLM_HF_HOME still overrides the primary HF cache for one shell.

# 2. Verify host readiness (Docker, buildx, Slurm tools, fork SHA).
dekk apxm vllm doctor

# 3. Build the APXM-vLLM runtime image. Pick a tag that records the
#    APXM + vllm-fork SHAs so artifacts are reproducible.
APXM_SHA=$(git rev-parse --short HEAD)
VLLM_DIR="${APXM_VLLM_DIR:-../../backends/vllm}"
VLLM_SHA=$(git -C "$VLLM_DIR" rev-parse --short HEAD)
IMAGE="apxm-vllm-runtime:${APXM_SHA}-${VLLM_SHA}-post-rebase"

dekk apxm vllm docker-build --image "$IMAGE" \
    --base-image rocm/vllm-dev:nightly_main_20260411  # or your base

# 4. Save the image to the shared archive store so worker nodes can
#    docker-load it from WekaFS.
dekk apxm vllm docker-save --image "$IMAGE"
```

The image is portable across nodes via the saved `.tar` in
`.apxm/vllm-images/`; the per-service Slurm wrapper loads it on the
allocated node before starting the container.

---

## Bootstrap your zoo manifest

```bash
cp deploy/vllm/zoo.example.toml deploy/vllm/zoo.toml
$EDITOR deploy/vllm/zoo.toml
```

`deploy/vllm/zoo.toml` is **gitignored**. The example file is a
template covering the three deployment shapes. Replace the
`example-*` entries with your own.

### Schema (per `[[deployment]]`)

| Field | Required | Description |
|---|---|---|
| `name` | yes | Service name (unique within the manifest). |
| `model` | yes | What vLLM loads. HF id (`openai/gpt-oss-120b`), local path, or any vLLM-supported ref. |
| `served_model_name` | yes | The bare name `/v1/models` will serve. Must match what APXM stores — typically the model name without the HF org/repo prefix. A mismatch produces silent 404s. |
| `backend_name` | yes | The APXM backend name the registry uses. Distinct per replica. |
| `tensor_parallel` | yes | GPUs per replica (TP shard count). |
| `replicas` | no (default 1) | Number of identical containers to spawn. |
| `port` | when `replicas == 1` | Listener port. |
| `port_base` | when `replicas > 1` | First replica gets `port_base`, second `port_base + 1`, etc. |
| `gpus` | one of these two | Comma-separated GPU index list (e.g. `"0,1,2,3,4,5,6,7"`). |
| `gpu_groups` | one of these two | List of GPU lists, one per replica (e.g. `[[0,1],[2,3]]`). |
| `weights_gb` | recommended | Approx model size; used by the capacity pre-check. |
| `max_model_len`, `max_num_seqs` | no | Forwarded to vLLM. |
| `image` | no (inherits `[defaults].image`) | Per-deployment image override. |
| `hf_home` | no | Per-deployment HF cache override. Usually leave unset; APXM searches configured HF cache roots and chooses the first root containing the model id. |
| `model_roots` | no | Extra host directories to mount read-only for local-path model refs. Inherits `[defaults]`; merged with `data.vllm.model_roots`. |
| `scheduling_policy` | no (default `priority`) | Only `priority` is accepted; FCFS is not supported by APXM. |
| `enable_prefix_caching` | no (default true) | Toggle vLLM's prefix cache. |
| `reasoning_parser` | model-specific | Only set when the model needs it (e.g. `openai_gptoss`). A wrong value crashes startup with a vocab `KeyError`. |
| `tool_call_parser` | model-specific | Same. |
| `enable_auto_tool_choice` | model-specific | Same. |

`[defaults]` at the top of the manifest applies to every entry unless
overridden. Pin `image` there so `zoo-apply` never falls through to env.
Storage roots are usually better in `.apxm/config.toml`; use manifest
`hf_home` or `model_roots` only when one deployment needs a different
cache or local model tree.

### Deployment shapes (illustrated in `zoo.example.toml`)

- **Single-instance**: `replicas = 1`, all node GPUs in one TP group.
  Maximizes per-request throughput for one big model.
- **Multi-instance on one node**: `replicas = N` with `gpu_groups`.
  Each replica pins a disjoint GPU subset on the same node. Use for
  smaller models when N parallel servers beats one big TP group.
- **Multi-instance across nodes**: `replicas = N` with `gpus`. Each
  replica gets its own Slurm allocation; APXM round-robins requests
  across them at the registry layer.

If a model truly exceeds one node, register it under a non-APXM
backend protocol or shrink TP / `max_model_len` until it fits.

---

## Daily flow

```bash
# Warm the HF cache for every model in the manifest.
# CPU-only, idempotent. Refuses to start if WekaFS free space <
# Σ(weights_gb) × 1.2.
dekk apxm vllm zoo-cache-warm

# Reconcile: submit Slurm jobs for any manifest entry that does not
# already have a recorded service. Idempotent — re-runs probe healthy
# services, never restarts them.
dekk apxm vllm zoo-apply

# Watch services come up.
dekk apxm vllm service-list

# Probe each endpoint for /v1/models + /v1/apxm/* round-trip.
dekk apxm vllm probe --port <PORT>

# Scale a service down to 0 (the only explicit-removal path; zoo-apply
# never silently cancels jobs that have peer-protection value).
dekk apxm vllm zoo-scale <NAME> --replicas 0

# Tail logs.
dekk apxm vllm zoo-logs <NAME>
```

`zoo-apply` writes a snapshot of the resolved manifest + Slurm job ids
to `.apxm/deploy/<TIMESTAMP>/zoo-snapshot.json` on every run, so any
benchmark or claim can be traced back to the exact zoo state at the
moment of submission.

---

## Adding a new model

1. `dekk apxm vllm cache-warm <MODEL_REF>` (one-time, CPU-only). APXM
   writes to the primary shared HF cache unless the model is already in
   a configured cache root or `--hf-home` / `hf_home` is supplied.
2. Append a `[[deployment]]` block to your `deploy/vllm/zoo.toml`.
3. `dekk apxm vllm zoo-apply` — only the new entry will be submitted.
4. `dekk apxm vllm probe --port <PORT>` confirms the APXM router is up.
5. The backend is automatically registered in the APXM config; verify
   with `dekk apxm backend list`.

Model-specific parser fields (`reasoning_parser`, `tool_call_parser`,
`enable_auto_tool_choice`) are **never** defaulted by the wrapper. If
the model needs them, set them in the manifest entry. If you set the
wrong one, vLLM crashes with a vocab `KeyError` at startup — the
container log will tell you which token is missing.

---

## Verifying a multi-replica service round-robins

The APXM resolver's `find_backend_for_model` collects every backend
matching the requested model name and dispatches via the configured
strategy (default round-robin). Unit tests at
`crates/runtime/apxm-backends/src/llm/registry/resolver.rs` cover the
algorithm; a behavioral check inspects the APXM trace after sending N
requests through the dispatcher.

---

## Failure modes worth knowing

- **HF cache path looks wrong** — run `dekk apxm vllm doctor` and
  check `hf_cache`, `hf_cache_roots`, and `model_roots` plus their
  `[source]` tags. Override via `.apxm/config.toml`
  (`data.vllm.hf_cache`, `data.vllm.hf_cache_roots`,
  `data.vllm.model_roots`) or the matching APXM_VLLM_* env vars.
- **Local path cannot be loaded** — make sure the host path is under
  one configured `model_roots` entry. APXM rewrites that host path to
  `/models/roots/<n>/...` before starting vLLM.
- **`required image not supplied`** — pin `image` in `[defaults]` or
  pass via env. No silent factory default.
- **`zoo manifest not found`** — copy `zoo.example.toml` to
  `zoo.toml`.
- **`vLLM server at … is missing /v1/apxm/scheduler`** — the endpoint
  is vanilla upstream vLLM, not the APXM fork. Either rebuild the
  image from `APXM_VLLM_DIR` or `../../backends/vllm`, or register it under `protocol=openai`.
- **`no healthy backends for <model>`** — all replicas are
  Unhealthy/Unknown. Check `service-list` and the container logs.
- **`port already in use`** — manifest port collides with a running
  service or another tenant. The allocator (when used by single-entry
  CLI) picks the next free port in `8916–8999`; manifest ports are
  required to be explicit.

---

## Cleanup

```bash
# Remove a service.
dekk apxm vllm zoo-scale <NAME> --replicas 0

# Remove ALL services NOT in the current manifest (confirmation by
# explicit flag; this is the only way zoo-apply auto-cancels jobs).
dekk apxm vllm zoo-apply --prune
```

The APXM backend registry deregistration is wired into the container
cleanup trap, so when a Slurm job ends, the backend entry is removed
automatically. Stale entries should never accumulate.

---

## Cross-references

- [`vllm.md`](vllm.md) — concept doc: contract, fork role, route list.
- [`storage-layout.md`](storage-layout.md) — where the HF cache, the saved image store, and runtime artifacts live on disk (and how to relocate them).
- [`../../deploy/vllm/zoo.example.toml`](../../deploy/vllm/zoo.example.toml) — bootstrap template.
- [`../../deploy/vllm/run-vllm.sh`](../../deploy/vllm/run-vllm.sh) — the Slurm wrapper invoked by every service.
