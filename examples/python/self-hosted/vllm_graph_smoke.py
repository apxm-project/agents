#!/usr/bin/env python3
"""Small APXM smoke graph for a self-hosted vLLM fork backend.

Usage:
  dekk apxm execute examples/python/self-hosted/vllm_graph_smoke.py

Routing:
  Register one model under a vLLM backend with alias "smoke", or replace
  SMOKE_ROUTE with another explicit registered backend/model selector.

The APXM compiler driver sets the emit flag when it needs AIR.
"""

from apxm import GraphRecorder, compile, emit_air_if_requested
from apxm._generated.providers import VLLM
from apxm.backends import select_backend


SMOKE_MODEL_ALIAS = "smoke"
NODE_ARCHITECTURE = "architecture"
NODE_SUMMARY = "summary"
NODE_PRINT_OUTPUT = "print_output"
PROMPT_ARCHITECTURE = (
    "You are running behind the APXM graph-aware vLLM fork. "
    "In 4 short bullets, list the evidence an operator should inspect to "
    "confirm that a graph-aware inference boundary is active. Mention only "
    "route selection, graph registration, node/session metrics, and backend "
    "graph status. Do not claim speedup, cache-hit rate, token reduction, or "
    "cost savings."
)
PROMPT_SUMMARY = (
    "Turn this into a 3 sentence evidence summary for an engineer validating "
    "a self-hosted model smoke test on the vLLM fork backend. Keep the summary "
    "strictly about what the run can verify from emitted metrics and backend "
    "graph status:\n\n{architecture}"
)
OUTPUT_HEADER = "=== VLLM FORK GRAPH SMOKE ==="
OUTPUT_TEMPLATE = (
    "{header}\n"
    "backend={backend}\n"
    "model={model}\n\n"
    "{{summary}}"
)


SMOKE_ROUTE = select_backend(protocol=VLLM.protocol, alias=SMOKE_MODEL_ALIAS)


@compile(default_route=SMOKE_ROUTE)
def vllm_graph_smoke(g: GraphRecorder):
    """Minimal graph-aware smoke test for a self-hosted model."""

    architecture = g.ask(
        name=NODE_ARCHITECTURE,
        prompt=PROMPT_ARCHITECTURE,
    )

    summary = g.think(
        name=NODE_SUMMARY,
        prompt=PROMPT_SUMMARY,
    )

    output = g.print(
        name=NODE_PRINT_OUTPUT,
        message=OUTPUT_TEMPLATE.format(
            header=OUTPUT_HEADER,
            backend=SMOKE_ROUTE.backend,
            model=SMOKE_ROUTE.model,
        ),
    )

    g.done(output)


if __name__ == "__main__":
    if emit_air_if_requested(vllm_graph_smoke):
        raise SystemExit(0)

    import apxm

    result = apxm.run(vllm_graph_smoke())
    print(result.content)
