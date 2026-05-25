#!/usr/bin/env bash
# Run the APXM Review Council dogfood evaluation as a paired APXM-on vs
# flat-HTTP workflow against an existing Dekk-managed vLLM service.
#
# Generated evidence is written under:
#   .apxm/evaluation/apxm-review-council/runs/<UTC>/
#
# Required by default:
#   dekk apxm vllm zoo-apply
#   dekk apxm vllm service-status vllm-gptoss --probe
#
# Optional env:
#   SERVICE (vllm-gptoss), APXM_ENDPOINT (http://127.0.0.1:8916), ITER (10),
#   CONC (16), PREFIX_TOK (8192), FANOUT (6), MODEL (gpt-oss-120b),
#   REVIEW_MAX_TOKENS (256), VERDICT_MAX_TOKENS (512),
#   PRE_REG (docs/preregistrations/20260521T001540Z-apxm-review-council.md),
#   TS, SMOKE (0), MATRIX_REPORT (1), REQUIRE_PRE_REG (1)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

mapfile -t LAYOUT_LINES < <(
  PYTHONPATH="$REPO_ROOT/tools/scripts${PYTHONPATH:+:$PYTHONPATH}" \
    python3 - "$REPO_ROOT" <<'PY'
import sys
from pathlib import Path
from apxm_vllm_contract import build_layout

layout = build_layout(Path(sys.argv[1]) / "tools/scripts/run_apxm_review_council.sh")
print(layout.evaluation_dir / "apxm-review-council")
print(layout.service_dir)
PY
)

: "${SERVICE:=vllm-gptoss}"
: "${APXM_ENDPOINT:=http://127.0.0.1:8916}"
: "${ITER:=10}"
: "${CONC:=16}"
PREFIX_TOK_WAS_SET="${PREFIX_TOK+x}"
FANOUT_WAS_SET="${FANOUT+x}"
: "${PREFIX_TOK:=8192}"
: "${FANOUT:=6}"
: "${REVIEW_MAX_TOKENS:=256}"
: "${VERDICT_MAX_TOKENS:=512}"
: "${MODEL:=gpt-oss-120b}"
: "${PRE_REG:=docs/preregistrations/20260521T001540Z-apxm-review-council.md}"
: "${TS:=$(date -u +%Y%m%dT%H%M%SZ)}"
: "${SMOKE:=0}"
: "${MATRIX_REPORT:=1}"
: "${REQUIRE_PRE_REG:=1}"
: "${WORKLOAD_APXM_CONFIG:=$HOME/.apxm/config.toml}"

if [[ "${SMOKE,,}" =~ ^(1|true|yes|on)$ ]]; then
  ITER=1
  CONC=1
  if [ -z "$PREFIX_TOK_WAS_SET" ]; then
    PREFIX_TOK=1024
  fi
  if [ -z "$FANOUT_WAS_SET" ]; then
    FANOUT=2
  fi
fi

RUN_ROOT="${LAYOUT_LINES[0]}"
SERVICE_DIR="${LAYOUT_LINES[1]}"
OUT_DIR="${RUN_ROOT}/runs/${TS}"
mkdir -p "$OUT_DIR"

echo "[review-council] run dir: $OUT_DIR"
echo "[review-council] service=$SERVICE endpoint=$APXM_ENDPOINT model=$MODEL"
echo "[review-council] iter=$ITER conc=$CONC prefix_tok=$PREFIX_TOK fanout=$FANOUT smoke=$SMOKE"
echo "[review-council] review_max_tokens=$REVIEW_MAX_TOKENS verdict_max_tokens=$VERDICT_MAX_TOKENS"

if [ ! -f "$PRE_REG" ]; then
  echo "[review-council] ERROR: pre-registration not found: $PRE_REG" >&2
  exit 2
fi

SERVICE_STATE="${SERVICE_DIR}/${SERVICE}.json"
dekk apxm vllm service-status "$SERVICE" --probe > "$OUT_DIR/service-probe.txt"
cp "$SERVICE_STATE" "$OUT_DIR/service-state.json"

COMMON_FLAGS=(
  --graph examples/python/benchmarks/workloads/apxm_review_council.py
  --apxm-endpoint "$APXM_ENDPOINT"
  --concurrency "$CONC"
  --iterations "$ITER"
  --model "$MODEL"
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

  local report_flags=()
  if [[ "${MATRIX_REPORT,,}" =~ ^(1|true|yes|on)$ ]]; then
    report_flags+=(--matrix-report "$OUT_DIR/${output%.csv}.matrix.json")
  fi

  echo "[review-council] === $label ==="
  dekk apxm vllm service-exec "$SERVICE" -- \
    env APXM_CONFIG="$WORKLOAD_APXM_CONFIG" \
    APXM_WORKLOAD_PREFIX_TOK="$PREFIX_TOK" APXM_WORKLOAD_FANOUT="$FANOUT" \
    APXM_WORKLOAD_REVIEW_MAX_TOKENS="$REVIEW_MAX_TOKENS" \
    APXM_WORKLOAD_VERDICT_MAX_TOKENS="$VERDICT_MAX_TOKENS" \
    python3 examples/python/benchmarks/concurrent_matrix.py \
      "${COMMON_FLAGS[@]}" \
      --opt-levels 2 \
      --cell-label "$arm" \
      --output "$OUT_DIR/$output" \
      "${report_flags[@]}" \
      "$@" \
    > "$OUT_DIR/${output}.log" 2>&1
  tail -120 "$OUT_DIR/${output}.log"
}

run_cell "APXM-on" "REVIEW-ON" "review.apxm-on.csv"
run_cell "flat-HTTP" "REVIEW-FH" "review.flat-http.csv" --no-apxm-hints

python3 examples/python/benchmarks/plan04_cross_workload.py \
  --input "apxm-review-council:$OUT_DIR/review.apxm-on.csv" \
  --input "apxm-review-council:$OUT_DIR/review.flat-http.csv" \
  --output "$OUT_DIR/combined.csv" \
  --summary "$OUT_DIR/summary.csv" \
  --manifest "$OUT_DIR/summary.json"

if python3 tools/scripts/assess_apxm_review_council_quality.py "$OUT_DIR" \
  > "$OUT_DIR/quality.json"; then
  echo "[review-council] quality gate: pass"
else
  echo "[review-council] quality gate: fail (see $OUT_DIR/quality.json)"
fi

echo "[review-council] DONE - $OUT_DIR"
