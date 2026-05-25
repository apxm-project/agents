#!/usr/bin/env bash
# Run the APXM priority-lane contention evaluation against a Dekk-managed vLLM service.
#
# Generated evidence is written under:
#   .apxm/evaluation/apxm-priority-lane/runs/<UTC>/
#
# Optional env:
#   SERVICE (vllm-gptoss), APXM_ENDPOINT (http://127.0.0.1:8916), ITER (10),
#   CONC (16), PREFIX_TOK (2048), BG_FANOUT (10), MODEL (gpt-oss-120b),
#   CRITICAL_MAX_TOKENS (96), BACKGROUND_MAX_TOKENS (384), FOCUS_NODE_ID (4),
#   PRE_REG, TS, SMOKE (0), REQUIRE_PRE_REG (1), STAGGER_MS (0),
#   ARM_ORDER (apxm,flat or flat,apxm)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

mapfile -t LAYOUT_LINES < <(
  PYTHONPATH="$REPO_ROOT/tools/scripts${PYTHONPATH:+:$PYTHONPATH}" \
    python3 - "$REPO_ROOT" <<'PY'
import sys
from pathlib import Path
from apxm_vllm_contract import build_layout

layout = build_layout(Path(sys.argv[1]) / "tools/scripts/run_apxm_priority_lane.sh")
print(layout.evaluation_dir / "apxm-priority-lane")
print(layout.service_dir)
PY
)

: "${SERVICE:=vllm-gptoss}"
: "${APXM_ENDPOINT:=http://127.0.0.1:8916}"
: "${ITER:=10}"
: "${CONC:=16}"
: "${PREFIX_TOK:=2048}"
: "${BG_FANOUT:=10}"
: "${CRITICAL_MAX_TOKENS:=96}"
: "${BACKGROUND_MAX_TOKENS:=384}"
: "${FOCUS_NODE_ID:=4}"
: "${MODEL:=gpt-oss-120b}"
: "${PRE_REG:=docs/preregistrations/20260521T030206Z-apxm-priority-lane.md}"
: "${TS:=$(date -u +%Y%m%dT%H%M%SZ)}"
: "${SMOKE:=0}"
: "${REQUIRE_PRE_REG:=1}"
: "${STAGGER_MS:=0}"
: "${ARM_ORDER:=apxm,flat}"
: "${WORKLOAD_APXM_CONFIG:=$HOME/.apxm/config.toml}"

if [[ "${SMOKE,,}" =~ ^(1|true|yes|on)$ ]]; then
  ITER=2
  CONC=2
  PREFIX_TOK=512
  BG_FANOUT=3
  CRITICAL_MAX_TOKENS=64
  BACKGROUND_MAX_TOKENS=96
fi

RUN_ROOT="${LAYOUT_LINES[0]}"
SERVICE_DIR="${LAYOUT_LINES[1]}"
OUT_DIR="${RUN_ROOT}/runs/${TS}"
mkdir -p "$OUT_DIR"

echo "[priority-lane] run dir: $OUT_DIR"
echo "[priority-lane] service=$SERVICE endpoint=$APXM_ENDPOINT model=$MODEL"
echo "[priority-lane] iter=$ITER conc=$CONC prefix_tok=$PREFIX_TOK bg_fanout=$BG_FANOUT stagger_ms=$STAGGER_MS smoke=$SMOKE"
echo "[priority-lane] critical_max_tokens=$CRITICAL_MAX_TOKENS background_max_tokens=$BACKGROUND_MAX_TOKENS focus_node_id=$FOCUS_NODE_ID"
echo "[priority-lane] arm_order=$ARM_ORDER"

if [ ! -f "$PRE_REG" ]; then
  echo "[priority-lane] ERROR: pre-registration not found: $PRE_REG" >&2
  exit 2
fi

SERVICE_STATE="${SERVICE_DIR}/${SERVICE}.json"
dekk apxm vllm service-status "$SERVICE" --probe > "$OUT_DIR/service-probe.txt"
cp "$SERVICE_STATE" "$OUT_DIR/service-state.json"

COMMON_FLAGS=(
  --graph examples/python/benchmarks/workloads/apxm_priority_lane.py
  --apxm-endpoint "$APXM_ENDPOINT"
  --concurrency "$CONC"
  --iterations "$ITER"
  --stagger-ms "$STAGGER_MS"
  --model "$MODEL"
  --focus-node-id "$FOCUS_NODE_ID"
  --pre-registration "$PRE_REG"
)

if [[ "${REQUIRE_PRE_REG,,}" =~ ^(1|true|yes|on)$ ]]; then
  COMMON_FLAGS+=(--require-pre-registration)
fi

run_cell() {
  local label="$1"
  local arm="$2"
  local output="$3"
  shift 3

  echo "[priority-lane] === $label ==="
  dekk apxm vllm service-exec "$SERVICE" -- \
    env APXM_CONFIG="$WORKLOAD_APXM_CONFIG" \
    APXM_WORKLOAD_PREFIX_TOK="$PREFIX_TOK" \
    APXM_WORKLOAD_FANOUT="$BG_FANOUT" \
    APXM_PRIORITY_CRITICAL_MAX_TOKENS="$CRITICAL_MAX_TOKENS" \
    APXM_PRIORITY_BACKGROUND_MAX_TOKENS="$BACKGROUND_MAX_TOKENS" \
    python3 examples/python/benchmarks/concurrent_matrix.py \
      "${COMMON_FLAGS[@]}" \
      --opt-levels 2 \
      --cell-label "$arm" \
      --output "$OUT_DIR/$output" \
      "$@" \
    > "$OUT_DIR/${output}.log" 2>&1
  tail -120 "$OUT_DIR/${output}.log"
}

IFS=',' read -r -a ARM_ORDER_PARTS <<< "$ARM_ORDER"
for requested_arm in "${ARM_ORDER_PARTS[@]}"; do
  case "${requested_arm,,}" in
    apxm|apxm-on|priority-on)
      run_cell "APXM-on" "PRIORITY-ON" "priority.apxm-on.csv"
      ;;
    flat|flat-http|priority-fh)
      run_cell "flat-HTTP" "PRIORITY-FH" "priority.flat-http.csv" --no-apxm-hints
      ;;
    "")
      ;;
    *)
      echo "[priority-lane] ERROR: unsupported ARM_ORDER entry: $requested_arm" >&2
      exit 2
      ;;
  esac
done

if [ ! -f "$OUT_DIR/priority.apxm-on.tenants.csv" ] || [ ! -f "$OUT_DIR/priority.flat-http.tenants.csv" ]; then
  echo "[priority-lane] ERROR: ARM_ORDER must include both apxm and flat" >&2
  exit 2
fi

if python3 examples/python/benchmarks/apxm_priority_lane_report.py \
  --apxm-tenants "$OUT_DIR/priority.apxm-on.tenants.csv" \
  --flat-tenants "$OUT_DIR/priority.flat-http.tenants.csv" \
  --output "$OUT_DIR/report.json" \
  --summary-csv "$OUT_DIR/summary.csv" \
  > "$OUT_DIR/report.stdout.json"; then
  echo "[priority-lane] promotion gates: pass"
else
  echo "[priority-lane] promotion gates: not passed (see $OUT_DIR/report.json)"
fi

echo "[priority-lane] DONE - $OUT_DIR"
