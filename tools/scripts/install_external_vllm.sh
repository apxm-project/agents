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

uv pip install --python .venv/bin/python --torch-backend=auto -r requirements/build.txt
uv pip install --python .venv/bin/python -e . --torch-backend=auto --no-build-isolation

current_head="$(git -C "${vllm_dir}" rev-parse --short HEAD 2>/dev/null || true)"
origin_url="$(git -C "${vllm_dir}" remote get-url origin 2>/dev/null || true)"
[[ -n "${current_head}" ]] && echo "external/vllm HEAD=${current_head}" >&2
[[ -n "${origin_url}" ]] && echo "external/vllm origin=${origin_url}" >&2

"${vllm_dir}/.venv/bin/python" - "${vllm_dir}" <<'PY'
import importlib
import sys
from pathlib import Path

expected = Path(sys.argv[1]).resolve()
vllm = importlib.import_module("vllm")
router = importlib.import_module("vllm.entrypoints.openai.apxm.api_router")
api_server = importlib.import_module("vllm.entrypoints.openai.api_server")

got = Path(vllm.__file__).resolve().parent.parent
if got != expected:
    print(
        f"ERROR: vllm imports from {got}, expected fork at {expected}",
        file=sys.stderr,
    )
    sys.exit(1)

router_path = Path(router.__file__).resolve()
if expected not in router_path.parents:
    print(
        f"ERROR: APXM router imports from {router_path}, expected under {expected}",
        file=sys.stderr,
    )
    sys.exit(1)

api_server_text = Path(api_server.__file__).read_text(encoding="utf-8")
if "vllm.entrypoints.openai.apxm.api_router" not in api_server_text:
    print(
        "ERROR: OpenAI API server does not mount the APXM router from this checkout.",
        file=sys.stderr,
    )
    sys.exit(1)

print("OK: editable install resolves to external/vllm fork")
print(f"OK: APXM router imports from {router_path}")
PY

cat >&2 <<'EOF'
Installed the fork from external/vllm using the fork-local uv workflow.

Next:
  dekk apxm vllm start <MODEL_REF> --served-model-name <SERVED_MODEL_ID> --wait

If this host-native install is not viable for your GPU environment, use the
direct-mounted Docker path documented in docs/external-vllm-fork.md so the
editable install still runs from the visible external/vllm checkout.
EOF
