#!/usr/bin/env bash
#
# APXM-vLLM Slurm wrapper (single-node).
#
# Each Slurm allocation hosts one APXM-vLLM container that serves one
# model. Multi-instance throughput is achieved by submitting multiple
# allocations (one per replica) via `dekk agents vllm zoo-apply`.
#
# Multi-node distributed inference (Ray TP+PP across nodes) is
# intentionally out of scope: the per-deployment shape is constrained to
# what fits on a single 8x MI300X node. If a model truly exceeds one
# node, register it as a separate backend protocol or shrink the
# `tensor_parallel` / `max_model_len` until it fits.
#
# Usage:
#   sbatch deploy/vllm/run-vllm.sh
#
# SBATCH directives are emitted as `sbatch` arguments from
# `_start_one_service` (--job-name, --output). Defaults below cover the
# per-rank resource shape; the caller may override --time, --partition,
# etc. on the sbatch line.

#SBATCH --cpus-per-task=112
#SBATCH --mem=0
#SBATCH --time=04:00:00

set -euo pipefail

MODEL_REF="${MODEL_REF:?MODEL_REF is required (set via manifest)}"
SERVED_MODEL_ID="${SERVED_MODEL_ID:?SERVED_MODEL_ID is required (set via manifest served_model_name)}"
BACKEND_NAME="${BACKEND_NAME:-vllm-fork}"
PORT="${PORT:?PORT is required (no controller default)}"
HF_HOME_HOST="${HF_HOME_HOST:?HF_HOME_HOST is required (resolved by the controller)}"
APXM_VLLM_IMAGE="${APXM_VLLM_IMAGE:?APXM_VLLM_IMAGE is required (set via manifest or APXM_VLLM_IMAGE env)}"
APXM_VLLM_IMAGE_ARCHIVE="${APXM_VLLM_IMAGE_ARCHIVE:-}"
TENSOR_PARALLEL_SIZE="${TENSOR_PARALLEL_SIZE:-8}"
MAX_MODEL_LEN="${MAX_MODEL_LEN:-32768}"
GPU_MEMORY_UTILIZATION="${GPU_MEMORY_UTILIZATION:-0.90}"
MAX_NUM_SEQS="${MAX_NUM_SEQS:-64}"
SCHEDULING_POLICY="${SCHEDULING_POLICY:-priority}"
ENABLE_PREFIX_CACHING="${ENABLE_PREFIX_CACHING:-1}"
STARTUP_TIMEOUT_SECONDS="${STARTUP_TIMEOUT_SECONDS:-7200}"
# Model-specific feature toggles: no shell defaults. Applying a
# non-matching reasoning parser crashes at vLLM startup with a vocab
# KeyError. The manifest must set these per [[deployment]] (zoo.toml
# field names: reasoning_parser, tool_call_parser,
# enable_auto_tool_choice).
REASONING_PARSER="${REASONING_PARSER:-}"
TOOL_CALL_PARSER="${TOOL_CALL_PARSER:-}"
ENABLE_AUTO_TOOL_CHOICE="${ENABLE_AUTO_TOOL_CHOICE:-}"
GPUS="${GPUS:-}"
APXM_VLLM_SERVICE_NAME="${APXM_VLLM_SERVICE_NAME:-$SLURM_JOB_NAME}"
CONTAINER_NAME="${CONTAINER_NAME:-apxm-vllm-${SLURM_JOB_ID:-manual}-${PORT}}"

cleanup() {
  # Deregister the APXM backend first so subsequent zoo-apply runs do not
  # see a stale registration pointing at a dead endpoint. Best-effort —
  # never let a deregister failure mask the container teardown.
  dekk agents backend remove "$BACKEND_NAME" >/dev/null 2>&1 || true
  docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

echo "host=$(hostname)"
echo "slurm_job_id=${SLURM_JOB_ID:-}"
echo "slurm_job_nodelist=${SLURM_JOB_NODELIST:-}"
echo "image=${APXM_VLLM_IMAGE}"
echo "image_archive=${APXM_VLLM_IMAGE_ARCHIVE:-<apxm image store default>}"
echo "model=${MODEL_REF}"
echo "served_model=${SERVED_MODEL_ID}"
echo "port=${PORT}"
echo "hf_home=${HF_HOME_HOST}"
echo "model_roots=${APXM_VLLM_MODEL_ROOTS:-}"
echo "tensor_parallel_size=${TENSOR_PARALLEL_SIZE}"
echo "max_num_seqs=${MAX_NUM_SEQS}"
echo "scheduling_policy=${SCHEDULING_POLICY}"
echo "enable_prefix_caching=${ENABLE_PREFIX_CACHING}"
echo "container_name=${CONTAINER_NAME}"

mkdir -p "$HF_HOME_HOST"

load_cmd=(dekk agents vllm docker-load --image "$APXM_VLLM_IMAGE")
if [ -n "$APXM_VLLM_IMAGE_ARCHIVE" ]; then
  load_cmd+=(--archive "$APXM_VLLM_IMAGE_ARCHIVE")
fi
"${load_cmd[@]}"

cmd=(
  dekk agents vllm docker-start "$MODEL_REF"
  --image "$APXM_VLLM_IMAGE"
  --served-model-name "$SERVED_MODEL_ID"
  --backend-name "$BACKEND_NAME"
  --container-name "$CONTAINER_NAME"
  --hf-home "$HF_HOME_HOST"
  --port "$PORT"
  --tensor-parallel-size "$TENSOR_PARALLEL_SIZE"
  --gpu-memory-utilization "$GPU_MEMORY_UTILIZATION"
  --max-model-len "$MAX_MODEL_LEN"
  --enable-prompt-tokens-details
  --enable-force-include-usage
  --enable
  --startup-timeout "$STARTUP_TIMEOUT_SECONDS"
  --max-num-seqs "$MAX_NUM_SEQS"
)
if [ -n "$GPUS" ]; then
  cmd+=(--gpus "$GPUS")
fi
if [ "$ENABLE_PREFIX_CACHING" = "1" ]; then
  cmd+=(--enable-prefix-caching)
fi
if [ -n "$SCHEDULING_POLICY" ]; then
  cmd+=(--scheduling-policy "$SCHEDULING_POLICY")
fi
if [ -n "$REASONING_PARSER" ]; then
  cmd+=(--reasoning-parser "$REASONING_PARSER")
fi
if [ -n "$TOOL_CALL_PARSER" ]; then
  cmd+=(--tool-call-parser "$TOOL_CALL_PARSER")
fi
if [ "$ENABLE_AUTO_TOOL_CHOICE" = "1" ]; then
  cmd+=(--enable-auto-tool-choice)
fi

"${cmd[@]}"

echo "vLLM is ready at http://127.0.0.1:${PORT}/v1"
echo "Press Ctrl-C or cancel the Slurm job to stop the container."
sleep infinity
