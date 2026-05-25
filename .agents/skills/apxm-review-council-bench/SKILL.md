---
name: apxm-review-council-bench
description: Use when running the APXM review-council benchmark (multi-judge review). Enforces preregistration, model-zoo deploy order, and artifact placement under .apxm/.
user-invocable: true
---

# APXM Review-Council Benchmark

Load `_shared/apxm-evaluation-rules.md`,
`_shared/apxm-preregistration-rules.md`, and
`_shared/apxm-storage-layout-rules.md` before broad work.

## What it measures

Multi-judge review quality / latency for a council of LLM reviewers
against a workload. Current focus: gpt-oss-120b vs mistral-7b
capped/uncapped (see
`docs/preregistrations/20260521T001540Z-apxm-review-council.md` and
the `-gptoss-capped` follow-up).

## Pre-flight

1. `apxm-preregistration` — must be committed first.
2. **Model zoo deploy order is non-negotiable** (see
   `apxm_model_zoo_deploy_pattern` memory):
   `docker-load → cache-warm → service-start → service-exec`.
   Never let `service-start` trigger an HF download under a GPU
   allocation — the GPU clock runs while bytes stream from
   `huggingface.co`.
3. `model.id` in APXM config must match vLLM's
   `served_model_name` — the bare name (`gpt-oss-120b`), not the HF
   repo (`openai/gpt-oss-120b`). Mismatch produces silent 404s. See
   `feedback_apxm_model_id_must_match_served`.
4. Two services running (one per model) per the zoo manifests
   `deploy/vllm/zoo.review-gptoss.toml` and
   `deploy/vllm/zoo.review-mistral7b.toml`.

## Execution

```bash
# Per service:
dekk apxm vllm zoo-status

# Run the workload via the harness:
bash tools/scripts/run_apxm_review_council.sh         # single model
bash tools/scripts/run_apxm_review_multimodel.sh      # multi-model
```

Artifacts land under `.apxm/evaluation/apxm_review_council/runs/<UTC>/`.

## Quality assessment

```bash
python3 tools/scripts/assess_apxm_review_council_quality.py \
  .apxm/evaluation/apxm_review_council/runs/<UTC>/
```

## Write-up

`apxm-claim-evidence` produces the write-up at
`docs/evaluation/apxm_review_council/<UTC>.md`.

## Anti-patterns

- Letting `service-start` download a model under a GPU allocation
  (wastes GPU-hours; cache-warm first without a GPU).
- `model.id` ≠ `served_model_name` → 404 storm masquerading as
  perf failure.
- Running both models on the same service (port conflict / inference
  contention); use two services with separate zoo manifests.
- Killing the user's Slurm allocation to free GPUs.
