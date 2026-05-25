# Shared rule — APXM storage layout rules

Load before any session that touches build paths, HF cache, vLLM images,
zoo manifests, or `~/.apxm/config.toml`.

## Filesystem reality

- **`/home`** is shared WekaFS — 9.1 TiB, 50+ tenants. Random ENOSPC
  events come from other tenants filling the volume. Do **not** treat
  `/home` as personal disk. See `apxm_home_is_shared_wekafs` memory.
- **`/tmp`** is 456 GiB node-local. This is where Cargo target dirs
  belong: `CARGO_TARGET_DIR=/tmp/apxm-target-$USER`. Building under
  `/home` causes both contention and random rustc SIGBUS from cache
  blocks the WekaFS layer can't satisfy.
- **HF cache, vLLM images, service registry** must sit on filesystems
  visible from **every Slurm compute node at the same path**. That
  rules out node-local `/tmp` for these. See
  `docs/backends/storage-layout.md`.

## Canonical paths

Use `tools/scripts/apxm_vllm_contract.py::RepoLayout` /
`build_layout()` — never invent path strings:

```python
from tools.scripts.apxm_vllm_contract import build_layout
layout = build_layout(__file__)
# layout.benchmarks_results_dir
# layout.evaluation_dir
# layout.vllm_images_dir
# layout.vllm_services_dir
# layout.hf_home  (or APXM_VLLM_HF_HOME)
```

Confirm the resolved layout at session start: `dekk apxm vllm doctor`.

## Config resolution order

`load_scoped` (the APXM config loader) is **first-wins, not merge**.
This has bitten us:

1. Project-local `.apxm/config.toml` (relative to `pwd`).
2. `~/.apxm/config.toml` (user-global).
3. Defaults.

A project-local config with only a `data-dir` override *shadows the
entire backend block* in `~/.apxm/config.toml`. The symptom is
"no backends configured" from `dekk apxm execute` despite
`dekk apxm backend list` showing them.

**Mitigation**: either keep all config in one file, or fully merge the
backend block into the project-local file. See
`apxm_config_resolver_does_not_merge` memory.

## HF cache deletion gotcha

Blobs under `~/.cache/huggingface-apxm-vllm/hub/<model>/blobs/` are
root-owned (downloads run inside docker as root). A plain `rm` silently
no-ops without freeing space, and `df` doesn't move. Use `sudo rm`. See
`feedback_hf_cache_root_owned` memory.

## Zoo manifests

- `deploy/vllm/zoo.toml` — operator-local, gitignored.
- `deploy/vllm/zoo.example.toml` — committed template.
- `deploy/vllm/zoo.test-*.toml`, `deploy/vllm/zoo.review-*.toml` —
  checked-in smoke-test / evaluation-service manifests; these are
  evidence and may be edited carefully but never overwritten.

`dekk apxm vllm zoo-apply` reconciles state; never edit
`.apxm/vllm-services/` manually.

## Boundaries

- Never `sudo rm -rf` outside `/tmp` or the HF cache path you own.
- Never re-create `~/.apxm/config.toml` from scratch — read first,
  diff, then surgically edit.
- Never commit `zoo.toml` (the operator manifest) — only the templates
  and test/review variants.
