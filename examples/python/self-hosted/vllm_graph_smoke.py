#!/usr/bin/env python3
"""Small APXM smoke graph for a self-hosted vLLM fork backend.

Usage:
  APXM_VLLM_MODEL=<SERVED_MODEL_ID> \
    dekk apxm execute examples/python/self-hosted/vllm_graph_smoke.py

Optional environment overrides:
  - APXM_VLLM_BACKEND
  - APXM_VLLM_MODEL

Backend requirements:
  - APXM backend name defaults to: vllm-fork
  - Model id must be the served model id registered under that backend
  - Endpoint shape: OpenAI-compatible vLLM fork with /v1 base URL
"""

import os

from apxm import GraphRecorder, compile


DEFAULT_VLLM_BACKEND = "vllm-fork"
ENV_VLLM_BACKEND = "APXM_VLLM_BACKEND"
ENV_VLLM_MODEL = "APXM_VLLM_MODEL"
ENV_EMIT_AIR = "APXM_EMIT_AIR"
ENV_FLAG_ENABLED = "1"
NODE_ARCHITECTURE = "architecture"
NODE_SUMMARY = "summary"
NODE_PRINT_OUTPUT = "print_output"
PROMPT_ARCHITECTURE = (
    "You are running behind the APXM graph-aware vLLM fork. "
    "In 4 short bullets, explain what a graph-aware inference boundary "
    "can do that isolated prompt calls cannot."
)
PROMPT_SUMMARY = (
    "Turn this into a 3 sentence summary for an engineer validating a "
    "self-hosted model smoke test on the vLLM fork backend:\n\n{architecture}"
)
OUTPUT_HEADER = "=== VLLM FORK GRAPH SMOKE ==="
OUTPUT_TEMPLATE = (
    "{header}\n"
    "backend={backend}\n"
    "model={model}\n\n"
    "{{summary}}"
)


def required_env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"{name} must be set to the served model id")
    return value


VLLM_BACKEND = os.environ.get(ENV_VLLM_BACKEND, DEFAULT_VLLM_BACKEND)
VLLM_MODEL = required_env(ENV_VLLM_MODEL)


@compile(
    default_backend=VLLM_BACKEND,
    default_model=VLLM_MODEL,
)
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
            backend=VLLM_BACKEND,
            model=VLLM_MODEL,
        ),
    )

    g.done(output)


if __name__ == "__main__":
    if os.environ.get(ENV_EMIT_AIR) == ENV_FLAG_ENABLED:
        print(vllm_graph_smoke._air_text)
    else:
        import apxm

        result = apxm.run(vllm_graph_smoke())
        print(result.content)
