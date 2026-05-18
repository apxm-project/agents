# APXM storage layout

This is the operator reference for **where APXM puts large files on disk**.
It covers the four buckets that grow without bound, the visibility
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
| Saved vLLM Docker images | `.apxm/vllm-images/*.docker.tar` (in repo) | All compute nodes | 5 – 15 GiB per image |
| Service state, logs, deploy snapshots | `.apxm/vllm-services/`, `.apxm/vllm-logs/`, `.apxm/deploy/` (in repo) | Controller host only | KiB – MiB |
| Eval/session artifacts (`.apxmobj`, CSVs, diagnostics) | `.apxm/evaluation/`, `.apxm/sessions/` (in repo) | Controller host only | MiB – low GiB |

The two buckets with cross-node visibility requirements (HF cache and
saved Docker images) **must** sit on a filesystem mounted at the same
path on every Slurm-eligible node. Everything else stays inside the
checkout.

## 1. Hugging Face model cache

The vLLM container bind-mounts the resolved cache path at `/models/hf`
and reads model weights from there. `dekk apxm vllm zoo-cache-warm`
writes into the same directory using the Hugging Face hub layout
(`hub/models--<org>--<name>/blobs|snapshots|refs`).

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
# hf_cache = "/shared/${USER}/.apxm/huggingface-apxm-vllm"
```

Resolution order (highest priority first):

1. CLI flag (e.g. `--hf-home`)
2. Per-resource env var (`APXM_VLLM_HF_HOME`, `APXM_VLLM_IMAGE_STORE`)
3. `APXM_HOME` env var (umbrella for `data.dir`)
4. Project config `<repo>/.apxm/config.toml`
5. Default: `<repo>/.apxm/<bucket>/`

`dekk apxm vllm doctor` prints the resolved layout with source
attribution per field — use it to confirm an override took effect.

### Hard requirements

1. **The path must resolve to the same content on every Slurm
   compute node**, because the per-service Slurm wrapper
   (`deploy/vllm/run-vllm.sh`) `-v $HF_HOME_HOST:/models/hf` on the
   allocated node. Local-to-login-host scratch volumes do not satisfy
   this — the compute node would see an empty directory.
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
| `$HOME` if `$HOME` is the same cluster-shared FS | Yes, when the home quota has room. |
| Per-node local scratch (`/scratch`, `/tmp`, NVMe LV) | **No** for a vanilla setup. Only valid if every Slurm allocation is pinned to a single host and you re-warm the cache after every node change. |
| Object storage (S3, GCS, Azure Blob) | No. vLLM expects POSIX. |

A typical export:

```bash
export APXM_VLLM_HF_HOME="$HOME/.cache/huggingface-apxm-vllm"
```

This default works whenever `$HOME` is the shared FS. On clusters where
`$HOME` is space-constrained or per-user but a separate
`/shared/<user>/` mount has more room, override:

```bash
export APXM_VLLM_HF_HOME="/shared/$USER/.apxm/huggingface-apxm-vllm"
```

### Migrating an existing cache to a new mount

Safe procedure when `$HOME` runs out of space:

```bash
OLD="$HOME/.cache/huggingface-apxm-vllm"
NEW="/shared/$USER/.apxm/huggingface-apxm-vllm"

mkdir -p "$NEW"
rsync -aHAX --info=stats2 "$OLD/" "$NEW/"
diff -rq <(cd "$OLD" && find . -type f | sort) \
         <(cd "$NEW" && find . -type f | sort)   # confirm no drift

rm -rf "$OLD"
ln -s "$NEW" "$OLD"                              # back-compat symlink
export APXM_VLLM_HF_HOME="$NEW"                  # update for new shells
```

The symlink lets every doc that still says
`$HOME/.cache/huggingface-apxm-vllm` keep working — Docker bind mounts
follow symlinks at resolution time. Update the canonical value of
`APXM_VLLM_HF_HOME` in your shell rc once you're confident.

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

- `dekk apxm vllm doctor` reports `hf_home_free_gb`. A 120 GB
  threshold is the warning line.
- `dekk apxm vllm zoo-cache-warm` refuses to start when the HF cache
  filesystem has less free space than `Σ(weights_gb) × 1.2`.

For a fast home-quota check after a long benchmark run:

```bash
df -h "$APXM_VLLM_HF_HOME" .apxm/vllm-images
du -sh "$APXM_VLLM_HF_HOME" .apxm/vllm-images .apxm/evaluation \
       .apxm/sessions 2>/dev/null
```

## Cross-references

- [`backends/model-zoo-quickstart.md`](model-zoo-quickstart.md) —
  setup walkthrough; references this doc from step 1.
- [`backends/model-zoo.md`](model-zoo.md) — operator reference; lists
  this doc under "Prerequisites".
- [`external-vllm-fork.md`](../external-vllm-fork.md) — contract +
  Dockerized serving path; defers to this doc for storage details.
- `deploy/vllm/run-vllm.sh` — the per-service Slurm wrapper that
  bind-mounts `$HF_HOME_HOST` and `docker-load`s the saved image.
