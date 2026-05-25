#!/usr/bin/env python3
"""APXM/vLLM legacy-pattern lint.

Hard-fails on any occurrence of the patterns the model-zoo migration
removes — legacy CLI surface, fallback/silent-skip chains, or capability
flags that mask a missing fork. Wire as a Dekk pre-merge hook or CI step
to prevent the patterns from reappearing.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class LintRule:
    name: str
    pattern: str
    description: str
    include_globs: tuple[str, ...]
    exclude_globs: tuple[str, ...] = ()


REPO_ROOT = Path(__file__).resolve().parents[2]


RULES: tuple[LintRule, ...] = (
    # --- Legacy CLI tokens ---------------------------------------------
    LintRule(
        # CLI invocations only; retrospective comments about removed commands
        # are legitimate and must not trigger the lint.
        name="legacy-service-start",
        pattern=r"dekk\s+apxm\s+vllm\s+service-start\b|sbatch\s+.*service-start",
        description="`service-start` CLI removed by the model-zoo migration; use `zoo apply` instead",
        include_globs=("tools/**/*.py", "docs/**/*.md", "deploy/**/*", "examples/**/*", ".dekk.toml", "README.md"),
        exclude_globs=("tools/scripts/check_no_legacy_vllm.py",),
    ),
    LintRule(
        name="legacy-service-adopt",
        pattern=r"dekk\s+apxm\s+vllm\s+service-adopt\b",
        description="`service-adopt` CLI deleted by the model-zoo migration; write a zoo.toml entry + zoo apply",
        include_globs=("tools/**/*.py", "docs/**/*.md", "deploy/**/*", "examples/**/*", ".dekk.toml", "README.md"),
        exclude_globs=("tools/scripts/check_no_legacy_vllm.py",),
    ),
    LintRule(
        name="legacy-run-vllm-slurm",
        pattern=r"run-vllm-slurm\.sh",
        description="`run-vllm-slurm.sh` deleted by the model-zoo migration; use unified deploy/vllm/run-vllm.sh",
        include_globs=("tools/**/*.py", "docs/**/*.md", "deploy/**/*", "examples/**/*", ".dekk.toml", "README.md"),
        exclude_globs=("tools/scripts/check_no_legacy_vllm.py",),
    ),
    LintRule(
        name="hardcoded-port-8916",
        pattern=r"\b8916\b",
        description=(
            "Hardcoded port 8916 outside the allocator range default. Use "
            "_allocate_port() or a manifest-supplied port instead."
        ),
        include_globs=("tools/**/*.py", "docs/**/*.md", "deploy/**/*", "examples/**/*", "README.md"),
        exclude_globs=(
            "tools/scripts/vllm.py",  # PORT_ALLOCATOR_MIN definition
            "crates/compiler/apxm-frontend/python/apxm/contract.py",  # historical defaults removed
            "tools/scripts/check_no_legacy_vllm.py",
            "docs/backends/model-zoo.md",  # explains the allocator range
            "docs/backends/vllm.md",  # legitimate concept reference
            "deploy/vllm/zoo.toml",  # operator's local manifest (gitignored)
            "deploy/vllm/zoo.example.toml",  # template for operators
            "deploy/vllm/zoo.test-*.toml",  # checked-in smoke-test manifests
            "deploy/vllm/zoo.review-*.toml",  # checked-in evaluation-service manifests
            "docs/preregistrations/**",  # evidence artifacts cite exact run state
            "docs/claims/**",  # claim files cite exact run state
            "docs/evaluation/**",  # result write-ups cite exact run state
            "docs/paper/**",  # paper drafts cite exact run state
            "docs/backends/model-zoo-quickstart.md",  # operator walkthrough; concrete example port
            "deploy/vllm/README.md",  # references allocator range
            # Test fixtures: stub endpoints, never actually contacted.
            "**/tests/**",
        ),
    ),
    # --- Fallback / capability-flag patterns ---------------------------
    LintRule(
        name="apxm-endpoints-available-flag",
        # Match actual references (declaration, field access, method call) —
        # anchor on syntax so retrospective comments don't trigger the lint.
        pattern=r"apxm_endpoints_available\s*[.:(]|self\.apxm_endpoints_available",
        description=(
            "Capability flag removed by the model-zoo migration; backend.rs probes "
            "/v1/apxm/scheduler synchronously in health_check instead."
        ),
        include_globs=("crates/**/*.rs",),
        exclude_globs=("tools/scripts/check_no_legacy_vllm.py",),
    ),
    LintRule(
        name="resolver-last-resort",
        pattern=r"Last resort: return any backend",
        description=(
            "`Last resort: return any backend` branch removed by the model-zoo migration; "
            "resolver hard-fails with `no healthy backends for <model>`."
        ),
        include_globs=("crates/runtime/apxm-backends/src/**/*.rs",),
    ),
    LintRule(
        name="resolver-rr-fallback",
        # Catch the actual call site, not prose that mentions the fallback.
        pattern=r"return\s+select_first_healthy\(",
        description=(
            "round-robin → first-healthy fallback removed by the model-zoo migration; "
            "exhausted pool surfaces as an explicit routing error."
        ),
        include_globs=("crates/runtime/apxm-backends/src/llm/registry/resolver.rs",),
    ),
    LintRule(
        # Match actual identifier references (Python/Rust enum, quoted
        # literal) — explanatory prose about why FCFS is unsupported is
        # legitimate and must not trigger the lint.
        name="scheduling-policy-fcfs",
        pattern=r"SchedulingPolicy\.FCFS\b|SchedulingPolicy::FCFS\b|\"fcfs\"|'fcfs'",
        description=(
            "SchedulingPolicy.FCFS removed by the model-zoo migration; manifest only "
            "accepts `priority`. Fork's FCFS branch becomes upstream-only."
        ),
        include_globs=(
            "crates/compiler/apxm-frontend/python/apxm/contract.py",
            "crates/runtime/apxm-backends/**/*.rs",
            "tools/scripts/vllm.py",
            "examples/python/benchmarks/**/*.py",
        ),
        exclude_globs=("tools/scripts/check_no_legacy_vllm.py",),
    ),
    LintRule(
        name="or-env-or-default-chain",
        pattern=r"or os\.environ\.get|or _default_",
        description=(
            "`args.X or env or default` chain removed by the model-zoo migration; every "
            "required value must be supplied explicitly (manifest or env)."
        ),
        include_globs=("tools/scripts/vllm.py",),
    ),
    LintRule(
        name="already-exists-skipping",
        pattern=r"already exists; skipping",
        description=(
            "Silent-skip on duplicate registration removed by the model-zoo migration; "
            "reconciliation is the zoo's job, not enable's."
        ),
        include_globs=("tools/scripts/vllm.py",),
    ),
    LintRule(
        # A non-empty shell default like
        # `REASONING_PARSER="${REASONING_PARSER:-openai_gptoss}"` silently
        # applies a model-specific parser to every model, crashing
        # non-matching models at vLLM startup with vocab KeyErrors.
        # Model-specific feature toggles must be empty by default in the
        # shell wrapper and supplied per-deployment in the zoo manifest.
        # The empty form `${VAR:-}` is allowed; non-empty defaults are not.
        name="shell-model-specific-default",
        pattern=(
            r"\$\{(REASONING_PARSER|TOOL_CALL_PARSER|ENABLE_AUTO_TOOL_CHOICE)"
            r":-[^}]+\}"
        ),
        description=(
            "Model-specific shell default in deploy/vllm/*.sh is "
            "prohibited; manifest entry must supply reasoning_parser / "
            "tool_call_parser / enable_auto_tool_choice per [[deployment]]. "
            "Empty defaults `${VAR:-}` are allowed."
        ),
        include_globs=("deploy/**/*.sh",),
    ),
    LintRule(
        # Dispatch-IR field names ("graph_registration", "request_hints",
        # "priority", "prefix_cohorts", "pin_release", "structured_outputs",
        # "backend_queue_state", "backend_cache_state", "cancel_groups",
        # "dispatch_ir_v1_internal", "admin_reset_prefix_cache") are
        # contract strings that must agree across the producer (the
        # runtime's `dispatch_fields_sent` collector) and the consumer
        # (`BackendGraphCapabilities::field_supported`). Hardcoding them
        # outside the constants module is exactly the drift the
        # no-legacy/no-fallback discipline guards against. Use
        # `apxm_core::constants::llm::apxm::dispatch_fields` instead.
        #
        # The pattern only matches the multi-word, contract-specific
        # names ("graph_registration", "prefix_cohorts", etc.) — bare
        # words like "priority" appear too widely in unrelated contexts
        # (LLM request bodies, plan-step priority, etc.) to be regex-safe.
        name="hardcoded-dispatch-field-literal",
        pattern=(
            r'"(graph_registration|request_hints|prefix_cohorts|pin_release|'
            r'backend_queue_state|backend_cache_state|cancel_groups|'
            r'dispatch_ir_v1_internal|admin_reset_prefix_cache)"'
        ),
        description=(
            "Dispatch-IR field name hardcoded as a string literal. "
            "Use `apxm_core::constants::llm::apxm::dispatch_fields::*` "
            "instead — the constants module is the single source of "
            "truth for producer (`dispatch_fields_sent`) and consumer "
            "(`BackendGraphCapabilities::field_supported`)."
        ),
        include_globs=("crates/**/*.rs",),
        exclude_globs=(
            # The constants module IS the source of truth.
            "crates/core/apxm-core/src/constants.rs",
            "tools/scripts/check_no_legacy_vllm.py",
        ),
    ),
)


def _git_tracked(globs: tuple[str, ...]) -> list[Path]:
    # Include tracked files AND untracked-not-ignored files so new
    # additions are linted before they are committed. -c = cached,
    # -o = others, plus the standard exclude file so .gitignore'd paths
    # stay out.
    cmd = [
        "git", "-C", str(REPO_ROOT), "ls-files",
        "-co", "--exclude-standard", "--", *globs,
    ]
    result = subprocess.run(cmd, check=False, capture_output=True, text=True)
    return [REPO_ROOT / line for line in result.stdout.splitlines() if line]


def _staged_files() -> set[Path]:
    cmd = [
        "git", "-C", str(REPO_ROOT), "diff", "--cached",
        "--name-only", "--diff-filter=ACMR", "-z",
    ]
    result = subprocess.run(cmd, check=False, capture_output=True, text=True)
    return {REPO_ROOT / p for p in result.stdout.split("\0") if p}


def _matches_any(path: Path, globs: tuple[str, ...]) -> bool:
    rel = path.relative_to(REPO_ROOT)
    return any(rel.match(g) for g in globs)


def _scan(rule: LintRule, staged: set[Path] | None) -> list[tuple[Path, int, str]]:
    regex = re.compile(rule.pattern)
    hits: list[tuple[Path, int, str]] = []
    candidates = _git_tracked(rule.include_globs)
    if staged is not None:
        candidates = [p for p in candidates if p in staged]
    for path in candidates:
        if not path.is_file():
            continue
        if rule.exclude_globs and _matches_any(path, rule.exclude_globs):
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        for line_no, line in enumerate(text.splitlines(), start=1):
            if regex.search(line):
                hits.append((path, line_no, line.rstrip()))
    return hits


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--strict",
        action="store_true",
        help="Exit non-zero on any match (CI mode). Default prints a punch list and exits 0.",
    )
    parser.add_argument(
        "--rule",
        help="Run only the named rule (for debugging).",
    )
    parser.add_argument(
        "--staged-only",
        action="store_true",
        help=(
            "Scan only files in the git index (added/modified/renamed). "
            "Pre-commit-hook mode: fast, skips files the commit doesn't touch."
        ),
    )
    args = parser.parse_args()

    staged: set[Path] | None = _staged_files() if args.staged_only else None
    if staged is not None and not staged:
        print("[skip] no staged files; nothing to scan")
        return 0

    total = 0
    for rule in RULES:
        if args.rule and rule.name != args.rule:
            continue
        hits = _scan(rule, staged)
        if not hits:
            print(f"[OK] {rule.name}: 0 matches")
            continue
        total += len(hits)
        print(f"[FAIL] {rule.name}: {len(hits)} match(es) — {rule.description}")
        for path, line_no, line in hits:
            rel = path.relative_to(REPO_ROOT)
            print(f"    {rel}:{line_no}: {line}")
    print()
    print(f"summary: {total} legacy/fallback violation(s)")
    if args.strict and total > 0:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
