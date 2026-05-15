"""Pull selected vLLM Prometheus counters and compute deltas / hit rate.

Used by benchmark harnesses to A/B prefix-cache reuse across cells. The
counters tracked here are the ones vLLM emits at `/metrics` for KV-cache
behavior; sums are aggregated across all label permutations of a counter.

Public surface:
    snapshot(metrics_url, *, timeout_s) -> dict[str, float]
    delta(before, after)                -> dict[str, float]
    hit_rate(delta_counters)            -> float | None

The module raises `urllib.error.URLError` (or `TimeoutError`) on a
connection failure during `snapshot` so that callers can fail fast in a
benchmark cell rather than silently recording empty data.
"""

from __future__ import annotations

import urllib.request

PREFIX_CACHE_QUERIES_TOTAL = "vllm:prefix_cache_queries_total"
PREFIX_CACHE_HITS_TOTAL = "vllm:prefix_cache_hits_total"
PROMPT_TOKENS_TOTAL = "vllm:prompt_tokens_total"
PROMPT_TOKENS_CACHED_TOTAL = "vllm:prompt_tokens_cached_total"
GENERATION_TOKENS_TOTAL = "vllm:generation_tokens_total"
REQUEST_SUCCESS_TOTAL = "vllm:request_success_total"

SELECTED_COUNTERS: tuple[str, ...] = (
    PREFIX_CACHE_QUERIES_TOTAL,
    PREFIX_CACHE_HITS_TOTAL,
    PROMPT_TOKENS_TOTAL,
    PROMPT_TOKENS_CACHED_TOTAL,
    GENERATION_TOKENS_TOTAL,
    REQUEST_SUCCESS_TOTAL,
)

DEFAULT_TIMEOUT_S = 30.0
COMMENT_PREFIX = "#"


def snapshot(metrics_url: str, *, timeout_s: float = DEFAULT_TIMEOUT_S) -> dict[str, float]:
    """Read selected vLLM counters at `metrics_url`.

    Returns a dict keyed by counter name with values summed across labels.
    Counters that do not appear in the response are omitted (not zero-filled).
    """
    with urllib.request.urlopen(metrics_url, timeout=timeout_s) as resp:
        text = resp.read().decode("utf-8")
    counters: dict[str, float] = {}
    for line in text.splitlines():
        if line.startswith(COMMENT_PREFIX):
            continue
        for name in SELECTED_COUNTERS:
            if line.startswith(name):
                value = float(line.rsplit(" ", 1)[-1])
                counters[name] = counters.get(name, 0.0) + value
                break
    return counters


def delta(before: dict[str, float], after: dict[str, float]) -> dict[str, float]:
    """Per-counter delta (after - before) across the union of keys.

    Missing keys on either side are treated as 0.0 so the delta is always
    non-negative for monotonic counters under normal vLLM behavior.
    """
    keys = sorted(set(before) | set(after))
    return {k: after.get(k, 0.0) - before.get(k, 0.0) for k in keys}


def hit_rate(delta_counters: dict[str, float]) -> float | None:
    """Compute prefix-cache hit rate from a delta dict.

    Returns hits / queries when queries > 0, else None (no traffic).
    """
    queries = delta_counters.get(PREFIX_CACHE_QUERIES_TOTAL, 0.0)
    if queries <= 0:
        return None
    hits = delta_counters.get(PREFIX_CACHE_HITS_TOTAL, 0.0)
    return hits / queries
