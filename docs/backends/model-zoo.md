# APXM vLLM model zoo

The zoo is the **sole** operator surface for deploying vLLM services under
APXM. `service-start` and `service-adopt` do not exist — every running vLLM
service is owned by an entry in `deploy/vllm/zoo.toml`.

This document is the runbook. The conceptual contract (`/v1/apxm/*` routes,
the role of the fork) lives in [`docs/backends/vllm.md`](vllm.md).

## Quick start

```bash
# 1. Warm the HF cache for every model in the manifest (CPU-only, idempotent,
#    refuses to start if WekaFS free < Σ(weights_gb) × 1.2).
dekk apxm vllm zoo-cache-warm

# 2. Reconcile the manifest against running services. Starts anything missing,
#    warns on services not in the manifest (use --prune to opt in to cancel).
dekk apxm vllm zoo-apply

# 3. Inspect.
dekk apxm vllm zoo-status
dekk apxm vllm service-list

# 4. Scale a single entry up/down.
dekk apxm vllm zoo-scale vllm-qwen3-fast --replicas 2

# 5. Tail container logs for everything zoo-managed.
dekk apxm vllm zoo-logs
```

Every `zoo-apply` writes a provenance artifact to
`.apxm/deploy/<TIMESTAMP>/zoo-snapshot.json` recording the manifest content
and resolved Slurm job ids. Benchmark pre-registration templates must cite
this snapshot path.

## Manifest schema

`deploy/vllm/zoo.toml` is the single source of truth. Schema:

```toml
schema_version = 1

[[deployment]]
# Required
name = "vllm-gptoss"                    # used as Slurm job name + service-state key
model = "openai/gpt-oss-120b"           # HF model id passed to vLLM

# Optional but normally set
served_model_name = "gpt-oss-120b"      # served-id alias (defaults to `model`)
backend_name = "vllm-fork"              # APXM backend name (per-replica suffix added)
nodes = 1                               # Slurm node count; >1 enables Ray
tensor_parallel = 8                     # TENSOR_PARALLEL_SIZE
pipeline_parallel = 1                   # PIPELINE_PARALLEL_SIZE (multi-node only)
replicas = 1                            # 1 = single instance, N = N replicas
port = 8916                             # explicit port (single-replica only)
port_base = 8920                        # base port (replicas=N → port_base..base+N-1)
gpus = "0,1,2,3,4,5,6,7"                # per-instance GPU subset
gpu_groups = [[0,1],[2,3],[4,5],[6,7]]  # per-replica GPU subsets (overrides `gpus`)
ray_port = 6379                         # Ray head port (multi-node only)
weights_gb = 240                        # used by the Σ(weights)*1.2 disk pre-check
max_model_len = 32768
max_num_seqs = 64
scheduling_policy = "priority"          # priority is the only supported value
enable_prefix_caching = true
startup_timeout = 7200
```

`name` must be unique across entries. The manifest expands to one Slurm
service per replica; multi-replica entries use `<name>-r<i>` and
`<port_base>+i`.

## Deployment shapes

The 4-model test set in `deploy/vllm/zoo.toml` exercises every supported
deployment shape.

| Shape | Example | Per-entry layout |
|---|---|---|
| Single-node single-instance | `vllm-gptoss` | TP=8, 1 node, GPUs 0–7 |
| Single-node N-replica | `vllm-qwen3-fast` (replicas=4) | TP=2, 4 replicas on one node, `gpu_groups` shards the 8 GPUs |
| N-node 1-replica per node | `vllm-deepseek` (replicas=2) | TP=8 each on 2 different nodes; ports `port_base..base+1` |
| Multi-node single-instance | `vllm-kimi` (nodes=3, PP=3) | TP=8 PP=3, Ray head on rank 0, workers on ranks 1–2 |

## Storage & log layout

Every path is resolved by `tools/scripts/apxm_vllm_contract.py:RepoLayout`.
**Do not invent parallel paths.**

| Artifact | Path |
|---|---|
| Model weights (HF cache) | `~/.cache/huggingface-apxm-vllm/` (mounted at `/models/hf` inside containers) |
| Service state | `.apxm/vllm-services/<name>.json` |
| Image store | `.apxm/vllm-images/<tag>.docker.tar` + sidecar `.json` |
| Slurm stdout (1-node) | `.apxm/vllm-logs/slurm-apxm-vllm-service-<name>-<jobid>.out` |
| Slurm stdout (Ray N-node) | `.apxm/vllm-logs/slurm-apxm-vllm-service-<name>-<jobid>.node<N>.out` |
| Zoo snapshots | `.apxm/deploy/<TIMESTAMP>/zoo-snapshot.json` |

## Port allocator

Ports are drawn from **[8916, 8999]**. The allocator scans
`.apxm/vllm-services/*.json` for taken ports and returns the first free
candidate; `service-start --port <PORT>` reuses `<PORT>` if free, else picks
the next available and logs the substitution. The range is exhausted as a
hard error — there is no implicit reuse.

## Adding a new model

1. `dekk apxm vllm cache-warm <hf-model-id>` — pulls weights to the shared
   HF cache. Idempotent; resumes partial.
2. Append a `[[deployment]]` block to `deploy/vllm/zoo.toml`.
3. `dekk apxm vllm zoo-apply` — starts the new service alongside existing
   ones. Existing services are left untouched.
4. `dekk apxm vllm probe --port <PORT>` — confirms the APXM graph routes
   work and the scheduler is in `priority` mode.

## Failure modes

| Symptom | Likely cause | Fix |
|---|---|---|
| `zoo cache-warm refused: WekaFS free < required` | Σ(weights_gb)*1.2 exceeds free space on `$HOME` | Free space, or coordinate a shared HF cache namespace with cluster ops |
| `port allocator exhausted` | Every port in [8916, 8999] is bound to a recorded service | `service-stop` orphaned entries; verify with `service-list` |
| `missing /v1/apxm/scheduler` at registration | Stock vLLM image (not the fork) | Verify image tag points to `apxm-vllm-runtime:<APXM>-<VLLM>`; register vanilla vLLM under `OpenAIBackend` instead |
| Ray worker never joins head | Network/firewall blocks `$RAY_PORT` between nodes | Confirm `--exclusive` allocation; check cluster network policy |
| Replica drift (manifest replicas=4, only 3 running) | One replica's Slurm job failed | `zoo-status` shows the gap; `zoo-apply` re-creates the missing replica |
| `probe` returns `policy != "priority"` | Fork running with `--scheduling-policy fcfs` | Per-request priority hints are inert; remove the override or relaunch the service |

## CLI surface

| Subcommand | Status | Notes |
|---|---|---|
| `zoo-apply` / `zoo-status` / `zoo-scale` / `zoo-cache-warm` / `zoo-logs` | **Public** | The entire zoo surface |
| `service-list` / `service-status` / `service-stop` / `service-exec` | **Public** | Operate on zoo-managed services |
| `cache-warm` / `enable` / `probe` / `doctor` / `docker-build` / `docker-save` / `docker-load` | **Public** | Image-lifecycle and operational primitives |
| `service-start` / `service-adopt` | **Not provided** | Write a `zoo.toml` entry and call `zoo-apply` |
| `docker-start` / `docker-stop` / `docker-status` / `docker-logs` | **Internal** | Invoked by `run-vllm.sh`; not part of the public CLI |

## Discipline

> Exactly one well-defined path for every operation. Missing config errors
> loudly at the earliest possible moment — never silently substitutes a
> default, never silently skips, never gracefully degrades, never keeps a
> parallel "old way that still works." The zoo manifest is the **sole**
> source of truth for what runs.

The CI lint at `tools/scripts/check_no_legacy_vllm.py` (also exposed as
`dekk apxm vllm check-no-legacy`) catches regressions: any reintroduction of
the legacy CLI tokens, the `apxm_endpoints_available` capability flag, the
`or env or default` chains, the `Last resort` fallback, or
`SchedulingPolicy.FCFS` fails the build.
