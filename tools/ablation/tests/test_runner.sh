#!/usr/bin/env bash
# Smoke test for the ablation harness.
# Verifies that `python3 -m ablation` runs end-to-end and emits the delta
# table header. Does NOT assert on exit code: the harness is expected to
# exit 1 whenever it finds a dead pass or a regression, and that signal is
# the whole point of the tool. This smoke test only proves the runner
# wires up to `dekk apxm compile` and produces parseable output.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
TOOLS_DIR="$(cd "$HERE/../.." && pwd)"

OUT="$(mktemp -t ablation_smoke.XXXXXX.md)"
trap 'rm -f "$OUT"' EXIT

cd "$TOOLS_DIR"
# `|| true` because exit 1 == dead pass / regression found, which is a
# valid harness outcome and must not fail the smoke test.
python3 -m ablation > "$OUT" || true

grep -q "^# Ablation delta table" "$OUT" \
    || { echo "FAIL: missing 'Ablation delta table' header"; cat "$OUT"; exit 1; }
grep -q "^| pass disabled | graph |" "$OUT" \
    || { echo "FAIL: missing table header row"; cat "$OUT"; exit 1; }

echo "PASS: ablation harness produced delta table"
