"""String constants used across the tier-3 harness.

Two source-of-truth boundaries cross language lines:
    1. Session file names (results.json, metrics.json) and the JSON keys
       they contain are owned by Rust constants in `apxm-core::constants::session::*`.
       The values below are hand-mirrored. A drift test in
       `tests/test_keys_match_rust.py` parses the Rust file and asserts equality.
    2. Rubric / budget TOML keys are owned by *this* harness — Python is the
       source of truth and Rust does not consume them.

CLI command + flag names live in their own block below; they pin the CLI
contract the runner depends on.

Adding a new key: add it to the right block here, reference it from the
relevant module, and (for Rust-backed keys) add the matching `pub const`
to `apxm-core::constants` so the drift test stays green.
"""

from __future__ import annotations

from typing import Final


# ---------------------------------------------------------------------------
# Session files (mirror of `apxm-core::constants::session::files`)

class SessionFiles:
    RESULTS: Final[str] = "results.json"
    METRICS: Final[str] = "metrics.json"
    NODE_STATUSES: Final[str] = "node_statuses.json"
    MANIFEST: Final[str] = "manifest.json"
    INPUT_GRAPH: Final[str] = "input.air"
    TRACE: Final[str] = "trace.ndjson"
    LIVE: Final[str] = "live.json"
    NODES_DIR: Final[str] = "nodes"


# ---------------------------------------------------------------------------
# Keys serialized into results.json
# (mirror of `apxm-core::constants::session::results_keys`)

class ResultsKeys:
    NODE_OUTPUTS: Final[str] = "node_outputs"
    TOKEN_VALUES: Final[str] = "token_values"
    EXIT_VALUES: Final[str] = "exit_values"
    FINAL_NODE_ID: Final[str] = "final_node_id"
    FINAL_OUTPUT: Final[str] = "final_output"


# ---------------------------------------------------------------------------
# Keys serialized into metrics.json by TokenAccountingSnapshot::to_json
# (mirror of `apxm-core::constants::session::metrics_keys`)

class MetricsKeys:
    TOKEN_ACCOUNTING: Final[str] = "token_accounting"
    TOTAL: Final[str] = "total"
    PER_NODE: Final[str] = "per_node"
    PER_FLOW: Final[str] = "per_flow"
    PER_AGENT: Final[str] = "per_agent"
    INPUT_TOKENS: Final[str] = "input_tokens"
    OUTPUT_TOKENS: Final[str] = "output_tokens"
    TOTAL_TOKENS: Final[str] = "total_tokens"
    CALL_COUNT: Final[str] = "call_count"


# ---------------------------------------------------------------------------
# expected.toml keys (Python is the source of truth)

class RubricKeys:
    MUST_CONTAIN: Final[str] = "must_contain"
    MUST_NOT_CONTAIN: Final[str] = "must_not_contain"
    REGEX_MATCH: Final[str] = "regex_match"
    MIN_CHARS: Final[str] = "min_chars"
    MAX_CHARS: Final[str] = "max_chars"
    CASE_SENSITIVE: Final[str] = "case_sensitive"
    JUDGE_PROMPT: Final[str] = "judge_prompt"
    JUDGE_THRESHOLD: Final[str] = "judge_threshold"


# ---------------------------------------------------------------------------
# budget.toml keys (Python is the source of truth)

class BudgetKeys:
    MAX_LLM_CALLS: Final[str] = "max_llm_calls"
    MAX_TOTAL_TOKENS: Final[str] = "max_total_tokens"


# ---------------------------------------------------------------------------
# Per-fixture file names under tests/quality_fixtures/<name>/

class FixtureFiles:
    GRAPH: Final[str] = "graph.air"
    EXPECTED: Final[str] = "expected.toml"
    BUDGET: Final[str] = "budget.toml"
    GOLDEN: Final[str] = "golden_output.txt"


# ---------------------------------------------------------------------------
# CLI surface the runner shells into. Pinning these keeps the dispatcher
# from drifting when the parent dekk wrapper renames flags.

class Cli:
    DEKK: Final[str] = "dekk"
    APXM: Final[str] = "apxm"
    EXECUTE: Final[str] = "execute"
    OPT_FLAG: Final[str] = "-O"
    EMIT_SESSION_FLAG: Final[str] = "--emit-session"


# Default LLM-judge backend model. Lives next to other constants because
# it's a public contract (changes here flip what every fixture's judge
# scores against).
DEFAULT_JUDGE_MODEL: Final[str] = "claude-haiku-4-5-20251001"
DEFAULT_JUDGE_THRESHOLD: Final[int] = 4
