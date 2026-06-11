# Domain — operations

vLLM zoo, service management, Slurm, storage layout.

## Skills

- **apxm-vllm-service** — operate the APXM-vLLM service path.
- **apxm-model-zoo-operate** — manage `deploy/vllm/zoo*.toml`
  manifests.
- **apxm-fork-vllm-rebase** — rebase the `external/vllm` fork.

## Deploy order (non-negotiable)

`docker-load → cache-warm → zoo-apply → service-exec/status`.
`docker-load` and `cache-warm` run without a GPU allocation. See
`apxm_model_zoo_deploy_pattern`.

## Storage reality

- `/home` is shared WekaFS (50+ tenants). Builds belong in
  `/tmp/apxm-target-$USER`. See `apxm_home_is_shared_wekafs`.
- HF cache + vLLM images must be visible from every Slurm compute
  node at the same path.
- HF cache blobs are root-owned; deletion needs `sudo rm`. See
  `feedback_hf_cache_root_owned`.
- Config resolver is **first-wins**, not merge. See
  `apxm_config_resolver_does_not_merge`.

## Slurm safety

Never `scancel` a Slurm job owned by `apxm`. Always allocate a
fresh service job alongside. See `feedback_no_kill_user_slurm`.

## Hardware

8x MI300X 192 GiB (1.5 TiB VRAM), 2x Xeon 8570, 2 TiB RAM. See
`apxm_hardware`.

## Docs

- `docs/backends/model-zoo.md`
- `docs/backends/model-zoo-quickstart.md`
- `docs/backends/storage-layout.md`
- `deploy/vllm/README.md`

## Related rules

- `_shared/apxm-development-rules.md`
- `_shared/apxm-storage-layout-rules.md`
