#!/usr/bin/env python3
"""apxm_priority_lane.py -- critical-lane latency under background contention.

This workload is shaped to exercise the APXM-vLLM priority path:

* one user-visible critical chain with small output budgets;
* many independent background branches with larger output budgets;
* all LLM calls are ready early enough to create backend queue contention;
* the final result waits for both lanes so total wall time is still recorded.

The publishable metric is not total batch wall time. It is the observed
critical-path finish time emitted by APXM detailed metrics and collected by
``concurrent_matrix.py`` as ``critical_path_finish_ms_*``.

Env knobs:
  APXM_WORKLOAD_PREFIX_TOK          default 2048  shared prompt prefix size
  APXM_WORKLOAD_FANOUT              default 10    background branches
  APXM_PRIORITY_CRITICAL_MAX_TOKENS default 96    per-critical-node cap
  APXM_PRIORITY_BACKGROUND_MAX_TOKENS default 384 per-background-node cap
  APXM_MATRIX_VARIANT               driver-set tenant index
"""

from __future__ import annotations

from apxm import GraphRecorder, compile

from _config import VLLM, VLLM_ROUTE
from _helpers import (
    DEFAULT_PREFIX_TOK,
    ENV_FANOUT,
    ENV_PREFIX_TOK,
    env_int,
    shared_prefix_text,
    variant_index,
)

ENV_CRITICAL_MAX_TOKENS = "APXM_PRIORITY_CRITICAL_MAX_TOKENS"
ENV_BACKGROUND_MAX_TOKENS = "APXM_PRIORITY_BACKGROUND_MAX_TOKENS"
DEFAULT_CRITICAL_MAX_TOKENS = 96
DEFAULT_BACKGROUND_MAX_TOKENS = 384
DEFAULT_BACKGROUND_FANOUT = 10
CRITICAL_PRIORITY = 95
BACKGROUND_PRIORITY = 0


@compile(default_provider=VLLM, default_route=VLLM_ROUTE)
def apxm_priority_lane(g: GraphRecorder):
    variant = variant_index()
    prefix_tok = env_int(ENV_PREFIX_TOK, DEFAULT_PREFIX_TOK)
    fanout = max(2, env_int(ENV_FANOUT, DEFAULT_BACKGROUND_FANOUT))
    critical_max_tokens = env_int(
        ENV_CRITICAL_MAX_TOKENS, DEFAULT_CRITICAL_MAX_TOKENS
    )
    background_max_tokens = env_int(
        ENV_BACKGROUND_MAX_TOKENS, DEFAULT_BACKGROUND_MAX_TOKENS
    )

    prefix = shared_prefix_text(variant, prefix_tok)
    topic = (
        "APXM-vLLM evaluation triage for a graph-aware scheduler, priority "
        "lane, prefix caching, and benchmark publication gate."
    )

    critical_0 = g.ask(
        name="critical_triage",
        prompt=(
            prefix
            + "\n\n"
            + f"TENANT VARIANT: {variant}\n"
            + f"USER REQUEST: {topic}\n"
            + "Return the three highest-risk blockers as compact bullets."
        ),
        priority=CRITICAL_PRIORITY,
        benchmark_milestone="critical_lane_step_0",
        token_budget=critical_max_tokens,
    )
    critical_1 = g.ask(
        name="critical_decision",
        prompt=(
            prefix
            + "\n\n"
            + "Critical triage:\n{critical_0}\n\n"
            + "Choose the one blocker that most affects a publishable APXM "
            "claim. Return a decision and one verification command."
        ),
        priority=CRITICAL_PRIORITY,
        benchmark_milestone="critical_lane_step_1",
        token_budget=critical_max_tokens,
    )
    critical_2 = g.ask(
        name="critical_action",
        prompt=(
            prefix
            + "\n\n"
            + "Critical decision:\n{critical_1}\n\n"
            + "Produce the shortest next action plan for the user-visible "
            "evaluation lane."
        ),
        priority=CRITICAL_PRIORITY,
        benchmark_milestone="critical_lane_step_2",
        token_budget=critical_max_tokens,
    )
    critical_final = g.ask(
        name="critical_user_answer",
        prompt=(
            "User-visible APXM evaluation answer:\n{critical_2}\n\n"
            "Return one paragraph and one command line. Keep it concise."
        ),
        priority=CRITICAL_PRIORITY,
        benchmark_milestone="critical_lane_result",
        token_budget=critical_max_tokens,
    )

    background = []
    for branch_idx in range(fanout):
        task = g.ask(
            name=f"background_audit_{branch_idx}",
            prompt=(
                prefix
                + "\n\n"
                + f"BACKGROUND BRANCH: {branch_idx}\n"
                + "Perform a deeper, non-user-blocking audit of APXM evidence "
                + "quality, benchmark threats, scheduler tradeoffs, and docs "
                + "cleanup. Return detailed notes with concrete file-path-like "
                + "references and commands."
            ),
            priority=BACKGROUND_PRIORITY,
            benchmark_milestone="background_lane",
            token_budget=background_max_tokens,
        )
        background.append(task)

    background_pack = g.merge("background_audit_pack", *background)
    full_pack = g.merge(
        "priority_lane_full_pack",
        critical_final,
        background_pack,
    )
    g.done(full_pack)


if __name__ == "__main__":
    print(apxm_priority_lane._graph.to_air())
