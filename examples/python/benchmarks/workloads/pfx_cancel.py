#!/usr/bin/env python3
"""pfx_cancel.py - Shared-prefix parallel fanout with cancellable branches.

A single shared prefix (one cohort), `APXM_WORKLOAD_FANOUT` parallel
branches, and `cancel_count = round(fanout * APXM_WORKLOAD_CANCEL_RATE)`
of those branches structurally pruned at compile time (their outputs are
not wired into the merge's downstream, so the dead-context-elimination
pass can drop them at O2). The remaining `fanout - cancel_count` branches
run normally.

Graph shape matches the proven pattern from
`stress/prefix_fanout_concurrent.py`: parallel asks → merge → done. The
merge is the terminal.

This workload exists to feed the comparable-workloads suite a graph shape
that exercises both prefix-cohort routing AND APXM's existing compile-time
elimination surface. When the in-flight `CancelGroup` protocol
ships, the same shape can be re-instrumented to record real
`cancel_count`, `tokens_avoided`, and `cancel_latency_ms` runtime fields.

Usage:
  APXM_WORKLOAD_FANOUT=8 APXM_WORKLOAD_CANCEL_RATE=0.5 \\
    dekk apxm execute examples/python/benchmarks/workloads/pfx_cancel.py -O2
"""
from apxm import compile, GraphRecorder

from _config import VLLM, VLLM_ROUTE
from _helpers import (
    DEFAULT_CANCEL_RATE,
    DEFAULT_COHORT_SIZE,
    DEFAULT_FANOUT,
    DEFAULT_PREFIX_TOK,
    ENV_CANCEL_RATE,
    ENV_COHORT_SIZE,
    ENV_FANOUT,
    ENV_PREFIX_TOK,
    cohort_root,
    env_float,
    env_int,
    shared_prefix_text,
    variant_index,
)


@compile(default_provider=VLLM, default_route=VLLM_ROUTE)
def pfx_cancel(g: GraphRecorder):
    variant = variant_index()
    prefix_tok = env_int(ENV_PREFIX_TOK, DEFAULT_PREFIX_TOK)
    fanout = max(1, env_int(ENV_FANOUT, DEFAULT_FANOUT))
    cohort_size = max(1, env_int(ENV_COHORT_SIZE, DEFAULT_COHORT_SIZE))
    cancel_rate = min(1.0, max(0.0, env_float(ENV_CANCEL_RATE, DEFAULT_CANCEL_RATE)))

    # Ensure at least one live branch so the merge always has an input,
    # while keeping live_count + cancel_count == fanout.
    cancel_count = min(fanout - 1, round(fanout * cancel_rate))
    live_count = fanout - cancel_count

    root = cohort_root(variant, cohort_size)
    prefix = shared_prefix_text(root, prefix_tok)

    live_branches = []
    for branch_idx in range(live_count):
        branch = g.ask(
            name=f"pfx_cancel_live_{branch_idx}",
            prompt=(
                prefix + "\n\n"
                f"Live branch {branch_idx}: name three concrete improvements to the "
                "cohort context above. Be specific."
            ),
        )
        live_branches.append(branch)

    # Cancel candidates: their output is built but never referenced by the
    # merge's downstream, so the dead-context-elimination pass at O2 can
    # prune them. At O0 they run; at O2 they should not.
    for branch_idx in range(cancel_count):
        g.ask(
            name=f"pfx_cancel_speculative_{branch_idx}",
            prompt=(
                prefix + "\n\n"
                f"Speculative branch {branch_idx}: hypothesize an unrelated "
                "extension that the user did not ask for."
            ),
        )

    merged = g.merge("pfx_cancel_merge", *live_branches)
    g.done(merged)


if __name__ == "__main__":
    print(pfx_cancel._graph.to_air())
