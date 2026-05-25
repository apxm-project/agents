#!/usr/bin/env bash
# Sequential APXM Review Council runs across multiple Dekk-managed vLLM
# services. This intentionally runs one model service at a time so the 120B
# deployment can own all GPUs without contaminating smaller-model numbers.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

: "${TS:=$(date -u +%Y%m%dT%H%M%SZ)}"
: "${GPTOSS_ITER:=10}"
: "${GPTOSS_CONC:=16}"
: "${GPTOSS_PREFIX_TOK:=8192}"
: "${GPTOSS_FANOUT:=6}"
: "${MISTRAL_ITER:=10}"
: "${MISTRAL_CONC:=8}"
: "${MISTRAL_PREFIX_TOK:=4096}"
: "${MISTRAL_FANOUT:=6}"
: "${SERVICE_READY_TIMEOUT_SECONDS:=7200}"

wait_for_service() {
  local service="$1"
  local backend="$2"
  local deadline=$((SECONDS + SERVICE_READY_TIMEOUT_SECONDS))
  local status=1

  echo "[review-multimodel] waiting for $service readiness probe and backend registration"
  while [ "$SECONDS" -lt "$deadline" ]; do
    set +e
    dekk apxm vllm service-status "$service" --probe
    status="$?"
    set -e
    if [ "$status" -eq 0 ] && dekk apxm backend list 2>/dev/null | grep -q "$backend"; then
      return 0
    fi
    sleep 30
  done

  echo "[review-multimodel] ERROR: $service did not pass probe within ${SERVICE_READY_TIMEOUT_SECONDS}s" >&2
  return 1
}

run_model() {
  local label="$1"
  local manifest="$2"
  local service="$3"
  local endpoint="$4"
  local model="$5"
  local backend="$6"
  local iter="$7"
  local conc="$8"
  local prefix_tok="$9"
  local fanout="${10}"

  echo "[review-multimodel] === $label: applying $manifest ==="
  dekk apxm vllm zoo-apply "$manifest" --prune
  wait_for_service "$service" "$backend"

  echo "[review-multimodel] === $label: running full review council ==="
  SERVICE="$service" \
  APXM_ENDPOINT="$endpoint" \
  MODEL="$model" \
  ITER="$iter" \
  CONC="$conc" \
  PREFIX_TOK="$prefix_tok" \
  FANOUT="$fanout" \
  TS="${TS}-${label}" \
    tools/scripts/run_apxm_review_council.sh
}

run_model \
  "gptoss120b" \
  "deploy/vllm/zoo.review-gptoss.toml" \
  "vllm-gptoss" \
  "http://127.0.0.1:8916" \
  "gpt-oss-120b" \
  "vllm-fork" \
  "$GPTOSS_ITER" \
  "$GPTOSS_CONC" \
  "$GPTOSS_PREFIX_TOK" \
  "$GPTOSS_FANOUT"

run_model \
  "mistral7b" \
  "deploy/vllm/zoo.review-mistral7b.toml" \
  "vllm-mistral7b" \
  "http://127.0.0.1:8920" \
  "Mistral-7B-Instruct-v0.3" \
  "vllm-fork-mistral7b" \
  "$MISTRAL_ITER" \
  "$MISTRAL_CONC" \
  "$MISTRAL_PREFIX_TOK" \
  "$MISTRAL_FANOUT"

echo "[review-multimodel] DONE TS=$TS"
