"""Route aliases for the Gemma 4 demo workflows.

The launcher (`scripts/run_demo.py`) writes a generated APXM config that maps
these aliases to whatever vLLM model is registered locally. Workflows reference
the alias only — never a hardcoded model id — so the same `.apxmobj` artifact
runs against any model the operator chose.
"""

from __future__ import annotations

from apxm._generated.providers import VLLM
from apxm.backends import select_backend


SHOWCASE_ALIAS = "showcase"
BENCHMARK_ALIAS = "benchmark"


def _select_alias(alias: str):
    return select_backend(protocol=VLLM.protocol, alias=alias)


def __getattr__(name: str):
    if name == "SHOWCASE_ROUTE":
        route = _select_alias(SHOWCASE_ALIAS)
        globals()[name] = route
        return route
    if name == "BENCHMARK_ROUTE":
        route = _select_alias(BENCHMARK_ALIAS)
        globals()[name] = route
        return route
    raise AttributeError(name)

__all__ = [
    "VLLM",
    "SHOWCASE_ALIAS",
    "SHOWCASE_ROUTE",
    "BENCHMARK_ALIAS",
    "BENCHMARK_ROUTE",
]
