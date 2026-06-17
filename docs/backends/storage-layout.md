# APXM storage layout

This is the operator reference for **where APXM puts large files on disk**.
It covers the storage buckets that grow without bound, the visibility
constraint each one has, and the supported way to relocate any of them
without breaking the Dekk / Slurm pipeline.

This doc is the source of truth referenced from
[`README.md`](../../README.md),
[`CONTRIBUTING.md`](../../CONTRIBUTING.md),
[`backends/model-zoo.md`](model-zoo.md), and
[`backends/model-zoo-quickstart.md`](model-zoo-quickstart.md). Update
this file (not the call sites) when the layout changes.

## TL;DR

| Bucket | Default location | Required visibility | Typical size |
|---|---|---|---|
| Hugging Face model cache | `.apxm/config.toml` → `data.vllm.hf_cache` (or `APXM_VLLM_HF_HOME`) | All compute nodes | 10s – 100s of GiB per model |
| Shared local model roots | `.apxm/config.toml` → `data.vllm.model_roots` (or `APXM_VLLM_MODEL_ROOTS`) | All compute nodes | Existing shared model trees |
| Saved vLLM Docker images | `.apxm/vllm-images/*.docker.tar` (in repo) | All compute nodes | 5 – 15 GiB per image |
| Service state, logs, deploy snapshots | `.apxm/vllm-services/`, `.apxm/vllm-logs/`, `.apxm/deploy/` (in repo) | Controller host only | KiB – MiB |
| Eval/session artifacts (`.apxmobj`, CSVs, diagnostics) | `.apxm/evaluation/`, `.apxm/sessions/` (in repo) | Controller host only | MiB – low GiB |

The model buckets with cross-node visibility requirements (HF cache,
local model roots, and saved Docker images) **must** sit on a filesystem
mounted at the same path on every Slurm-eligible node. Everything else
stays inside the checkout.

## 1. Hugging Face model cache

The vLLM container bind-mounts the resolved cache path at `/models/hf`
and reads model weights from there. `dekk apxm vllm zoo-cache-warm`
writes into the same directory using the Hugging Face hub layout
(`hub/models--<org>--<name>/blobs|snapshots|refs`). Optional
`data.vllm.hf_cache_roots` entries are searched read-only before
launching a service; if a model id is already present in an extra root,
APXM mounts that root as `/models/hf` for the service instead of
duplicating weights.

### Configuration (preferred: config file)

Set the path once in `.apxm/config.toml`. `dekk apxm install` and
`dekk apxm vllm doctor` materialize it from `.apxm/config.example.toml`
on first run, so editing it is the only manual step.

```toml
schema_version = 1

[data]
dir = "/shared/${USER}/.apxm"

# Optional: override one bucket independently of data.dir.
[data.vllm]
# hf_cache = "/shared/models/cache/huggingface"
# hf_cache_roots = ["/shared/models/cache/huggingface/other-namespace"]
# model_roots = ["/shared/models"]
```

Resolution order (highest priority first):

1. CLI flag (e.g. `--hf-home`)
2. Per-resource env var (`APXM_VLLM_HF_HOME`,
   `APXM_VLLM_HF_CACHE_ROOTS`, `APXM_VLLM_MODEL_ROOTS`,
   `APXM_VLLM_IMAGE_STORE`)
3. `APXM_HOME` env var (umbrella for `data.dir`)
4. Project config `<repo>/.apxm/config.toml`
5. Default: `<repo>/.apxm/<bucket>/`

`dekk apxm vllm doctor` prints the resolved layout with source
attribution per field — use it to confirm an override took effect.

### Local model roots

For local path model refs, configure the shared parent directory once:

```toml
[data.vllm]
model_roots = ["/shared/models"]
```

APXM mounts each root read-only at `/models/roots/<n>` and rewrites a
host model path such as `/shared/models/my-model` to the corresponding
container path before invoking `vllm serve`. This keeps local model
trees in shared storage and avoids copying weights into `$HOME` or into
the APXM checkout.

### Hard requirements

1. **The path must resolve to the same content on every Slurm
   compute node**, because the per-service Slurm wrapper
   (`deploy/vllm/run-vllm.sh`) `-v $HF_HOME_HOST:/models/hf` and mounts
   configured model roots on the allocated node. Local-to-login-host
   scratch volumes do not satisfy this — the compute node would see an
   empty directory.
2. **Free space ≥ Σ(model `weights_gb`) × 1.2.** `zoo-cache-warm`
   refuses to start otherwise.
3. **With no config and no env**, the cache resolves to
   `<repo>/.apxm/huggingface/`. That's a safe default for
   single-machine setups, but unusable for multi-node Slurm clusters
   where the repo lives only on the controller.

### Picking a location

| Filesystem class | Use it for the HF cache? |
|---|---|
| Cluster-shared network filesystem (NFS / WekaFS / Lustre) mounted at the same path on login + all compute nodes | **Yes.** This is the supported default. |
| `$HOME` if `$HOME` is the same cluster-shared FS | Only for small single-user setups. Do not use it for shared cluster model weights when `/shared/models` is available. |
| Per-node local scratch (`/scratch`, `/tmp`, NVMe LV) | **No** for a vanilla setup. Only valid if every Slurm allocation is pinned to a single host and you re-warm the cache after every node change. |
| Object storage (S3, GCS, Azure Blob) | No. vLLM expects POSIX. |

A typical cluster config points at the shared cache and shared model
namespace:

```toml
[data.vllm]
hf_cache = "/shared/models/cache/huggingface"
hf_cache_roots = ["/shared/models/cache/huggingface/apxm-cache"]
model_roots = ["/shared/models"]
```

### Migrating an existing cache to a new mount

Safe procedure if an old per-user `$HOME` cache already exists and needs
to move to shared storage:

```bash
OLD="$HOME/.cache/huggingface-apxm-vllm"
NEW="/shared/models/cache/huggingface/$USER"

mkdir -p "$NEW"
rsync -aHAX --info=stats2 "$OLD/" "$NEW/"
diff -rq <(cd "$OLD" && find . -type f | sort) \
         <(cd "$NEW" && find . -type f | sort)   # confirm no drift

rm -rf "$OLD"
ln -s "$NEW" "$OLD"                              # back-compat symlink
# Then set data.vllm.hf_cache or APXM_VLLM_HF_HOME to "$NEW".
```

The symlink lets every doc that still says
`$HOME/.cache/huggingface-apxm-vllm` keep working — Docker bind mounts
follow symlinks at resolution time. Update `.apxm/config.toml` once
you're confident.

Stop any running `dekk apxm vllm` service before relocating; the
container does not handle the bind-mount source disappearing
mid-flight.

## 2. Saved vLLM Docker images (`.apxm/vllm-images/`)

`dekk apxm vllm docker-save` writes each built image as a
`.docker.tar` into `.apxm/vllm-images/`. The per-service Slurm wrapper
runs `docker-load` on the allocated node before launching the
container, so the tarball must be readable from any node that Slurm
might pick.

**Where this must live.** Inside the checkout — i.e. wherever you
cloned the repo. The supported deployment puts the checkout on a
cluster-shared filesystem so every compute node can reach
`.apxm/vllm-images/`. Moving the directory off-tree is not supported
without modifying the wrapper.

**Cleanup.** Old image tarballs are not garbage-collected. Periodically
`ls -lh .apxm/vllm-images/` and remove tarballs whose `image` tag is no
longer referenced by any active service in `.apxm/vllm-services/`.

## 3. Controller-local state (`.apxm/vllm-services/`, `.apxm/vllm-logs/`, `.apxm/deploy/`)

Service records, container stdout/stderr captured by the wrapper, and
`zoo-snapshot.json` files for every `zoo-apply` invocation. These live
inside the checkout and only the controller host needs to read them.
Size stays in the MiB range; rotate logs by deleting
`.apxm/vllm-logs/slurm-*.out` when the corresponding Slurm job is no
longer in `squeue`.

## 4. Evaluation and session artifacts (`.apxm/evaluation/`, `.apxm/sessions/`)

Benchmark CSVs, compiler diagnostics, `.apxmobj` artifacts, and any
other generated evidence go under `.apxm/`. The
`apxm-evaluation-artifacts` skill enforces this — artifacts must not
appear under `examples/` or in the docs source tree.

## Free-space discipline

The controller already runs two pre-checks; rely on them rather than
your memory:

- `dekk apxm vllm doctor` reports the resolved `hf_cache`,
  `hf_cache_roots`, and `model_roots`; run `df -h` on those paths before
  large downloads.
- `dekk apxm vllm zoo-cache-warm` refuses to start when the HF cache
  filesystem has less free space than `Σ(weights_gb) × 1.2`.

For a fast shared-storage check after a long benchmark run:

```bash
df -h /shared/models/cache/huggingface .apxm/vllm-images
du -sh /shared/models/cache/huggingface .apxm/vllm-images .apxm/evaluation \
       .apxm/sessions 2>/dev/null
```

## Cross-references

- [`backends/model-zoo-quickstart.md`](model-zoo-quickstart.md) —
  setup walkthrough; references this doc from step 1.
- [`backends/model-zoo.md`](model-zoo.md) — operator reference; lists
  this doc under "Prerequisites".
- [`vllm-fork.md`](../vllm-fork.md) — contract +
  Dockerized serving path; defers to this doc for storage details.
- `deploy/vllm/run-vllm.sh` — the per-service Slurm wrapper that
  bind-mounts `$HF_HOME_HOST` and `docker-load`s the saved image.
