"""Benchmark routing config — single source of truth for backend identity.

Setup (run once):
    dekk apxm backend add vllm-bench --type local --protocol vllm \
        --endpoint http://localhost:8000
"""
from apxm._generated.providers import VLLM
from apxm import Vllm

VLLM_BACKEND: str = "vllm-bench"

__all__ = ["VLLM", "Vllm", "VLLM_BACKEND"]
