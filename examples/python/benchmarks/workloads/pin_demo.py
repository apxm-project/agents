#!/usr/bin/env python3
"""pin_demo.py - Force-engage the APXM pin path with an explicit reuse_group.

prefix_fanout_concurrent and gsp produce graphs where the *content*
shares a prefix but the compiler does not (yet) tag a `reuse_group`
node attribute, so the request never carries `pin_policy.mode = "prefix"`
and the vLLM scheduler's APXM pin code path is dormant.

This workload tags every fanout branch with the same explicit
`reuse_group="pin_demo_cohort"` so the runtime stamps
`pin_policy.mode = "prefix"` on each request. When KV utilization is
below the pin gate threshold (`_APXM_PIN_ALLOW_USAGE = 0.85`) the
vLLM scheduler's `_can_pin_apxm_policy` returns True and pin handles
get created. `pinned_blocks_peak_max` rises above 0 in the apxm-on arm.

Knobs (env-var):
  APXM_WORKLOAD_PREFIX_TOK   default 4096   shared-prefix size
  APXM_WORKLOAD_FANOUT       default 8      branches sharing the prefix
  APXM_MATRIX_VARIANT        default 0      tenant index (driver-set)
"""
import os

from apxm import compile, GraphRecorder

from _config import VLLM, VLLM_ROUTE
from _helpers import (
    DEFAULT_FANOUT,
    DEFAULT_PREFIX_TOK,
    ENV_FANOUT,
    ENV_PREFIX_TOK,
    env_int,
    shared_prefix_text,
    variant_index,
)

REUSE_GROUP_NAME = "pin_demo_cohort"


@compile(default_provider=VLLM, default_route=VLLM_ROUTE)
def pin_demo(g: GraphRecorder):
    variant = variant_index()
    prefix_tok = env_int(ENV_PREFIX_TOK, DEFAULT_PREFIX_TOK)
    fanout = max(2, env_int(ENV_FANOUT, DEFAULT_FANOUT))

    # All tenants land in the same reuse_group so the vLLM scheduler
    # treats their pinned blocks as one cohort. This is the explicit
    # version of what the compiler's prefix-analysis pass would emit
    # given the same workload.
    prefix = shared_prefix_text(variant, prefix_tok)

    branches = []
    for branch_idx in range(fanout):
        question = (
            f"Question {branch_idx}: summarize line {branch_idx * 3} of the "
            f"context above in one short sentence."
        )
        branch = g.ask(
            name=f"pin_demo_b{branch_idx}",
            prompt=prefix + "\n\n" + question,
            reuse_group=REUSE_GROUP_NAME,
        )
        branches.append(branch)

    merged = g.merge("pin_demo_merge", *branches)
    g.done(merged)


if __name__ == "__main__":
    print(pin_demo._graph.to_air())
