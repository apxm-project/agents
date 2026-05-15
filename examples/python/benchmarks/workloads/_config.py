"""Benchmark routing config for the comparable-workloads suite.

Mirrors `examples/python/benchmarks/stress/_config.py` so workload scripts
under `workloads/` resolve the same `vllm-bench` backend alias as the
stress workloads. Setup is the same:

    dekk apxm backend add vllm-bench --type local --protocol vllm \\
        --endpoint http://localhost:8000
    dekk apxm backend add-model vllm-bench <SERVED_MODEL_ID> --alias benchmark
"""
from apxm._generated.providers import VLLM
from apxm.backends import select_backend

BENCHMARK_MODEL_ALIAS = "benchmark"
VLLM_ROUTE = select_backend(protocol=VLLM.protocol, alias=BENCHMARK_MODEL_ALIAS)

__all__ = ["VLLM", "VLLM_ROUTE"]
