#!/usr/bin/env python3
"""apxm_review_council.py -- APXM dogfood workload for graph-aware vLLM.

This workload is intentionally simple and claim-shaped:

* one large shared context built from APXM's own vLLM/evaluation code;
* N parallel specialist reviewers that all read the same prefix;
* one critical-path synthesis node that merges the reviewers.

That is the workflow APXM should make better than flat HTTP: graph
registration, shared-prefix cohorts, priority scheduling, pin telemetry, and
J/req can all be measured on a real APXM task without introducing a new
agent framework.

Env knobs:
  APXM_WORKLOAD_PREFIX_TOK   default 4096   target shared-context size
  APXM_WORKLOAD_FANOUT       default 6      reviewer branches
  APXM_WORKLOAD_REVIEW_MAX_TOKENS   default 256   per-reviewer output cap
  APXM_WORKLOAD_VERDICT_MAX_TOKENS  default 512   synthesis output cap
  APXM_MATRIX_VARIANT        driver-set     tenant index; used only in task id
"""

from __future__ import annotations

from pathlib import Path

from apxm import GraphRecorder, compile

from _config import VLLM, VLLM_ROUTE
from _helpers import (
    DEFAULT_PREFIX_TOK,
    ENV_FANOUT,
    ENV_PREFIX_TOK,
    env_int,
    variant_index,
)

REUSE_GROUP_NAME = "apxm_review_council_shared_context"
ENV_REVIEW_MAX_TOKENS = "APXM_WORKLOAD_REVIEW_MAX_TOKENS"
ENV_VERDICT_MAX_TOKENS = "APXM_WORKLOAD_VERDICT_MAX_TOKENS"
DEFAULT_REVIEW_MAX_TOKENS = 256
DEFAULT_VERDICT_MAX_TOKENS = 512
TOKENS_TO_CHARS = 4
REPO_ROOT = Path(__file__).resolve().parents[4]

SOURCE_PATHS = [
    "docs/backends/vllm.md",
    "docs/backends/model-zoo.md",
    "crates/compiler/apxm-frontend/python/apxm/contract.py",
    "tools/scripts/cross_system_per_cell.sh",
    "examples/python/benchmarks/concurrent_matrix.py",
    "examples/python/benchmarks/workloads/apxm_review_council.py",
]

REVIEW_ROLES = [
    (
        "vllm-contract",
        "Find risks in the APXM-vLLM graph-aware contract, scheduler policy, "
        "or route assumptions. Cite concrete files or functions from the context.",
    ),
    (
        "benchmark-methodology",
        "Find risks in the benchmark methodology, A/B isolation, cache reset, "
        "or artifact placement. Cite concrete files or commands from the context.",
    ),
    (
        "service-operations",
        "Find risks in the Dekk service lifecycle, Slurm service reuse, model "
        "cache layout, or image-store assumptions. Cite concrete files or env vars.",
    ),
    (
        "evidence-quality",
        "Find risks that would make a claim hard to publish: missing manifests, "
        "unclear metrics, unpaired arms, or quality regressions.",
    ),
    (
        "regression-hunter",
        "Find one likely implementation regression or stale instruction in the "
        "context and explain a minimal verification command.",
    ),
    (
        "release-editor",
        "Extract the clearest publication story from the context: what APXM can "
        "claim, what it cannot claim, and the next evidence cell.",
    ),
]


def _read_source(path: str, *, max_chars: int = 9000) -> str:
    source = REPO_ROOT / path
    try:
        text = source.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        text = f"[unavailable: {path}: {exc}]"
    # Curly braces are APXM template syntax. Replace them so source snippets do
    # not accidentally become unresolved prompt variables.
    text = text.replace("{", "(").replace("}", ")")
    if len(text) > max_chars:
        text = text[:max_chars] + "\n[truncated]\n"
    return f"\n===== {path} =====\n{text}\n"


def _shared_context(target_tokens: int) -> str:
    target_chars = max(1024, target_tokens * TOKENS_TO_CHARS)
    header = (
        "APXM REVIEW COUNCIL SHARED CONTEXT\n"
        "Task: audit APXM's evaluation and graph-aware vLLM path. Every reviewer "
        "gets this same prefix, so APXM-on should expose prefix reuse and pin "
        "telemetry while flat HTTP pays the ordinary request path.\n"
        "Grounding rules: cite file paths from the context, avoid unsupported "
        "claims, and distinguish evidence from recommendations.\n"
    )
    chunks = [_read_source(path) for path in SOURCE_PATHS]
    body = header + "\n".join(chunks)
    while len(body) < target_chars:
        body += "\n[repeat-context-for-prefix-pressure]\n" + "\n".join(chunks[:3])
    return body[:target_chars]


@compile(default_provider=VLLM, default_route=VLLM_ROUTE)
def apxm_review_council(g: GraphRecorder):
    variant = variant_index()
    prefix_tok = env_int(ENV_PREFIX_TOK, DEFAULT_PREFIX_TOK)
    fanout = max(2, env_int(ENV_FANOUT, len(REVIEW_ROLES)))
    review_max_tokens = env_int(ENV_REVIEW_MAX_TOKENS, DEFAULT_REVIEW_MAX_TOKENS)
    verdict_max_tokens = env_int(ENV_VERDICT_MAX_TOKENS, DEFAULT_VERDICT_MAX_TOKENS)
    context = _shared_context(prefix_tok)

    reviews = []
    for branch_idx in range(fanout):
        role, instruction = REVIEW_ROLES[branch_idx % len(REVIEW_ROLES)]
        role_key = role.replace("-", "_")
        review = g.ask(
            name=f"review_{branch_idx}_{role_key}",
            prompt=(
                context
                + "\n\n"
                + f"REVIEWER ROLE: {role}\n"
                + f"TENANT VARIANT: {variant}\n"
                + instruction
                + "\nReturn exactly three bullets. Each bullet must include: "
                "risk, evidence path, and next verification command."
            ),
            reuse_group=REUSE_GROUP_NAME,
            shared_prefix_group=REUSE_GROUP_NAME,
            shared_prefix_est_tokens=prefix_tok,
            fanout_count=fanout,
            warmup_candidate=(branch_idx == 0),
            token_budget=review_max_tokens,
        )
        reviews.append(review)

    merged = g.merge("review_council_merge", *reviews)
    verdict = g.ask(
        name="publishable_eval_verdict",
        prompt=(
            "You are the APXM evaluation lead. Synthesize these parallel "
            "review findings into one publishable evaluation plan.\n\n"
            "Reviewer findings:\n{merged}\n\n"
            "Output four sections: headline workflow, primary metrics, "
            "honest negatives, and next run command."
        ),
        token_budget=verdict_max_tokens,
    )
    final_pack = g.merge("review_council_output_pack", merged, verdict)
    g.done(final_pack)


if __name__ == "__main__":
    print(apxm_review_council._graph.to_air())
