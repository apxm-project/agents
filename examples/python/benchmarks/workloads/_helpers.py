"""Helpers shared across the comparable-workloads suite.

Workload scripts under `workloads/` follow a strict env-var contract so the
existing `concurrent_matrix.py` driver can launch many tenants in parallel
with `--graph workloads/<name>.py` and collect a `BatchRow` CSV without per-
workload special-casing. This module centralises that contract.

Env-var contract (read by every workload):
  APXM_MATRIX_VARIANT       int >= 0  — tenant index, set by the driver
  APXM_WORKLOAD_PREFIX_TOK  int       — target token count for shared prefix
  APXM_WORKLOAD_FANOUT      int       — number of parallel branches
  APXM_WORKLOAD_COHORT_SIZE int       — tenants sharing the same prefix root
  APXM_WORKLOAD_CANCEL_RATE float     — fraction of branches expected to be cancelled

All knobs are optional with sensible defaults so a workload can be invoked
directly with `dekk apxm execute workloads/<name>.py` for ad-hoc testing.
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

_REPO_ROOT = Path(__file__).resolve().parents[4]
_APXM_PKG_DIR = str(_REPO_ROOT / "crates" / "compiler" / "apxm-frontend" / "python")
if _APXM_PKG_DIR not in sys.path:
    sys.path.insert(0, _APXM_PKG_DIR)

from apxm.contract import EnvVar  # noqa: E402

ENV_VARIANT = EnvVar.APXM_MATRIX_VARIANT.value
ENV_PREFIX_TOK = "APXM_WORKLOAD_PREFIX_TOK"
ENV_FANOUT = "APXM_WORKLOAD_FANOUT"
ENV_COHORT_SIZE = "APXM_WORKLOAD_COHORT_SIZE"
ENV_CANCEL_RATE = "APXM_WORKLOAD_CANCEL_RATE"

DEFAULT_PREFIX_TOK = 1024
DEFAULT_FANOUT = 8
DEFAULT_COHORT_SIZE = 1
DEFAULT_CANCEL_RATE = 0.0

# Each cohort root shares a single prefix across `cohort_size` consecutive
# variant indices. Variants in different cohorts get distinct prefixes so
# the backend's prefix cache treats them as disjoint blocks.
#
# Approximate tokens emitted per filler line of the shared prefix.
# Calibrated from the line template length (~75 chars at ~4 chars/token).
# This is a coarse target; actual token counts depend on the model
# tokenizer. Workloads using `shared_prefix_text` should treat the
# `target_tokens` argument as a soft lower bound, not an exact size.
TOKENS_PER_FILLER_LINE = 18


def env_int(name: str, default: int) -> int:
    raw = os.environ.get(name)
    if raw is None or raw == "":
        return default
    try:
        return max(0, int(raw))
    except ValueError:
        return default


def env_float(name: str, default: float) -> float:
    raw = os.environ.get(name)
    if raw is None or raw == "":
        return default
    try:
        return max(0.0, float(raw))
    except ValueError:
        return default


def variant_index() -> int:
    return env_int(ENV_VARIANT, 0)


def cohort_root(variant: int, cohort_size: int) -> int:
    """Map a variant index to its cohort's root id (shared-prefix anchor)."""
    if cohort_size <= 1:
        return variant
    return (variant // cohort_size) * cohort_size


def shared_prefix_text(cohort_root_id: int, target_tokens: int) -> str:
    """Build a deterministic prefix of approximately `target_tokens` tokens.

    All variants whose `cohort_root` matches `cohort_root_id` produce the
    same prefix string byte-for-byte, so the backend's prefix cache can
    treat them as a shared cohort.
    """
    header = (
        f"# Cohort {cohort_root_id} shared context\n"
        f"# Target prefix size: {target_tokens} tokens (approx)\n\n"
    )
    body_lines = []
    line_count = max(1, target_tokens // TOKENS_PER_FILLER_LINE)
    for i in range(line_count):
        body_lines.append(
            f"context_line_{i:05d}: cohort={cohort_root_id} "
            f"deterministic filler chunk for prefix-cache reuse measurement"
        )
    return header + "\n".join(body_lines) + "\n"
