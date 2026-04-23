#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../.." && pwd)"
vllm_dir="${repo_root}/external/vllm"

if [[ ! -f "${vllm_dir}/pyproject.toml" ]]; then
  git -C "${repo_root}" submodule update --init external/vllm
fi

if [[ ! -f "${vllm_dir}/pyproject.toml" ]]; then
  echo "external/vllm is not available after submodule init" >&2
  exit 1
fi

if ! command -v uv >/dev/null 2>&1; then
  curl -LsSf https://astral.sh/uv/install.sh | sh
  export PATH="${HOME}/.local/bin:${PATH}"
fi

cd "${vllm_dir}"

if [[ ! -x ".venv/bin/python" ]]; then
  uv venv --python 3.12 .venv
fi

if [[ "${APXM_VLLM_USE_PRECOMPILED:-1}" == "1" ]]; then
  VLLM_USE_PRECOMPILED=1 \
    uv pip install --python .venv/bin/python -e . --torch-backend=auto
else
  uv pip install --python .venv/bin/python -e . --torch-backend=auto
fi

cat >&2 <<'EOF'
Installed the fork from external/vllm using the fork-local uv workflow.

Next:
  dekk apxm vllm serve <HF_MODEL_ID>

If you prefer the explicit direct command:
  external/vllm/.venv/bin/python -m vllm.entrypoints.cli.main serve <HF_MODEL_ID>

If this host-native install is not viable for your GPU environment, use the
direct-mounted Docker path documented in docs/external-vllm-fork.md so the
editable install still runs from the visible external/vllm checkout.
EOF
