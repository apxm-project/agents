#!/usr/bin/env bash
# Per-cell Plan 04 orchestrator with engine restart between cells.
#
# Upstream vLLM v0.21 + gpt-oss-120b + TP=8 + sustained concurrent load
# triggers a TCPStore/HeartbeatMonitor broken-pipe race that kills
# EngineCore after ~20-30 minutes of continuous serving. This driver
# isolates each (workload, arm) cell to its own engine lifetime so a
# crash in cell N does not poison cells N+1..6.
#
# For each of the 6 cells (mooncake/sharegpt/loogle × apxm-on/flat-http)
# this script: scancels the prior service, archives its state record,
# zoo-applies a fresh vllm-gptoss, waits for "vLLM is ready" on the
# slurm log, runs that single cell via `dekk apxm vllm service-exec`,
# and moves on. After all 6 cells, runs plan04_cross_workload.py stitch.
#
# Required env:
#   APXM_ENDPOINT — e.g. http://127.0.0.1:8916
# Optional env:
#   ITER (6), CONC (32), ROWS (100), TS, PRE_REG
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

: "${APXM_ENDPOINT:?APXM_ENDPOINT (no trailing /v1) is required}"
export APXM_CONFIG="${APXM_CONFIG:-$HOME/.apxm/config.toml}"
: "${ITER:=6}"
: "${CONC:=32}"
: "${ROWS:=100}"
: "${PRE_REG:=docs/preregistrations/20260519T030358Z-plan04-cross-system.md}"
: "${TS:=$(date -u +%Y%m%dT%H%M%SZ)}"

OUT_DIR=".apxm/evaluation/cross-system/${TS}"
mkdir -p "$OUT_DIR"
SVC_STATE="/home/apxm/projects/apxm/.apxm/vllm-services/vllm-gptoss.json"
LOG_PAT="/home/apxm/projects/apxm/.apxm/vllm-logs/slurm-apxm-vllm-service-vllm-gptoss-%j.out"

echo "[plan04-per-cell] run dir: $OUT_DIR"
echo "[plan04-per-cell] endpoint: $APXM_ENDPOINT  iter=$ITER conc=$CONC rows=$ROWS"

# ── helpers ──────────────────────────────────────────────────────────────

current_job_id() {
  [ -f "$SVC_STATE" ] && python3 -c "import json,sys; print(json.load(open('$SVC_STATE'))['job_id'])" 2>/dev/null || true
}

teardown_service() {
  local jid="${1:-}"
  if [ -n "$jid" ]; then
    echo "[plan04-per-cell] scancel $jid"
    scancel "$jid" 2>/dev/null || true
    # Wait until squeue stops listing it
    local waited=0
    while squeue -j "$jid" -h 2>/dev/null | grep -q "$jid"; do
      sleep 2; waited=$((waited+2))
      if [ "$waited" -gt 60 ]; then echo "[plan04-per-cell] WARN: scancel of $jid still pending after 60s"; break; fi
    done
    mv -f "$SVC_STATE" "${SVC_STATE}.cancelled-${jid}" 2>/dev/null || true
  fi
}

bring_up_service_wait_ready() {
  echo "[plan04-per-cell] zoo-apply (fresh vllm-gptoss)"
  local out
  out=$(dekk apxm vllm zoo-apply 2>&1)
  local jid
  jid=$(echo "$out" | grep -oE "Submitted batch job [0-9]+" | head -1 | awk '{print $4}')
  if [ -z "$jid" ]; then
    echo "[plan04-per-cell] ERROR: zoo-apply did not submit a job"
    echo "$out" | tail -20
    exit 1
  fi
  echo "[plan04-per-cell] new vllm-gptoss job_id=$jid"
  local log="/home/apxm/projects/apxm/.apxm/vllm-logs/slurm-apxm-vllm-service-vllm-gptoss-${jid}.out"
  local waited=0
  local max_wait=900
  while [ ! -f "$log" ]; do
    sleep 5; waited=$((waited+5))
    if [ "$waited" -gt "$max_wait" ]; then
      echo "[plan04-per-cell] ERROR: log $log never appeared after ${max_wait}s"
      exit 1
    fi
  done
  echo "[plan04-per-cell] log present; waiting for ready signal"
  waited=0
  max_wait=900
  while ! grep -q "vLLM is ready" "$log" 2>/dev/null; do
    if grep -qE "EngineDeadError|CUDA out of memory|FATAL" "$log" 2>/dev/null; then
      echo "[plan04-per-cell] ERROR: startup failed; tail of $log:"
      tail -30 "$log"
      exit 1
    fi
    sleep 5; waited=$((waited+5))
    if [ "$waited" -gt "$max_wait" ]; then
      echo "[plan04-per-cell] ERROR: ready signal never arrived in ${max_wait}s"
      tail -30 "$log"
      exit 1
    fi
  done
  echo "[plan04-per-cell] vllm-gptoss $jid READY"
  # Capture service-state once per run on first cell only
  if [ ! -f "$OUT_DIR/service-state.json" ]; then
    dekk apxm vllm service-status vllm-gptoss --probe 2>/dev/null > "$OUT_DIR/service-state.json" \
      || echo "[plan04-per-cell] WARN: could not capture service-state"
  fi
}

reset_cache() {
  curl -fsS -X POST "$APXM_ENDPOINT/v1/apxm/admin/reset_prefix_cache" \
    -H 'content-type: application/json' -d '{}' >/dev/null \
    || echo "[plan04-per-cell] WARN: reset_prefix_cache returned non-zero"
}

run_cell() {
  local label="$1"; shift
  local driver="$1"; shift
  local workload_arg="$1"; shift
  local arm="$1"; shift
  local out_csv="$1"; shift
  local extra_flags="$1"; shift
  echo "[plan04-per-cell] === cell: $label ==="
  # Fresh engine for this cell:
  teardown_service "$(current_job_id)"
  bring_up_service_wait_ready
  reset_cache
  echo "[plan04-per-cell] launching cell driver via service-exec"
  local cmd="python3 examples/python/benchmarks/${driver} \
    --apxm-endpoint $APXM_ENDPOINT \
    ${workload_arg} \
    --concurrency $CONC \
    --iterations $ITER \
    --opt-levels 2 \
    --cell-label $arm \
    --model gpt-oss-120b \
    --output $OUT_DIR/$out_csv \
    --pre-registration $PRE_REG \
    --require-pre-registration $extra_flags"
  dekk apxm vllm service-exec vllm-gptoss -- bash -c "$cmd" 2>&1 | tail -200
}

# ── 6 cells ──────────────────────────────────────────────────────────────

run_cell "mooncake / apxm-on"  "mooncake_replay.py"    "--rows $ROWS"                                                          "M-ON"  "mooncake.apxm-on.csv"   ""
run_cell "mooncake / flat-http" "mooncake_replay.py"    "--rows $ROWS"                                                          "M-FH"  "mooncake.flat-http.csv" "--no-apxm-hints"
run_cell "sharegpt / apxm-on"  "concurrent_matrix.py"  "--graph examples/python/benchmarks/workloads/sharegpt_row.py"          "S-ON"  "sharegpt.apxm-on.csv"   ""
run_cell "sharegpt / flat-http" "concurrent_matrix.py"  "--graph examples/python/benchmarks/workloads/sharegpt_row.py"          "S-FH"  "sharegpt.flat-http.csv" "--no-apxm-hints"
run_cell "loogle / apxm-on"    "concurrent_matrix.py"  "--graph examples/python/benchmarks/workloads/loogle_row.py"            "L-ON"  "loogle.apxm-on.csv"     ""
run_cell "loogle / flat-http"  "concurrent_matrix.py"  "--graph examples/python/benchmarks/workloads/loogle_row.py"            "L-FH"  "loogle.flat-http.csv"   "--no-apxm-hints"

# ── stitch ──────────────────────────────────────────────────────────────
echo "[plan04-per-cell] stitching combined.csv + summary.json"
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

echo "[plan04-per-cell] DONE — $OUT_DIR"
ls -la "$OUT_DIR"
