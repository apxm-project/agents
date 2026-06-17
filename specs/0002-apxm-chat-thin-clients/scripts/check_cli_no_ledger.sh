#!/usr/bin/env bash
# The CLI may forward budget/cap inputs to apxm-server and print server ledger
# status, but it must not keep local turn/event/tool counters or enforce budget
# comparisons host-side.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPEC_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_DIR="$(cd "$SPEC_DIR/../.." && pwd)"

python3 - "$REPO_DIR" <<'PY'
from __future__ import annotations

import re
import sys
from pathlib import Path

repo = Path(sys.argv[1])
targets = [
    repo / "crates/tools/apxm-cli/src/commands/chat.rs",
    repo / "crates/tools/apxm-cli/src/commands/watch.rs",
]

checks: list[tuple[re.Pattern[str], str]] = [
    (
        re.compile(
            r"\blet\s+mut\s+"
            r"(?:(?:turn|turns|event|events|tool|tools)_"
            r"(?:count|seen|used|spent|remaining|budget|cap|limit)"
            r"|(?:remaining|used|spent)_(?:turn|turns|event|events|tool|tools))"
            r"[A-Za-z0-9_]*\s*(?::|=)"
        ),
        "local mutable turn/event/tool counter",
    ),
    (
        re.compile(r"\b(?:turn|event|tool)[A-Za-z0-9_]*\s*\+=\s*1\b"),
        "incrementing a local turn/event/tool counter",
    ),
    (
        re.compile(r"\b(?:turn|event|tool)[A-Za-z0-9_]*\s*=\s*(?:\w+\.)?(?:turn|event|tool)[A-Za-z0-9_]*\s*\+\s*1\b"),
        "assigning an incremented local turn/event/tool counter",
    ),
    (
        re.compile(r"\b(?:remaining|used|spent)_(?:turn|turns|event|events|tool|tools|budget|budgets)[A-Za-z0-9_]*\b"),
        "local remaining/used/spent budget state",
    ),
    (
        re.compile(r"\b(?:turns_remaining|events_remaining|budget_remaining|tool_calls_used|tool_calls_remaining)\b"),
        "local ledger-style budget state",
    ),
    (
        re.compile(r"\bif\s+[^;\n]*(?:max_turns|max_events)[^;\n]*(?:>=|>|==|<=|<)"),
        "host-side max_turns/max_events enforcement",
    ),
    (
        re.compile(r"\bwhile\s+[^;\n]*(?:max_turns|max_events)\b"),
        "host-side max_turns/max_events loop enforcement",
    ),
]

failures: list[str] = []
for path in targets:
    text = path.read_text(encoding="utf-8")
    rel = path.relative_to(repo)
    for lineno, line in enumerate(text.splitlines(), start=1):
        stripped = line.strip()
        if stripped.startswith("//"):
            continue
        for pattern, label in checks:
            if pattern.search(line):
                failures.append(f"{rel}:{lineno}: {label}: {stripped}")

if failures:
    print("check_cli_no_ledger: FAIL", file=sys.stderr)
    for failure in failures:
        print(failure, file=sys.stderr)
    sys.exit(1)

print("check_cli_no_ledger: OK (CLI has no host-side ledger counters)")
PY
