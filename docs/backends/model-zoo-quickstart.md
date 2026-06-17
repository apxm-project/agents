# APXM vLLM zoo — 15-minute quickstart

Goal: a working APXM-vLLM zoo with one model serving requests, ready
for graph execution. End-to-end runnable copy-paste, no hidden
prerequisites except a Slurm cluster with Docker + ROCm (or CUDA)
nodes.

For the operator manual (schema reference, daily flow, all failure
modes), see [`model-zoo.md`](model-zoo.md). For the contract
(`/v1/apxm/*` routes, fork role), see [`vllm.md`](vllm.md).

---

## 1. Set shared model storage

Prefer `.apxm/config.toml` so login shells, Slurm jobs, and direct
`docker-start` calls all resolve the same shared paths:

```toml
[data.vllm]
hf_cache = "/shared/models/cache/huggingface"
hf_cache_roots = ["/shared/models/cache/huggingface/apxm-cache"]
model_roots = ["/shared/models"]
```

The cache and every model root must live on a filesystem that every
Slurm compute node can read at the same path. `APXM_VLLM_HF_HOME`,
`APXM_VLLM_HF_CACHE_ROOTS`, and `APXM_VLLM_MODEL_ROOTS` are still valid
one-shell overrides, but cluster use should not put model weights in
`$HOME`. The full rules and migration procedure live in
[`storage-layout.md`](storage-layout.md).

## 2. Verify host readiness

```bash
dekk apxm vllm doctor
```

Must report `docker_daemon_ready=true`, `docker_buildx_ready=true`,
and all `slurm_tools` available. The APXM-fork checks under
`APXM_VLLM_DIR` or `../../backends/vllm` must pass.

## 3. Build the runtime image (once per APXM/fork SHA pair)

```bash
APXM_SHA=$(git rev-parse --short HEAD)
VLLM_DIR="${APXM_VLLM_DIR:-../../backends/vllm}"
VLLM_SHA=$(git -C "$VLLM_DIR" rev-parse --short HEAD)
IMAGE="apxm-vllm-runtime:${APXM_SHA}-${VLLM_SHA}-post-rebase"

dekk apxm vllm docker-build --image "$IMAGE" \
    --base-image rocm/vllm-dev:nightly_main_20260411
dekk apxm vllm docker-save  --image "$IMAGE"
```

The save step writes the image to `.apxm/vllm-images/` so the per-node
Slurm wrapper can `docker-load` it onto whatever node Slurm picks.

## 4. Bootstrap your local zoo manifest

```bash
cp deploy/vllm/zoo.example.toml deploy/vllm/zoo.toml
# Edit zoo.toml: replace the <HF_OR_LOCAL_REF> placeholders with one
# real model entry (start with one — add more once you have a green
# baseline).
$EDITOR deploy/vllm/zoo.toml
```

`deploy/vllm/zoo.toml` is **gitignored** so your local zoo doesn't
leak into the repo. The example is the schema reference.

### Minimal one-entry manifest

```toml
schema_version = 1

[defaults]
image = "apxm-vllm-runtime:<APXM_SHA>-<VLLM_SHA>-post-rebase"
scheduling_policy = "priority"
enable_prefix_caching = true
max_num_seqs = 64

[[deployment]]
name = "vllm-mini"
model = "Qwen/Qwen2.5-0.5B"
served_model_name = "Qwen2.5-0.5B"
backend_name = "vllm-mini"
tensor_parallel = 1
replicas = 1
port = 8916
gpus = "0"
weights_gb = 2
max_model_len = 8192
```

Notes:
- `served_model_name` must be the **bare** name (e.g. `Qwen2.5-0.5B`,
  not `Qwen/Qwen2.5-0.5B`). It's what `/v1/models` returns and what
  APXM stores. A mismatch produces silent 404s — the controller
  validates after startup.
- `tensor_parallel` = GPUs per replica. `replicas` × `tensor_parallel`
  must fit on a node (for `gpus`) or be a list of disjoint subsets
  (for `gpu_groups`).
- `port` is required when `replicas == 1`; `port_base` is required
  when `replicas > 1`. No silent allocator fallback inside a
  manifest run.

## 5. Cache-warm the models (CPU-only)

```bash
dekk apxm vllm zoo-cache-warm
```

Walks the manifest's `model` fields and downloads each into the primary
shared `hf_cache` unless the model is already present in a configured
HF cache root or the manifest supplies `hf_home`. No GPU allocation.
Idempotent and resumable.
Refuses to start if the cache filesystem has less free space than
`Σ(weights_gb) × 1.2`.

## 6. Apply the manifest (submits Slurm jobs)

```bash
dekk apxm vllm zoo-apply
```

For each manifest entry without an existing recorded service, this
submits a Slurm job using the unified wrapper. The wrapper on the
allocated node:
1. `docker-load`s the image from the shared archive
2. Starts the vLLM container with the right `--gpus`,
   `--tensor-parallel-size`, `--max-model-len`, etc.
3. Registers the backend in the APXM config
4. Probes `/v1/apxm/scheduler` for `policy = "priority"`

A `zoo-snapshot.json` is written to
`.apxm/deploy/<TIMESTAMP>/` for provenance.

## 7. Watch services come up

```bash
dekk apxm vllm service-list
```

Prints one row per recorded service with name / port / Slurm state /
model. Small models reach `R` (Slurm) within a minute; vLLM startup
itself takes minutes-to-tens-of-minutes depending on weight size and
TP shard count.

When the service is fully up, the per-job log shows
`vLLM is ready at http://127.0.0.1:<PORT>/v1`:

```bash
tail -f .apxm/vllm-logs/slurm-apxm-vllm-service-vllm-mini-*.out
```

## 8. Probe end-to-end

```bash
dekk apxm vllm probe --port 8916
```

Round-trips `POST /v1/apxm/graphs/register`,
`GET /v1/apxm/graphs/{id}`, `DELETE /v1/apxm/graphs/{id}`, and reads
`/v1/apxm/scheduler` to confirm `policy = "priority"`. A failure
here means the served vLLM is not the APXM fork.

```bash
dekk apxm backend list
```

Should show your service registered as an APXM backend.

## 9. Send a request

Direct chat completion (sanity):

```bash
srun --jobid=<JOB_ID> --overlap curl -sw "HTTP=%{http_code}\n" \
  -X POST http://127.0.0.1:8916/v1/chat/completions \
  -H "content-type: application/json" \
  -d '{"model":"Qwen2.5-0.5B","max_completion_tokens":5,
       "messages":[{"role":"user","content":"reply with one word"}]}'
```

Through an APXM workflow:

```bash
export APXM_BENCHMARK_BACKEND=vllm-mini  # disambiguate when zoo has
                                          # multiple `benchmark`
                                          # backends
dekk apxm execute path/to/your/graph.py -O 2 --json
```

`APXM_BENCHMARK_BACKEND` (and `APXM_BENCHMARK_MODEL`) are the
explicit disambiguators read by
`examples/python/benchmarks/stress/_config.py` — the harness's
backend selector. With only one entry in your zoo, you don't need
them.

## 10. When you're done

```bash
# Scale a service to zero (the only auto-stop path; protects peers).
dekk apxm vllm zoo-scale vllm-mini --replicas 0

# Or remove ALL services not in the manifest (requires --prune).
dekk apxm vllm zoo-apply --prune
```

`zoo-apply` never silently cancels jobs — removing a manifest entry
prints a peer-protection warning. Use `zoo-scale --replicas 0` or
`zoo-apply --prune` for explicit cancellation.

---

## Common pitfalls

- **HF cache path looks wrong** — run `dekk apxm vllm doctor` and
  check `hf_cache`, `hf_cache_roots`, and `model_roots` plus their
  `[source]` tags. Override via `.apxm/config.toml`
  (`data.vllm.hf_cache`, `data.vllm.hf_cache_roots`,
  `data.vllm.model_roots`) or the matching APXM_VLLM_* env vars.
- **`required image not supplied`** — pin `image` in `[defaults]`.
  No silent factory default.
- **`zoo manifest not found`** — you forgot step 4. Copy the example.
- **`vLLM server at … is missing /v1/apxm/scheduler`** — the served
  vLLM is upstream vanilla, not the APXM fork. Rebuild from
  `APXM_VLLM_DIR` or `../../backends/vllm`, or register under `protocol=openai` for non-APXM
  evaluation use.
- **`backend/model selection is ambiguous`** — two backends advertise
  the same alias. Set `APXM_BENCHMARK_BACKEND=<NAME>` or
  `APXM_BENCHMARK_MODEL=<SERVED_NAME>` to disambiguate.
- **Reasoning-model crash mid-request** (e.g. gpt-oss-120b)** —
  ensure `reasoning_parser`, `tool_call_parser`, and
  `enable_auto_tool_choice` are set in the manifest entry. A wrong or
  missing reasoning parser will silently crash the engine.
- **Service Slurm state stays `R` but `/v1/models` returns nothing** —
  the container exited but the wrapper's `sleep infinity` keeps the
  job alive. Check `docker ps -a` on the node; restart with
  `dekk apxm vllm zoo-apply` after clearing the state file.

---

## What this gives you

After step 9 succeeds, you have:
- a Slurm-owned vLLM container serving on `<NODE>:<PORT>` with the
  APXM fork's `/v1/apxm/*` routes
- an APXM backend entry routing to that endpoint
- a graph that can execute against it via `dekk apxm execute`
- a provenance snapshot at `.apxm/deploy/<TIMESTAMP>/` for any
  benchmark that follows

Once that works, add more `[[deployment]]` blocks to your zoo for
multiple models or multi-replica throughput. The path scales — every
service is one Slurm job, every replica is one container, and the
APXM resolver round-robins requests across same-model replicas at
runtime.

See [`model-zoo.md`](model-zoo.md) for the operator reference and
[`vllm.md`](vllm.md) for the protocol contract.
