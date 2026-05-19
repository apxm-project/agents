#!/usr/bin/env bash
# Plan 04 cross-system paired-arm orchestrator.
#
# Runs 6 cells: {mooncake, sharegpt, loogle} × {apxm-on, flat-http} at c=32
# against the live vllm-gptoss zoo service. Resets the prefix cache between
# workloads via POST /v1/apxm/admin/reset_prefix_cache so RadixAttention
# state from workload N does not leak into workload N+1. Stitches the
# 6 per-cell CSVs via plan04_cross_workload.py at the end.
#
# Required env:
#   APXM_ENDPOINT — base URL of vllm-gptoss (no trailing /v1)
#                   e.g. http://localhost:8916
#
# Optional env:
#   ITER        — paired iterations per cell (default 10)
#   CONC        — concurrency per cell (default 32)
#   ROWS        — rows per Mooncake pass (default 100)
#   PRE_REG     — pre-registration path (default the committed Plan 04 one)
#   TS          — run timestamp (default current UTC)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

: "${APXM_ENDPOINT:?APXM_ENDPOINT (no trailing /v1) is required}"
# The Rust apxm config resolver picks the FIRST .apxm/config.toml found
# walking ancestors; the project-local file holds data-bucket paths only
# (no backends), so without this override `dekk apxm execute` would fail
# with "No backends configured" while `apxm backend list` shows the
# registration in ~/.apxm/config.toml. Forcing the user-level file makes
# the execute path see the backend that `apxm backend add` writes.
export APXM_CONFIG="${APXM_CONFIG:-$HOME/.apxm/config.toml}"
: "${ITER:=10}"
: "${CONC:=32}"
: "${ROWS:=100}"
: "${PRE_REG:=docs/preregistrations/20260519T030358Z-plan04-cross-system.md}"
: "${TS:=$(date -u +%Y%m%dT%H%M%SZ)}"

OUT_DIR=".apxm/evaluation/cross-system/${TS}"
mkdir -p "$OUT_DIR"

echo "[plan04] run dir: $OUT_DIR"
echo "[plan04] endpoint: $APXM_ENDPOINT  iter=$ITER conc=$CONC rows=$ROWS"

reset_cache() {
  echo "[plan04] reset prefix cache"
  curl -fsS -X POST "$APXM_ENDPOINT/v1/apxm/admin/reset_prefix_cache" \
    -H 'content-type: application/json' -d '{}' >/dev/null \
    || echo "[plan04] WARN: reset_prefix_cache returned non-zero"
}

# Capture service state once per run (for the claim's evidence block).
dekk apxm vllm service-status vllm-gptoss --probe 2>/dev/null > "$OUT_DIR/service-state.json" \
  || echo "[plan04] WARN: could not capture service-state"

# ── Mooncake ─────────────────────────────────────────────────────────────
reset_cache
echo "[plan04] cell: mooncake / apxm-on"
python3 examples/python/benchmarks/mooncake_replay.py \
  --apxm-endpoint "$APXM_ENDPOINT" \
  --rows "$ROWS" \
  --concurrency "$CONC" \
  --iterations "$ITER" \
  --opt-levels 2 \
  --cell-label "M-ON" \
  --model gpt-oss-120b \
  --output "$OUT_DIR/mooncake.apxm-on.csv" \
  --pre-registration "$PRE_REG" \
  --require-pre-registration

reset_cache
echo "[plan04] cell: mooncake / flat-http"
python3 examples/python/benchmarks/mooncake_replay.py \
  --apxm-endpoint "$APXM_ENDPOINT" \
  --rows "$ROWS" \
  --concurrency "$CONC" \
  --iterations "$ITER" \
  --opt-levels 2 \
  --no-apxm-hints \
  --cell-label "M-FH" \
  --model gpt-oss-120b \
  --output "$OUT_DIR/mooncake.flat-http.csv" \
  --pre-registration "$PRE_REG" \
  --require-pre-registration

# ── ShareGPT ─────────────────────────────────────────────────────────────
reset_cache
echo "[plan04] cell: sharegpt / apxm-on"
python3 examples/python/benchmarks/concurrent_matrix.py \
  --apxm-endpoint "$APXM_ENDPOINT" \
  --graph examples/python/benchmarks/workloads/sharegpt_row.py \
  --concurrency "$CONC" \
  --iterations "$ITER" \
  --opt-levels 2 \
  --cell-label "S-ON" \
  --model gpt-oss-120b \
  --output "$OUT_DIR/sharegpt.apxm-on.csv" \
  --pre-registration "$PRE_REG" \
  --require-pre-registration

reset_cache
echo "[plan04] cell: sharegpt / flat-http"
python3 examples/python/benchmarks/concurrent_matrix.py \
  --apxm-endpoint "$APXM_ENDPOINT" \
  --graph examples/python/benchmarks/workloads/sharegpt_row.py \
  --concurrency "$CONC" \
  --iterations "$ITER" \
  --opt-levels 2 \
  --no-apxm-hints \
  --cell-label "S-FH" \
  --model gpt-oss-120b \
  --output "$OUT_DIR/sharegpt.flat-http.csv" \
  --pre-registration "$PRE_REG" \
  --require-pre-registration

# ── LooGLE ───────────────────────────────────────────────────────────────
reset_cache
echo "[plan04] cell: loogle / apxm-on"
python3 examples/python/benchmarks/concurrent_matrix.py \
  --apxm-endpoint "$APXM_ENDPOINT" \
  --graph examples/python/benchmarks/workloads/loogle_row.py \
  --concurrency "$CONC" \
  --iterations "$ITER" \
  --opt-levels 2 \
  --cell-label "L-ON" \
  --model gpt-oss-120b \
  --output "$OUT_DIR/loogle.apxm-on.csv" \
  --pre-registration "$PRE_REG" \
  --require-pre-registration

reset_cache
echo "[plan04] cell: loogle / flat-http"
python3 examples/python/benchmarks/concurrent_matrix.py \
  --apxm-endpoint "$APXM_ENDPOINT" \
  --graph examples/python/benchmarks/workloads/loogle_row.py \
  --concurrency "$CONC" \
  --iterations "$ITER" \
  --opt-levels 2 \
  --no-apxm-hints \
  --cell-label "L-FH" \
  --model gpt-oss-120b \
  --output "$OUT_DIR/loogle.flat-http.csv" \
  --pre-registration "$PRE_REG" \
  --require-pre-registration

# ── Cross-workload stitch ────────────────────────────────────────────────
echo "[plan04] stitching combined.csv + summary.json"
python3 examples/python/benchmarks/plan04_cross_workload.py \
  --input "mooncake:$OUT_DIR/mooncake.apxm-on.csv" \
  --input "mooncake:$OUT_DIR/mooncake.flat-http.csv" \
  --input "sharegpt:$OUT_DIR/sharegpt.apxm-on.csv" \
  --input "sharegpt:$OUT_DIR/sharegpt.flat-http.csv" \
  --input "loogle:$OUT_DIR/loogle.apxm-on.csv" \
  --input "loogle:$OUT_DIR/loogle.flat-http.csv" \
  --output "$OUT_DIR/combined.csv" \
  --summary "$OUT_DIR/summary.csv" \
  --manifest "$OUT_DIR/summary.json"

echo "[plan04] DONE — $OUT_DIR"
ls -la "$OUT_DIR"
