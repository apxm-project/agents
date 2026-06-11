#!/usr/bin/env python3
"""dispatch_open_ended.py — open-ended workflow dispatch demo.

A 1-PLAN-node workflow: the user's free-form request becomes the PLAN
node's `goal`. When the planner LLM responds with a structured
`inner_plan.task_dag`, the runtime's PLAN handler:

  1. Validates the DAG up-front (TaskDag::validate — rejects cycles,
     dangling depends_on, dup ids with an actionable error).
  2. Emits the `PlanWorkflowEmitted` event with `parallel_fanout_max` so
     trace consumers can quantify the extracted parallelism.
  3. Links the task DAG through `task_dag_to_air_module` → compile →
     splice into the live execution DAG (the `inner_plan` path).
  4. Runs the spliced workflow end-to-end and returns the result.

Built from the PLAN primitive and existing Python frontend workflow
authoring.

Usage:
    APXM_BENCHMARK_BACKEND=<your-backend> python3 \\
        examples/python/demos/plan-workflows/dispatch_open_ended.py \\
        "research the prefix-cache literature and produce a 3-section brief"

Backend selection: the PLAN node honors `backend=` / `model=` /
`route=` like any other op. This demo uses `APXM_BENCHMARK_BACKEND` to
match the benchmark workloads' convention so the same `dekk apxm
backend add` setup works.

Sandbox / preview: pass `--dry-run` to skip execution and print the
emitted AIR text — useful for confirming the prompt + schema before
hitting an LLM backend.
"""
from __future__ import annotations

import argparse
import os
import sys

from apxm import GraphRecorder, compile

# Path-side import of the benchmark _config so the demo runs against
# whatever benchmark backend is registered. Keeps the demo zero-config
# for anyone who already followed the model-zoo quickstart.
sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "..", "benchmarks", "workloads"),
)
from _config import VLLM, VLLM_ROUTE  # noqa: E402


DEFAULT_REQUEST = (
    "Research the prefix-cache literature briefly and produce a 3-section "
    "brief: (1) what RadixAttention is, (2) why long-shared-context "
    "workloads benefit most, (3) one concrete failure mode."
)


def _build_dispatch_graph(request: str):
    """Compile a 1-PLAN-node graph that asks the planner LLM to emit a
    task_dag for the request. The PLAN handler does the rest."""

    @compile(default_provider=VLLM, default_route=VLLM_ROUTE)
    def dispatch_graph(g: GraphRecorder):
        # The PLAN handler's runtime behaviour (see
        # crates/runtime/apxm-runtime/src/executor/handlers/plan.rs):
        # when the LLM response includes inner_plan.task_dag, the
        # handler validates -> links -> splices -> executes the inner workflow
        # automatically. No additional workflow nodes are needed here.
        plan_node = g.plan(name="open_ended_plan", goal=request)
        g.done(plan_node)

    return dispatch_graph


def _parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "request",
        nargs="?",
        default=DEFAULT_REQUEST,
        help=(
            "Free-form user request. Defaults to a short research-brief "
            "prompt so the demo runs without arguments."
        ),
    )
    p.add_argument(
        "--dry-run",
        action="store_true",
        help=(
            "Compile the dispatch graph and print its AIR text without "
            "calling any LLM backend. Useful for inspecting the prompt "
            "shape before hitting a live service."
        ),
    )
    return p.parse_args()


def main() -> int:
    args = _parse_args()
    graph = _build_dispatch_graph(args.request)

    if args.dry_run:
        print(graph._graph.to_air())
        return 0

    import apxm

    print(f"[dispatch] request: {args.request}", file=sys.stderr)
    print("[dispatch] running 1-PLAN-node graph; PLAN handler will "
          "splice the inner task_dag the LLM returns.", file=sys.stderr)
    result = apxm.run(graph())
    print(result.content if hasattr(result, "content") else result)
    return 0


if __name__ == "__main__":
    sys.exit(main())
