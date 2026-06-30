---
name: model-zoo-operate
group: Domain
description: Use when adding, scaling, or probing models in the vLLM zoo (deploy/vllm/zoo*.toml). Enforces docker-load then cache-warm then zoo-apply then service-exec/status; use the zoo surface only.
user-invocable: true
---

# APXM Model Zoo Operate

Load `_shared/apxm-development-rules.md`,
`_shared/apxm-storage-layout-rules.md` before broad work.

## Deploy order (non-negotiable)

The pattern that prevents GPU-time-on-HF-download incidents:

```
docker-load  →  cache-warm  →  zoo-apply  →  service-exec/status
```

`docker-load` and `cache-warm` run **without** a GPU allocation.
`zoo-apply` reconciles services and is the first GPU-allocating step.
See `apxm_model_zoo_deploy_pattern` memory.

## Canonical commands

```bash
dekk agents vllm doctor                    # env + paths
dekk agents vllm docker-load               # load image from .apxm/vllm-images/
dekk agents vllm zoo-cache-warm            # cache-warm every model in manifest
dekk agents vllm zoo-apply                 # reconcile services with manifest
dekk agents vllm zoo-status                # probe every service
dekk agents vllm zoo-scale <name> <n>      # scale a service's replicas
dekk agents vllm zoo-logs <name>           # tail container logs
dekk agents vllm service-list              # all recorded services
dekk agents vllm service-status <name> --probe
dekk agents vllm service-exec <name> -- <command>
dekk agents vllm service-stop <name>       # cancel a service job
```

## Rules

- **`zoo apply` is the way** — it reconciles services from the manifest.
- `model.id` in APXM config must match the vLLM `served_model_name`
  exactly. Bare name (`gpt-oss-120b`), not HF repo
  (`openai/gpt-oss-120b`). See `feedback_apxm_model_id_must_match_served`.
- Zoo manifests:
  - `deploy/vllm/zoo.toml` — operator-local, gitignored.
  - `deploy/vllm/zoo.example.toml` — committed template.
  - `deploy/vllm/zoo.test-*.toml`, `deploy/vllm/zoo.review-*.toml` —
    checked-in test/eval manifests; edit carefully, never overwrite.
## Hardware reality

`apxm_hardware` memory: 8x MI300X (1.5 TiB VRAM), 2x Xeon 8570,
2 TiB RAM. Plan deployments against this; don't ask for 16 GPUs.

## Anti-patterns

- Letting a GPU-allocating step trigger an HF download. Cache-warm first.
- Editing `~/.apxm/config.toml` with a stray `data-dir` only — silently
  shadows the backend block. See `apxm_config_resolver_does_not_merge`.
- Committing `deploy/vllm/zoo.toml` (the operator manifest).
- `scancel`-ing the user's running services to free GPUs. Always
  allocate fresh. See `feedback_no_kill_user_slurm`.
