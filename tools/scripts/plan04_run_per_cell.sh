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
#   POWER_CAPTURE (0), CELL_LABEL_SUFFIX, APXM_ON_OPT_LEVELS (2),
#   FLAT_HTTP_OPT_LEVELS (2)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

mapfile -t LAYOUT_LINES < <(
  PYTHONPATH="$REPO_ROOT/tools/scripts${PYTHONPATH:+:$PYTHONPATH}" \
    python3 - "$REPO_ROOT" <<'PY'
import sys
from pathlib import Path
from apxm_vllm_contract import build_layout

layout = build_layout(Path(sys.argv[1]) / "tools/scripts/plan04_run_per_cell.sh")
print(layout.service_dir / "vllm-gptoss.json")
print(layout.log_dir)
print(layout.evaluation_dir / "cross-system")
PY
)

: "${APXM_ENDPOINT:?APXM_ENDPOINT (no trailing /v1) is required}"
export APXM_CONFIG="${APXM_CONFIG:-$HOME/.apxm/config.toml}"
: "${ITER:=6}"
: "${CONC:=32}"
: "${ROWS:=100}"
: "${PRE_REG:=docs/preregistrations/20260519T030358Z-plan04-cross-system.md}"
: "${TS:=$(date -u +%Y%m%dT%H%M%SZ)}"
: "${POWER_CAPTURE:=0}"
: "${CELL_LABEL_SUFFIX:=}"
: "${APXM_ON_OPT_LEVELS:=2}"
: "${FLAT_HTTP_OPT_LEVELS:=2}"
: "${ROCM_SMI_INTERVAL_SECONDS:=2}"

SVC_STATE="${LAYOUT_LINES[0]}"
LOG_DIR="${LAYOUT_LINES[1]}"
CROSS_SYSTEM_DIR="${LAYOUT_LINES[2]}"
OUT_DIR="${CROSS_SYSTEM_DIR}/${TS}"
POWER_HELPER_DIR="$REPO_ROOT/.apxm/evaluation/agentic/_run_template"
mkdir -p "$OUT_DIR"

echo "[plan04-per-cell] run dir: $OUT_DIR"
echo "[plan04-per-cell] endpoint: $APXM_ENDPOINT  iter=$ITER conc=$CONC rows=$ROWS"
echo "[plan04-per-cell] power_capture=$POWER_CAPTURE opt_levels(apxm=$APXM_ON_OPT_LEVELS flat=$FLAT_HTTP_OPT_LEVELS)"

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
  CURRENT_JOB_ID="$jid"
  local log="${LOG_DIR}/slurm-apxm-vllm-service-vllm-gptoss-${jid}.out"
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

truthy() {
  case "${1,,}" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

stop_sidecar() {
  local pid="${1:-}"
  if [ -n "$pid" ]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
}

run_cell() {
  local label="$1"; shift
  local workload="$1"; shift
  local arm_name="$1"; shift
  local driver="$1"; shift
  local workload_arg="$1"; shift
  local arm="$1"; shift
  local out_csv="$1"; shift
  local extra_flags="$1"; shift
  local opt_levels="$1"; shift
  local n_tasks_per_iter="$1"; shift
  echo "[plan04-per-cell] === cell: $label ==="
  # Fresh engine for this cell:
  teardown_service "$(current_job_id)"
  bring_up_service_wait_ready
  reset_cache
  local sidecar_pid=""
  local power_dir="$OUT_DIR/power/${workload}.${arm_name}"
  if truthy "$POWER_CAPTURE"; then
    mkdir -p "$power_dir"
    echo "[plan04-per-cell] starting rocm-smi sidecar for job $CURRENT_JOB_ID -> $power_dir/rocm-smi.csv"
    ROCM_SMI_INTERVAL_SECONDS="$ROCM_SMI_INTERVAL_SECONDS" \
      "$POWER_HELPER_DIR/rocm-smi-sidecar.sh" "$power_dir/rocm-smi.csv" "$CURRENT_JOB_ID" \
      > "$power_dir/sidecar.log" 2>&1 &
    sidecar_pid="$!"
  fi
  echo "[plan04-per-cell] launching cell driver via service-exec"
  local cmd="python3 examples/python/benchmarks/${driver} \
    --apxm-endpoint $APXM_ENDPOINT \
    ${workload_arg} \
    --concurrency $CONC \
    --iterations $ITER \
    --opt-levels $opt_levels \
    --cell-label ${arm}${CELL_LABEL_SUFFIX} \
    --model gpt-oss-120b \
    --output $OUT_DIR/$out_csv \
    --pre-registration $PRE_REG \
    --require-pre-registration $extra_flags"
  set +e
  dekk apxm vllm service-exec vllm-gptoss -- bash -c "$cmd" > "$OUT_DIR/${out_csv}.log" 2>&1
  local cell_status="$?"
  set -e
  stop_sidecar "$sidecar_pid"
  tail -200 "$OUT_DIR/${out_csv}.log"
  if [ "$cell_status" -ne 0 ]; then
    echo "[plan04-per-cell] ERROR: cell failed with status $cell_status ($label)"
    exit "$cell_status"
  fi
  if truthy "$POWER_CAPTURE"; then
    cp "$OUT_DIR/$out_csv" "$power_dir/matrix.csv"
    python3 "$POWER_HELPER_DIR/integrate-joules.py" \
      "$power_dir/rocm-smi.csv" \
      --n-tasks "$((ITER * n_tasks_per_iter))" \
      --arm "$arm_name" \
      > "$power_dir/joules.json" \
      || echo "[plan04-per-cell] WARN: joule integration failed for $label"
  fi
}

# ── 6 cells ──────────────────────────────────────────────────────────────

run_cell "mooncake / apxm-on"   "mooncake" "apxm-on"   "mooncake_replay.py"   "--rows $ROWS"                                                 "M-ON" "mooncake.apxm-on.csv"   ""                "$APXM_ON_OPT_LEVELS"   "$ROWS"
run_cell "mooncake / flat-http" "mooncake" "flat-http" "mooncake_replay.py"   "--rows $ROWS"                                                 "M-FH" "mooncake.flat-http.csv" "--no-apxm-hints" "$FLAT_HTTP_OPT_LEVELS" "$ROWS"
run_cell "sharegpt / apxm-on"   "sharegpt" "apxm-on"   "concurrent_matrix.py" "--graph examples/python/benchmarks/workloads/sharegpt_row.py" "S-ON" "sharegpt.apxm-on.csv"   ""                "$APXM_ON_OPT_LEVELS"   "$CONC"
run_cell "sharegpt / flat-http" "sharegpt" "flat-http" "concurrent_matrix.py" "--graph examples/python/benchmarks/workloads/sharegpt_row.py" "S-FH" "sharegpt.flat-http.csv" "--no-apxm-hints" "$FLAT_HTTP_OPT_LEVELS" "$CONC"
run_cell "loogle / apxm-on"     "loogle"   "apxm-on"   "concurrent_matrix.py" "--graph examples/python/benchmarks/workloads/loogle_row.py"   "L-ON" "loogle.apxm-on.csv"     ""                "$APXM_ON_OPT_LEVELS"   "$CONC"
run_cell "loogle / flat-http"   "loogle"   "flat-http" "concurrent_matrix.py" "--graph examples/python/benchmarks/workloads/loogle_row.py"   "L-FH" "loogle.flat-http.csv"   "--no-apxm-hints" "$FLAT_HTTP_OPT_LEVELS" "$CONC"

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

if truthy "$POWER_CAPTURE"; then
  echo "[plan04-per-cell] computing per-workload J/req bootstrap summaries"
  for workload in mooncake sharegpt loogle; do
    python3 "$POWER_HELPER_DIR/per-iter-energy.py" \
      --apxm-on-dir "$OUT_DIR/power/${workload}.apxm-on" \
      --flat-http-dir "$OUT_DIR/power/${workload}.flat-http" \
      > "$OUT_DIR/jreq.${workload}.json" \
      || echo "[plan04-per-cell] WARN: per-iter J/req bootstrap failed for $workload"
  done
fi

echo "[plan04-per-cell] DONE — $OUT_DIR"
ls -la "$OUT_DIR"
