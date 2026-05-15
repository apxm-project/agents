#!/usr/bin/env bash
#
# Slurm-owned APXM-vLLM launcher.
#
# Usage:
#   sbatch deploy/vllm/run-vllm-slurm.sh
#   srun ... --pty deploy/vllm/run-vllm-slurm.sh
#
# Optional command arguments are executed after vLLM is ready, while the trap
# keeps container cleanup tied to the Slurm job lifecycle.

#SBATCH -N 1
#SBATCH --cpus-per-task=112
#SBATCH --mem=0
#SBATCH --time=04:00:00

set -euo pipefail

MODEL_REF="${MODEL_REF:-openai/gpt-oss-120b}"
SERVED_MODEL_ID="${SERVED_MODEL_ID:-gpt-oss-120b}"
BACKEND_NAME="${BACKEND_NAME:-vllm-fork}"
PORT="${PORT:-8916}"
HF_HOME_HOST="${HF_HOME_HOST:-${APXM_VLLM_HF_HOME:-$HOME/.cache/huggingface-apxm-vllm}}"
APXM_COMMIT="${APXM_COMMIT:-$(git rev-parse --short HEAD)}"
VLLM_COMMIT="${VLLM_COMMIT:-$(git -C external/vllm rev-parse --short HEAD)}"
APXM_VLLM_IMAGE="${APXM_VLLM_IMAGE:-apxm-vllm-runtime:${APXM_COMMIT}-${VLLM_COMMIT}}"
APXM_VLLM_IMAGE_ARCHIVE="${APXM_VLLM_IMAGE_ARCHIVE:-}"
TENSOR_PARALLEL_SIZE="${TENSOR_PARALLEL_SIZE:-8}"
MAX_MODEL_LEN="${MAX_MODEL_LEN:-32768}"
GPU_MEMORY_UTILIZATION="${GPU_MEMORY_UTILIZATION:-0.90}"
MAX_NUM_SEQS="${MAX_NUM_SEQS:-64}"
SCHEDULING_POLICY="${SCHEDULING_POLICY:-priority}"
ENABLE_PREFIX_CACHING="${ENABLE_PREFIX_CACHING:-1}"
STARTUP_TIMEOUT_SECONDS="${STARTUP_TIMEOUT_SECONDS:-7200}"
REASONING_PARSER="${REASONING_PARSER:-openai_gptoss}"
TOOL_CALL_PARSER="${TOOL_CALL_PARSER:-openai}"
ENABLE_AUTO_TOOL_CHOICE="${ENABLE_AUTO_TOOL_CHOICE:-1}"
CONTAINER_NAME="${CONTAINER_NAME:-apxm-vllm-${SLURM_JOB_ID:-manual}-${PORT}}"

cleanup() {
  dekk apxm vllm docker-stop --port "$PORT" --container-name "$CONTAINER_NAME" >/dev/null 2>&1 || true
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
echo "max_num_seqs=${MAX_NUM_SEQS}"
echo "scheduling_policy=${SCHEDULING_POLICY}"
echo "enable_prefix_caching=${ENABLE_PREFIX_CACHING}"
echo "container_name=${CONTAINER_NAME}"

mkdir -p "$HF_HOME_HOST"

load_cmd=(dekk apxm vllm docker-load --image "$APXM_VLLM_IMAGE")
if [ -n "$APXM_VLLM_IMAGE_ARCHIVE" ]; then
  load_cmd+=(--archive "$APXM_VLLM_IMAGE_ARCHIVE")
fi
"${load_cmd[@]}"

cmd=(
  dekk apxm vllm docker-start "$MODEL_REF"
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
  --alias benchmark
  --startup-timeout "$STARTUP_TIMEOUT_SECONDS"
  --max-num-seqs "$MAX_NUM_SEQS"
)

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

if [ "$#" -gt 0 ]; then
  "$@"
else
  echo "vLLM is ready at http://127.0.0.1:${PORT}/v1"
  echo "Press Ctrl-C or cancel the Slurm job to stop the container."
  sleep infinity
fi
