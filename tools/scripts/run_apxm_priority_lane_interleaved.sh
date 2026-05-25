#!/usr/bin/env bash
# Run APXM priority-lane evaluation interleaved by batch.
#
# Generated evidence is written under:
#   .apxm/evaluation/apxm-priority-lane/runs/<UTC>/
#
# Optional env:
#   SERVICE (vllm-gptoss), APXM_ENDPOINT (http://127.0.0.1:8916), ITER (5),
#   CONC (16), PREFIX_TOK (1024), BG_FANOUT (16), MODEL (gpt-oss-120b),
#   CRITICAL_MAX_TOKENS (64), BACKGROUND_MAX_TOKENS (256), FOCUS_NODE_ID (4),
#   PRE_REG, TS, SMOKE (0), REQUIRE_PRE_REG (1), STAGGER_MS (0),
#   FIRST_ARM (flat), ALTERNATE_ORDER (1)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

mapfile -t LAYOUT_LINES < <(
  PYTHONPATH="$REPO_ROOT/tools/scripts${PYTHONPATH:+:$PYTHONPATH}" \
    python3 - "$REPO_ROOT" <<'PY'
import sys
from pathlib import Path
from apxm_vllm_contract import build_layout

layout = build_layout(Path(sys.argv[1]) / "tools/scripts/run_apxm_priority_lane_interleaved.sh")
print(layout.evaluation_dir / "apxm-priority-lane")
print(layout.service_dir)
PY
)

: "${SERVICE:=vllm-gptoss}"
: "${APXM_ENDPOINT:=http://127.0.0.1:8916}"
: "${ITER:=5}"
: "${CONC:=16}"
: "${PREFIX_TOK:=1024}"
: "${BG_FANOUT:=16}"
: "${CRITICAL_MAX_TOKENS:=64}"
: "${BACKGROUND_MAX_TOKENS:=256}"
: "${FOCUS_NODE_ID:=4}"
: "${MODEL:=gpt-oss-120b}"
: "${PRE_REG:=docs/preregistrations/20260521T123000Z-apxm-priority-lane-c16-bg16-interleaved.md}"
: "${TS:=$(date -u +%Y%m%dT%H%M%SZ)-interleaved}"
: "${SMOKE:=0}"
: "${REQUIRE_PRE_REG:=1}"
: "${STAGGER_MS:=0}"
: "${FIRST_ARM:=flat}"
: "${ALTERNATE_ORDER:=1}"
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
TMP_DIR="$OUT_DIR/interleaved-parts"
mkdir -p "$TMP_DIR"

echo "[priority-lane-interleaved] run dir: $OUT_DIR"
echo "[priority-lane-interleaved] service=$SERVICE endpoint=$APXM_ENDPOINT model=$MODEL"
echo "[priority-lane-interleaved] iter=$ITER conc=$CONC prefix_tok=$PREFIX_TOK bg_fanout=$BG_FANOUT stagger_ms=$STAGGER_MS smoke=$SMOKE"
echo "[priority-lane-interleaved] critical_max_tokens=$CRITICAL_MAX_TOKENS background_max_tokens=$BACKGROUND_MAX_TOKENS focus_node_id=$FOCUS_NODE_ID"
echo "[priority-lane-interleaved] first_arm=$FIRST_ARM alternate_order=$ALTERNATE_ORDER"

if [ ! -f "$PRE_REG" ]; then
  echo "[priority-lane-interleaved] ERROR: pre-registration not found: $PRE_REG" >&2
  exit 2
fi

SERVICE_STATE="${SERVICE_DIR}/${SERVICE}.json"
dekk apxm vllm service-status "$SERVICE" --probe > "$OUT_DIR/service-probe.txt"
cp "$SERVICE_STATE" "$OUT_DIR/service-state.json"

COMMON_FLAGS=(
  --graph examples/python/benchmarks/workloads/apxm_priority_lane.py
  --apxm-endpoint "$APXM_ENDPOINT"
  --concurrency "$CONC"
  --iterations 1
  --stagger-ms "$STAGGER_MS"
  --model "$MODEL"
  --focus-node-id "$FOCUS_NODE_ID"
  --pre-registration "$PRE_REG"
)

if [[ "${REQUIRE_PRE_REG,,}" =~ ^(1|true|yes|on)$ ]]; then
  COMMON_FLAGS+=(--require-pre-registration)
fi

run_one_batch() {
  local iteration="$1"
  local arm="$2"

  local label cell output extra_flags=()
  case "${arm,,}" in
    apxm|apxm-on|priority-on)
      label="APXM-on"
      cell="PRIORITY-ON"
      output="$TMP_DIR/apxm.iter${iteration}.csv"
      ;;
    flat|flat-http|priority-fh)
      label="flat-HTTP"
      cell="PRIORITY-FH"
      output="$TMP_DIR/flat.iter${iteration}.csv"
      extra_flags+=(--no-apxm-hints)
      ;;
    *)
      echo "[priority-lane-interleaved] ERROR: unsupported arm: $arm" >&2
      exit 2
      ;;
  esac

  echo "[priority-lane-interleaved] === iter=$iteration $label ==="
  dekk apxm vllm service-exec "$SERVICE" -- \
    env APXM_CONFIG="$WORKLOAD_APXM_CONFIG" \
    APXM_WORKLOAD_PREFIX_TOK="$PREFIX_TOK" \
    APXM_WORKLOAD_FANOUT="$BG_FANOUT" \
    APXM_PRIORITY_CRITICAL_MAX_TOKENS="$CRITICAL_MAX_TOKENS" \
    APXM_PRIORITY_BACKGROUND_MAX_TOKENS="$BACKGROUND_MAX_TOKENS" \
    python3 examples/python/benchmarks/concurrent_matrix.py \
      "${COMMON_FLAGS[@]}" \
      --opt-levels 2 \
      --cell-label "$cell" \
      --output "$output" \
      "${extra_flags[@]}" \
    > "$OUT_DIR/${cell}.iter${iteration}.log" 2>&1
  tail -80 "$OUT_DIR/${cell}.iter${iteration}.log"
}

other_arm() {
  case "${1,,}" in
    apxm|apxm-on|priority-on) echo "flat" ;;
    flat|flat-http|priority-fh) echo "apxm" ;;
    *)
      echo "[priority-lane-interleaved] ERROR: unsupported FIRST_ARM: $1" >&2
      exit 2
      ;;
  esac
}

FIRST_ARM_NORMALIZED="${FIRST_ARM,,}"
SECOND_ARM="$(other_arm "$FIRST_ARM_NORMALIZED")"

for iteration in $(seq 1 "$ITER"); do
  first="$FIRST_ARM_NORMALIZED"
  second="$SECOND_ARM"
  if [[ "${ALTERNATE_ORDER,,}" =~ ^(1|true|yes|on)$ ]] && (( iteration % 2 == 0 )); then
    first="$SECOND_ARM"
    second="$FIRST_ARM_NORMALIZED"
  fi
  run_one_batch "$iteration" "$first"
  run_one_batch "$iteration" "$second"
done

python3 - "$TMP_DIR" "$OUT_DIR" "$ITER" <<'PY'
import csv
import sys
from pathlib import Path

tmp = Path(sys.argv[1])
out = Path(sys.argv[2])
iterations = int(sys.argv[3])


def stitch(stem: str, suffix: str, output_name: str) -> None:
    rows = []
    fieldnames = None
    for iteration in range(1, iterations + 1):
        path = tmp / f"{stem}.iter{iteration}{suffix}"
        if not path.exists():
            raise SystemExit(f"missing expected part: {path}")
        with path.open(newline="") as fh:
            reader = csv.DictReader(fh)
            if fieldnames is None:
                fieldnames = reader.fieldnames
            elif reader.fieldnames != fieldnames:
                raise SystemExit(f"field mismatch in {path}")
            for row in reader:
                row["iteration"] = str(iteration)
                rows.append(row)

    if fieldnames is None:
        raise SystemExit(f"no rows for {stem}{suffix}")
    output = out / output_name
    with output.open("w", newline="") as fh:
        writer = csv.DictWriter(fh, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)


stitch("apxm", ".csv", "priority.apxm-on.csv")
stitch("apxm", ".tenants.csv", "priority.apxm-on.tenants.csv")
stitch("flat", ".csv", "priority.flat-http.csv")
stitch("flat", ".tenants.csv", "priority.flat-http.tenants.csv")
PY

if python3 examples/python/benchmarks/apxm_priority_lane_report.py \
  --apxm-tenants "$OUT_DIR/priority.apxm-on.tenants.csv" \
  --flat-tenants "$OUT_DIR/priority.flat-http.tenants.csv" \
  --output "$OUT_DIR/report.json" \
  --summary-csv "$OUT_DIR/summary.csv" \
  > "$OUT_DIR/report.stdout.json"; then
  echo "[priority-lane-interleaved] promotion gates: pass"
else
  echo "[priority-lane-interleaved] promotion gates: not passed (see $OUT_DIR/report.json)"
fi

echo "[priority-lane-interleaved] DONE - $OUT_DIR"
