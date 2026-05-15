#!/usr/bin/env bash
# Paired-arm benchmark runner. Internal tool; not part of the public CLI.
# Args: EVAL_DIR CONCURRENCY WORKLOAD_PATH [PREFIX_TOK=4096] [FANOUT=8] [ITERATIONS=5]
set -euo pipefail
REPO_ROOT=/home/apxm/projects/apxm
EVAL_DIR=${1:?eval_dir}
CONCURRENCY=${2:?concurrency}
WORKLOAD=${3:?workload_path}
PREFIX_TOK=${4:-4096}
FANOUT=${5:-8}
ITERATIONS=${6:-5}
mkdir -p "$EVAL_DIR"
cd "$REPO_ROOT"
OPTS="0 2"
echo "[paired] WORKLOAD=$(basename $WORKLOAD) c=$CONCURRENCY prefix=$PREFIX_TOK fanout=$FANOUT iters=$ITERATIONS"
echo "[paired] APXM-on arm"
APXM_WORKLOAD_PREFIX_TOK=$PREFIX_TOK \
APXM_WORKLOAD_FANOUT=$FANOUT \
python3 examples/python/benchmarks/phase_g_concurrent.py \
  --concurrency "$CONCURRENCY" --iterations "$ITERATIONS" --opt-levels $OPTS \
  --graph "$WORKLOAD" --output "$EVAL_DIR/apxm.csv" \
  --metrics-url http://127.0.0.1:8916/metrics
echo "[paired] flat-HTTP arm"
APXM_WORKLOAD_PREFIX_TOK=$PREFIX_TOK \
APXM_WORKLOAD_FANOUT=$FANOUT \
python3 examples/python/benchmarks/phase_g_concurrent.py \
  --concurrency "$CONCURRENCY" --iterations "$ITERATIONS" --opt-levels $OPTS \
  --graph "$WORKLOAD" --no-apxm-hints --output "$EVAL_DIR/flat.csv" \
  --metrics-url http://127.0.0.1:8916/metrics
cat "$EVAL_DIR/apxm.csv" > "$EVAL_DIR/paired.csv"
tail -n +2 "$EVAL_DIR/flat.csv" >> "$EVAL_DIR/paired.csv"
python3 examples/python/benchmarks/comparison_report.py \
  "$EVAL_DIR/paired.csv" --markdown-out "$EVAL_DIR/report.md"
echo "[paired] done"
