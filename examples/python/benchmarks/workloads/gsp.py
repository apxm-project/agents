#!/usr/bin/env python3
"""gsp.py - Generated Shared Prefix workload.

Synthetic microbench for cohort-routing claims. One shared prefix per cohort
(controlled by `APXM_WORKLOAD_COHORT_SIZE`), then a `APXM_WORKLOAD_FANOUT`-way
parallel ask so each tenant exercises within-cohort prefix reuse. With
`COHORT_SIZE > 1`, multiple variants in the same cohort emit the *same*
prefix bytes so the backend's prefix cache treats them as a shared cohort
across tenants.

Graph shape matches the proven pattern from
`stress/prefix_fanout_concurrent.py`: parallel asks → merge → done. The
merge is the terminal so the harness's metrics reliably reflect every
branch's completion.

The workload is deliberately content-deterministic given (variant index,
prefix tokens, cohort size) — no randomness in the prefix — so paired
APXM-on / flat-HTTP runs land identical inputs at the backend.

Usage:
  APXM_WORKLOAD_PREFIX_TOK=4096 APXM_WORKLOAD_FANOUT=8 \\
    dekk apxm execute examples/python/benchmarks/workloads/gsp.py -O2

Or as a concurrent matrix cell (see workloads/README.md).
"""
from apxm import compile, GraphRecorder

from _config import VLLM, VLLM_ROUTE
from _helpers import (
    DEFAULT_COHORT_SIZE,
    DEFAULT_FANOUT,
    DEFAULT_PREFIX_TOK,
    ENV_COHORT_SIZE,
    ENV_FANOUT,
    ENV_PREFIX_TOK,
    cohort_root,
    env_int,
    shared_prefix_text,
    variant_index,
)


@compile(default_provider=VLLM, default_route=VLLM_ROUTE)
def gsp(g: GraphRecorder):
    variant = variant_index()
    prefix_tok = env_int(ENV_PREFIX_TOK, DEFAULT_PREFIX_TOK)
    fanout = max(1, env_int(ENV_FANOUT, DEFAULT_FANOUT))
    cohort_size = max(1, env_int(ENV_COHORT_SIZE, DEFAULT_COHORT_SIZE))

    root = cohort_root(variant, cohort_size)
    prefix = shared_prefix_text(root, prefix_tok)

    branches = []
    for branch_idx in range(fanout):
        question = (
            f"Question {branch_idx}: summarize line {branch_idx * 3} of the "
            f"context above in one short sentence."
        )
        branch = g.ask(
            name=f"gsp_branch_{branch_idx}",
            prompt=prefix + "\n\n" + question,
        )
        branches.append(branch)

    merged = g.merge("gsp_merge", *branches)
    g.done(merged)


if __name__ == "__main__":
    print(gsp._graph.to_air())
