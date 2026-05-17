#!/usr/bin/env bash
#
# Unified APXM-vLLM Slurm wrapper.
#
# NODES=1 (default): single-node single-instance. The unified `docker-start`
# path creates the container and launches vLLM in one shot.
#
# NODES>1: multi-node single-instance via Ray. Every rank must host a Ray
# process inside a container *before* vLLM is launched on rank 0, because
# vLLM's TP+PP startup expects to discover a complete Ray cluster on
# admission. The wrapper therefore separates Ray-container startup from
# vLLM startup and uses `docker exec` to launch the API server on rank 0
# only after the worker pool is healthy.
#
# Usage:
#   sbatch deploy/vllm/run-vllm.sh
#
# SBATCH directives required for either mode are emitted as `sbatch`
# arguments from `_start_one_service` (-N, --ntasks-per-node, --exclusive).
# Defaults below cover the per-rank resource shape; the caller may override
# --time, --partition, etc. on the sbatch line.

#SBATCH --cpus-per-task=112
#SBATCH --mem=0
#SBATCH --time=04:00:00

set -euo pipefail

MODEL_REF="${MODEL_REF:-openai/gpt-oss-120b}"
SERVED_MODEL_ID="${SERVED_MODEL_ID:-gpt-oss-120b}"
BACKEND_NAME="${BACKEND_NAME:-vllm-fork}"
PORT="${PORT:?PORT is required (no controller default)}"
NODES="${NODES:-1}"
HF_HOME_HOST="${HF_HOME_HOST:?HF_HOME_HOST is required (set APXM_VLLM_HF_HOME via the controller)}"
APXM_COMMIT="${APXM_COMMIT:-$(git rev-parse --short HEAD)}"
VLLM_COMMIT="${VLLM_COMMIT:-$(git -C external/vllm rev-parse --short HEAD)}"
APXM_VLLM_IMAGE="${APXM_VLLM_IMAGE:?APXM_VLLM_IMAGE is required (set via manifest or APXM_VLLM_IMAGE env)}"
APXM_VLLM_IMAGE_ARCHIVE="${APXM_VLLM_IMAGE_ARCHIVE:-}"
TENSOR_PARALLEL_SIZE="${TENSOR_PARALLEL_SIZE:-8}"
PIPELINE_PARALLEL_SIZE="${PIPELINE_PARALLEL_SIZE:-1}"
MAX_MODEL_LEN="${MAX_MODEL_LEN:-32768}"
GPU_MEMORY_UTILIZATION="${GPU_MEMORY_UTILIZATION:-0.90}"
MAX_NUM_SEQS="${MAX_NUM_SEQS:-64}"
SCHEDULING_POLICY="${SCHEDULING_POLICY:-priority}"
ENABLE_PREFIX_CACHING="${ENABLE_PREFIX_CACHING:-1}"
STARTUP_TIMEOUT_SECONDS="${STARTUP_TIMEOUT_SECONDS:-7200}"
REASONING_PARSER="${REASONING_PARSER:-openai_gptoss}"
TOOL_CALL_PARSER="${TOOL_CALL_PARSER:-openai}"
ENABLE_AUTO_TOOL_CHOICE="${ENABLE_AUTO_TOOL_CHOICE:-1}"
GPUS="${GPUS:-}"
RAY_PORT="${RAY_PORT:-6379}"
RAY_READY_TIMEOUT="${RAY_READY_TIMEOUT:-600}"
APXM_VLLM_SERVICE_NAME="${APXM_VLLM_SERVICE_NAME:-$SLURM_JOB_NAME}"
CONTAINER_NAME="${CONTAINER_NAME:-apxm-vllm-${SLURM_JOB_ID:-manual}-rank${SLURM_PROCID:-0}-${PORT}}"

cleanup() {
  # Deregister the APXM backend first so subsequent zoo-apply runs do not
  # see a stale registration pointing at a dead endpoint. Best-effort —
  # never let a deregister failure mask the container teardown.
  dekk apxm backend remove "$BACKEND_NAME" >/dev/null 2>&1 || true
  docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

echo "host=$(hostname)"
echo "slurm_job_id=${SLURM_JOB_ID:-}"
echo "slurm_job_nodelist=${SLURM_JOB_NODELIST:-}"
echo "slurm_procid=${SLURM_PROCID:-0}"
echo "image=${APXM_VLLM_IMAGE}"
echo "image_archive=${APXM_VLLM_IMAGE_ARCHIVE:-<apxm image store default>}"
echo "model=${MODEL_REF}"
echo "served_model=${SERVED_MODEL_ID}"
echo "port=${PORT}"
echo "nodes=${NODES}"
echo "tensor_parallel_size=${TENSOR_PARALLEL_SIZE}"
echo "pipeline_parallel_size=${PIPELINE_PARALLEL_SIZE}"
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

# ---- helpers ---------------------------------------------------------------

# Build the dekk apxm vllm docker-start command (used by the single-node
# path and the multi-node rank-0 API launch via docker exec).
build_vllm_cmd() {
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
  if [ -n "$GPUS" ]; then
    cmd+=(--gpus "$GPUS")
  fi
  if [ "$PIPELINE_PARALLEL_SIZE" -gt 1 ]; then
    cmd+=(--pipeline-parallel-size "$PIPELINE_PARALLEL_SIZE")
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
}

# Start a long-lived container that runs Ray (head or worker) and idles.
# No vLLM, no port binding (Ray's own ports are exposed via host network).
# Used for every rank in the multi-node path.
start_ray_container() {
  local role="$1"          # "head" or "worker"
  local head_addr="$2"     # e.g. node-12:6379, ignored when role=head
  local ray_cmd
  if [ "$role" = "head" ]; then
    ray_cmd="ray start --head --port=${RAY_PORT} --num-cpus=$(nproc) && tail -f /dev/null"
  else
    ray_cmd="ray start --address=${head_addr} --num-cpus=$(nproc) && tail -f /dev/null"
  fi
  docker run -d \
    --name "$CONTAINER_NAME" \
    --network host \
    --ipc=host \
    --device /dev/kfd --device /dev/dri \
    --group-add video --group-add render \
    --security-opt seccomp=unconfined \
    -e "HF_HOME=/models/hf" \
    -v "${HF_HOME_HOST}:/models/hf" \
    ${GPUS:+-e "HIP_VISIBLE_DEVICES=${GPUS}"} \
    ${GPUS:+-e "CUDA_VISIBLE_DEVICES=${GPUS}"} \
    --entrypoint bash \
    "$APXM_VLLM_IMAGE" \
    -c "$ray_cmd"
}

# Block until `ray status` inside CONTAINER_NAME reports the expected
# number of nodes (rank 0 only). Times out after RAY_READY_TIMEOUT seconds.
wait_for_ray_cluster() {
  local expected="$1"
  local deadline=$((SECONDS + RAY_READY_TIMEOUT))
  while [ "$SECONDS" -lt "$deadline" ]; do
    if docker exec "$CONTAINER_NAME" ray status 2>/dev/null \
        | awk '/^Active:/{n++} END{exit !(n>0)}'; then
      local node_count
      node_count=$(docker exec "$CONTAINER_NAME" ray status 2>/dev/null \
        | awk '/node_/{n++} END{print n+0}')
      if [ "$node_count" -ge "$expected" ]; then
        echo "ray cluster ready: ${node_count}/${expected} nodes"
        return 0
      fi
      echo "ray cluster forming: ${node_count}/${expected} nodes"
    fi
    sleep 5
  done
  echo "ray cluster never reached ${expected} nodes within ${RAY_READY_TIMEOUT}s" >&2
  return 1
}

# ---- single-node path -----------------------------------------------------

if [ "$NODES" -le 1 ]; then
  build_vllm_cmd
  "${cmd[@]}"
else
  # ---- multi-node Ray path ------------------------------------------------
  HEAD_NODE=$(scontrol show hostnames "$SLURM_JOB_NODELIST" | head -1)
  HEAD_ADDR="${HEAD_NODE}:${RAY_PORT}"
  echo "ray_head_node=${HEAD_NODE}"
  echo "ray_port=${RAY_PORT}"

  if [ "${SLURM_PROCID:-0}" = "0" ]; then
    # Rank 0: start the Ray-head container, wait until every worker has
    # attached, then launch vLLM via docker exec inside the same container.
    start_ray_container "head" ""
    wait_for_ray_cluster "$NODES"

    docker exec "$CONTAINER_NAME" bash -c "
      python -m vllm.entrypoints.openai.api_server \\
        --model '${MODEL_REF}' \\
        --served-model-name '${SERVED_MODEL_ID}' \\
        --host 127.0.0.1 \\
        --port ${PORT} \\
        --tensor-parallel-size ${TENSOR_PARALLEL_SIZE} \\
        --pipeline-parallel-size ${PIPELINE_PARALLEL_SIZE} \\
        --gpu-memory-utilization ${GPU_MEMORY_UTILIZATION} \\
        --max-model-len ${MAX_MODEL_LEN} \\
        --max-num-seqs ${MAX_NUM_SEQS} \\
        --scheduling-policy ${SCHEDULING_POLICY} \\
        $([ "${ENABLE_PREFIX_CACHING}" = "1" ] && echo --enable-prefix-caching) \\
        $([ "${ENABLE_AUTO_TOOL_CHOICE}" = "1" ] && echo --enable-auto-tool-choice) \\
        $([ -n "${REASONING_PARSER}" ] && echo --reasoning-parser ${REASONING_PARSER}) \\
        $([ -n "${TOOL_CALL_PARSER}" ] && echo --tool-call-parser ${TOOL_CALL_PARSER})
    " &
    VLLM_PID=$!

    # Cross-shard pin verification (Risk 4) must be run separately via
    # `tools/scripts/verify_cross_shard_pins.py` against this service
    # before the model is committed to the production manifest.

    wait "$VLLM_PID"
  else
    # Rank >0: start a worker container that just hosts Ray, attach to
    # the head, and idle. No vLLM admission on this rank.
    start_ray_container "worker" "$HEAD_ADDR"
    echo "rank ${SLURM_PROCID} attached to ray head ${HEAD_ADDR}; idling"
    sleep infinity
  fi
fi

if [ "$#" -gt 0 ]; then
  "$@"
else
  echo "vLLM is ready at http://127.0.0.1:${PORT}/v1"
  echo "Press Ctrl-C or cancel the Slurm job to stop the container."
  sleep infinity
fi
