# Evaluation Plans

This directory holds tracked, publication-facing evaluation plans and run
recipes. Generated CSVs, service probes, sessions, matrices, logs, and
claim evidence belong under `.apxm/evaluation/`.

- [APXM Results, 2026-05-21](apxm-results-20260521.md) — concise
  publication-facing summary of the current APXM evidence. Headline: APXM
  graph-aware priority hints reduce user-visible critical-lane latency under
  background LLM queue contention. Boundary: this is not a broad APXM speedup,
  batch-wall, or prefix-pinning claim.
- [APXM Paper Draft](../paper/apxm-paper-draft.md) — working Markdown paper
  draft with generated SVG figures, narrative, vision, evaluation, limitations,
  and future work.
- [APXM Review Council](apxm-review-council.md) — dogfood APXM-vLLM workflow
  for shared-context fanout, graph hints, priority scheduling, prefix-cache
  telemetry, and APXM-on vs flat-HTTP comparison. Current 2026-05-21 result:
  mechanism engaged end-to-end, but GPT-OSS latency is negative under
  batch-wall evaluation.
- [APXM Priority Lane](apxm-priority-lane.md) — pre-registered APXM-vLLM
  priority-scheduling workload for user-visible critical-lane latency under
  background LLM queue contention. Current 2026-05-21 result: promoted
  positive on focus-node finish time. Across two fresh-service interleaved
  repeats: mean APXM/flat ratio `0.661` (95% CI `[0.598, 0.729]`) and p95
  ratio `0.532` (95% CI `[0.320, 0.590]`), with `320` paired tenant rows,
  zero failed tenants, APXM honoring `priority`, and combined batch-level
  focus ratio `0.661` (95% CI `[0.445, 0.963]`) across `20` independent
  batch units.
