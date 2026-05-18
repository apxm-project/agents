#!/usr/bin/env python3
"""pin_temporal_demo.py — pin's by-design eviction-recovery regime.

The earlier pin_demo.py engages pin (`pinned_blocks_peak_max > 0`) but does
not produce a latency advantage when KV utilization is well below 0.85,
because RadixAttention's natural LRU never has to evict shared blocks.
Pin's value is *preferential preservation across diverse traffic* — keeping
"important" cohort blocks alive while LRU would otherwise evict them.

This workload manufactures that exact scenario inside a single graph:

  Anchor warmup:  cohort_A used with shared_prefix_text(0)
  Flood:          `flood_branches` parallel ASKs with per-branch
                  distinct prefixes (cohort_unique_per_branch).
                  Goal: fill the prefix-cache with traffic the
                  scheduler has no reason to keep.
  Anchor return:  cohort_A used again.

All tenants share cohort_A (`reuse_group="pin_temporal_anchor"` in the
anchor nodes) so the runtime stamps `pin_policy.mode = "prefix"` on those nodes.
Flood branch cohort tags rotate so they do NOT pin (no `reuse_group`
kwarg → `pin_policy.mode = "none"`), letting RadixAttention LRU evict
them naturally.

Without pin: flood traffic + LRU may evict cohort_A blocks before
the return node lands → the return node re-prefills the anchor → measurable latency hit.
With pin:    cohort_A blocks survive the flood → the return node hits the cache →
             measurably faster.

Latency delta on the return node (and therefore total batch wall) is the signal.

Knobs (env-var):
  APXM_WORKLOAD_PREFIX_TOK    default 4096   per-cohort prefix size
  APXM_WORKLOAD_FANOUT        default 8      flood branches per tenant
                                              (drives cache pressure)
  APXM_MATRIX_VARIANT         default 0      tenant index (driver-set)
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

REUSE_GROUP_NAME = "pin_temporal_anchor"
ANCHOR_COHORT_ID = 9_000_000  # well outside the variant_index range


@compile(default_provider=VLLM, default_route=VLLM_ROUTE)
def pin_temporal_demo(g: GraphRecorder):
    variant = variant_index()
    prefix_tok = env_int(ENV_PREFIX_TOK, DEFAULT_PREFIX_TOK)
    flood_branches = max(2, env_int(ENV_FANOUT, DEFAULT_FANOUT))

    anchor_prefix = shared_prefix_text(ANCHOR_COHORT_ID, prefix_tok)

    phase_a = g.ask(
        name="anchor_warm",
        prompt=anchor_prefix + "\n\nQuestion A: name one keyword from the context.",
        reuse_group=REUSE_GROUP_NAME,
    )

    flood_branches_nodes = []
    for branch_idx in range(flood_branches):
        unique_cohort_id = variant * 10_000 + branch_idx
        unique_prefix = shared_prefix_text(unique_cohort_id, prefix_tok)
        flood = g.ask(
            name=f"flood_b{branch_idx}",
            prompt=unique_prefix + f"\n\nQuestion B{branch_idx}: one short fact.",
        )
        flood_branches_nodes.append(flood)

    flood_merge = g.merge("flood_merge", *flood_branches_nodes)

    phase_c_branches = []
    for follow_idx in range(2):
        comeback = g.ask(
            name=f"anchor_return_{follow_idx}",
            prompt=anchor_prefix + f"\n\nQuestion C{follow_idx}: list two keywords.",
            reuse_group=REUSE_GROUP_NAME,
        )
        phase_c_branches.append(comeback)

    g.add_edge(phase_a, flood_branches_nodes[0])
    for branch in flood_branches_nodes[1:]:
        g.add_edge(phase_a, branch)
    for comeback in phase_c_branches:
        g.add_edge(flood_merge, comeback)

    final = g.merge("temporal_final", *phase_c_branches)
    g.done(final)


if __name__ == "__main__":
    print(pin_temporal_demo._graph.to_air())
