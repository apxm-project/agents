# Comparable workloads

Workloads in this directory follow a strict env-var contract so the existing
`concurrent_matrix.py` driver can launch them as `--graph workloads/<name>.py`
and collect the same `BatchRow` CSV schema across very different workload
shapes.

The companion methodology charter is at
`docs/plans/00-evaluation-methodology.md`. This directory implements the
"comparable workloads" track described in
`docs/plans/03-comparable-workloads.md`.

## Tiers

**Tier 1 — Synthetic (in-process graph scripts).** Each workload is a Python
graph script with knobs read from env vars. The driver invokes them via
`dekk apxm execute`. Currently shipped:

- `gsp.py` — Generated Shared Prefix. Knobs: prefix tokens, fanout,
  cohort size. Controlled microbench for cohort-routing claims.
- `pfx_cancel.py` — Parallel-fanout with cancellation. One shared prefix,
  N branches, M < N expected to be pruned. The cancellation hook is wired
  but the in-flight-cancel surface is Plan-08; until then `cancel_count`
  populates from compiler-side dead-context elimination only.

**Tier 2 — Dataset-bound replay drivers.** Mooncake trace, ShareGPT
multi-turn, LooGLE long-shared-context. These need separate driver scripts
because they replay real per-row prefix structure rather than running a
static graph script per tenant. Currently shipped:

- **Mooncake** — `examples/python/benchmarks/mooncake_replay.py` (driver)
  + `workloads/mooncake_row.py` (per-row graph) + `workloads/data/`
  (vendored 500-row sample + fetcher for full trace). Closed-loop
  subprocess dispatch via `dekk apxm execute`. Honors the trace's
  prefix-cache cohort structure via `_mooncake_hash.synthesize_prompt`
  (the upstream trace is privacy-redacted; prompt text is synthesized
  deterministically from `hash_ids`). Open-loop arrival timing is a
  follow-up wave — see `mooncake_replay.py` docstring.

- **ShareGPT multi-turn (W4b)** — `workloads/sharegpt_row.py` (per-row
  graph) + `workloads/data/sharegpt_sample.jsonl` (3-conversation
  vendored smoke sample) + `workloads/data/fetch_sharegpt.sh`
  (idempotent fetcher for `ShareGPT_V3_unfiltered_cleaned_split.json`
  into gitignored `.apxm/datasets/sharegpt/`). The row adapter emits a
  sequential ASK chain — one node per `human` turn, each prompted with
  the full prior-turn history — so the rendered prefix grows
  monotonically and RadixAttention / the APXM pin path see the real
  multi-turn prefix-cache pattern. Env contract:
  `SHAREGPT_CONVERSATION_JSON`, `SHAREGPT_ROW_INDEX`,
  `SHAREGPT_MAX_TURNS` (default 8).

- **LooGLE long-shared-context (W4c)** — `workloads/loogle_row.py`
  (per-row graph) + `workloads/data/loogle_sample.jsonl`
  (2-document vendored smoke sample) + `workloads/data/fetch_loogle.sh`
  (fetcher for `longdep_qa.jsonl` + `shortdep_qa.jsonl` from
  `bigai-nlco/LooGLE` into gitignored `.apxm/datasets/loogle/`). The
  row adapter emits a fan-out graph: one ASK per sub-question, each
  prompted with `document + question`. The document is byte-identical
  across all questions in a row, so the prefix cache sees the
  document once and reuses it for every question — the canonical
  long-shared-context speedup pattern. Env contract:
  `LOOGLE_DOCUMENT`, `LOOGLE_QUESTIONS_JSON`, `LOOGLE_ROW_INDEX`,
  `LOOGLE_MAX_QUESTIONS` (default 6).

## Env-var contract

All Tier-1 workloads honor these env vars (defaults in `_helpers.py`):

| Var | Default | Meaning |
|---|---|---|
| `APXM_MATRIX_VARIANT` | 0 | Tenant index assigned by the driver |
| `APXM_WORKLOAD_PREFIX_TOK` | 1024 | Target token count for the shared prefix |
| `APXM_WORKLOAD_FANOUT` | 8 | Number of parallel branches |
| `APXM_WORKLOAD_COHORT_SIZE` | 1 | Variants per cohort sharing one prefix |
| `APXM_WORKLOAD_CANCEL_RATE` | 0.0 | Fraction of branches expected to be cancelled |

Workloads ignore unknown values gracefully (defaults applied).

## Adding a new workload

1. Create `workloads/<name>.py` following the pattern of `gsp.py`.
2. Read knobs via `_helpers.env_int()` / `env_float()`; never hardcode.
3. Build the graph with `apxm.GraphRecorder`; emit one `g.done(...)` node.
4. Add a row to the table above documenting any new env vars.
5. Smoke-test with `dekk apxm execute workloads/<name>.py -O0`.
6. Wire into a benchmark cell by passing `--graph workloads/<name>.py` to
   `concurrent_matrix.py`.

## Running Tier-1 (synthetic)

```bash
# Single ad-hoc invocation with default knobs:
dekk apxm execute examples/python/benchmarks/workloads/gsp.py -O2

# With custom knobs:
APXM_WORKLOAD_PREFIX_TOK=4096 APXM_WORKLOAD_FANOUT=16 \
  dekk apxm execute examples/python/benchmarks/workloads/gsp.py -O2

# As a concurrent matrix cell (4 concurrent tenants, prefix-cache on, priority):
python3 examples/python/benchmarks/concurrent_matrix.py \
  --graph examples/python/benchmarks/workloads/gsp.py \
  --concurrency 4 --iterations 5 --opt-levels 0 2 \
  --output .apxm/benchmarks/results/gsp-concurrent-matrix.csv \
  --metrics-url "${APXM_ENDPOINT}/metrics"
```

## Running Tier-2 (Mooncake)

```bash
# Driver-only smoke (no vLLM required) — validates the data pipeline
# end to end through comparison_report's arm-comparison section.
python3 examples/python/benchmarks/mooncake_replay.py --smoke

# Optional: fetch the full trace into .apxm/datasets/mooncake/ (gitignored)
bash examples/python/benchmarks/workloads/data/fetch_mooncake.sh

# Single-row dispatch smoke (vLLM running):
python3 examples/python/benchmarks/mooncake_replay.py \
  --rows 1 --concurrency 1 --opt-levels 2 --iterations 1 \
  --output /tmp/mooncake_smoke.csv

# Paired arms (run both, then combine for comparison_report):
python3 examples/python/benchmarks/mooncake_replay.py \
  --rows 50 --concurrency 4 --opt-levels 0 2 --iterations 5 \
  --output /tmp/mooncake_apxm.csv
python3 examples/python/benchmarks/mooncake_replay.py \
  --rows 50 --concurrency 4 --opt-levels 0 2 --iterations 5 \
  --no-apxm-hints --output /tmp/mooncake_flat.csv
cat /tmp/mooncake_apxm.csv > /tmp/mooncake_paired.csv
tail -n +2 /tmp/mooncake_flat.csv >> /tmp/mooncake_paired.csv
python3 examples/python/benchmarks/comparison_report.py /tmp/mooncake_paired.csv
```

## Companion paper alignment

The Tier-1 synthetics are *diagnostic* — they isolate single dispatch
features under controlled conditions but do not stand alone as evidence
for any cross-system claim. Real evidence requires at least one Tier-2
workload (production trace replay) or one Tier-3 (agentic / dogfooding)
workload.
