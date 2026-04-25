"""Benchmark routing config from the registered APXM backend inventory.

Setup (run once):
    dekk apxm backend add vllm-bench --type local --protocol vllm \
        --endpoint http://localhost:8000
    dekk apxm backend add-model vllm-bench <SERVED_MODEL_ID> --alias benchmark
"""
from apxm._generated.providers import VLLM
from apxm.backends import select_backend

BENCHMARK_MODEL_ALIAS = "benchmark"
VLLM_ROUTE = select_backend(protocol=VLLM.protocol, alias=BENCHMARK_MODEL_ALIAS)

__all__ = ["VLLM", "VLLM_ROUTE"]
