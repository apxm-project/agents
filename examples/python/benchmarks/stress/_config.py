"""Benchmark routing config from the registered APXM backend inventory.

Disambiguation rules (no fallback; hard-fail at config time):

1. If `APXM_BENCHMARK_BACKEND` is set in the environment, route to that
   specific backend name. This is the explicit selector when multiple
   backends advertise the `benchmark` alias (e.g. a model zoo with
   `--alias benchmark` on every entry).
2. Else if `APXM_BENCHMARK_MODEL` is set, route to whichever backend
   serves that model id (still unique-or-fail).
3. Else fall back to alias='benchmark' — only valid when a single
   backend has the alias. Ambiguous otherwise; the registry will raise
   `BackendRegistryError` listing the candidates so the operator can
   pick.

Setup (run once):
    dekk apxm backend add vllm-bench --type local --protocol vllm \
        --endpoint http://localhost:8000
    dekk apxm backend add-model vllm-bench <SERVED_MODEL_ID> --alias benchmark

Multi-backend zoo:
    export APXM_BENCHMARK_BACKEND=vllm-fork-qwen3-r0  # disambiguate
    # or:
    export APXM_BENCHMARK_MODEL=Qwen3-32B
"""
import os

from apxm._generated.providers import VLLM
from apxm.backends import select_backend

BENCHMARK_MODEL_ALIAS = "benchmark"
BENCHMARK_BACKEND_ENV = "APXM_BENCHMARK_BACKEND"
BENCHMARK_MODEL_ENV = "APXM_BENCHMARK_MODEL"

_backend_override = os.environ.get(BENCHMARK_BACKEND_ENV, "").strip()
_model_override = os.environ.get(BENCHMARK_MODEL_ENV, "").strip()

if _backend_override:
    # backend is explicit; let select_backend pick the single model on
    # it (no alias/model filter needed — and applying alias=benchmark
    # over-constrains when the operator didn't tag the backend).
    VLLM_ROUTE = select_backend(
        protocol=VLLM.protocol,
        backend=_backend_override,
    )
elif _model_override:
    VLLM_ROUTE = select_backend(protocol=VLLM.protocol, model=_model_override)
else:
    VLLM_ROUTE = select_backend(protocol=VLLM.protocol, alias=BENCHMARK_MODEL_ALIAS)

__all__ = ["VLLM", "VLLM_ROUTE", "BENCHMARK_BACKEND_ENV", "BENCHMARK_MODEL_ENV"]
