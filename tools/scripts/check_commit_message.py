#!/usr/bin/env python3
"""APXM commit-message lint.

Enforces `.agents/skills/_shared/apxm-commit-message-rules.md`:

- subject: `<type>(<scope>): <subject>`, ≤72 chars, imperative, no emoji.
- type ∈ ALLOWED_TYPES; deprecated spellings map to a suggestion.
- scope = subsystem; `planNN` only allowed for `prereg`/`eval`.
- body: no AI-attribution lines, no referential phrasing.

Modes:
  python3 check_commit_message.py <FILE>           # message file mode
  python3 check_commit_message.py --current        # HEAD's message
  python3 check_commit_message.py --range A..B     # every commit in range

Exit 0 = clean. Exit 1 = at least one rule failed.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass


ALLOWED_TYPES: frozenset[str] = frozenset({
    "feat", "fix", "perf", "refactor", "docs", "test",
    "chore", "bench", "eval", "prereg", "sec", "style",
})

DEPRECATED_TYPES: dict[str, str] = {
    "pre-reg": "prereg",
    "preregister": "prereg",
    "preregistration": "prereg",
}

PLAN_SCOPE_TYPES: frozenset[str] = frozenset({"prereg", "eval"})
PLAN_SCOPE_RE = re.compile(r"^plan-?\d{1,3}$")
SCOPE_SHAPE_RE = re.compile(r"^[A-Za-z][A-Za-z0-9_/\-]*$")

RESERVED_SCOPE_NAMES: frozenset[str] = frozenset({
    "ultrathink", "claude", "codex", "cursor", "aider", "copilot",
})

RESERVED_SUBJECT_WORDS: frozenset[str] = frozenset({
    "wip", "tmp", "misc", "tweaks",
})

EMOJI_RE = re.compile(
    r"["
    r"\U0001F300-\U0001FAFF"
    r"\U00002600-\U000027BF"
    r"\U0001F000-\U0001F02F"
    r"\U0001F100-\U0001F1FF"
    r"]"
)

SUBJECT_RE = re.compile(
    r"^(?P<type>[a-zA-Z-]+)"
    r"(?:\((?P<scope>[A-Za-z0-9_/\-]+)\))?"
    r":\s+(?P<subject>.+)$"
)

AI_ATTRIBUTION_PATTERNS: tuple[re.Pattern[str], ...] = (
    re.compile(r"^\s*Co-[Aa]uthored-[Bb]y:\s*Claude\b", re.MULTILINE),
    re.compile(r"Generated with \[?Claude Code\]?", re.IGNORECASE),
    re.compile(r"^\s*🤖", re.MULTILINE),
)

REFERENTIAL_PHRASES: tuple[str, ...] = (
    "as discussed",
    "per the chat",
    "per the user",
    "claude said",
    "claude told me",
)

MERGE_PREFIXES: tuple[str, ...] = ("Merge ", "Revert ")


@dataclass(frozen=True)
class Finding:
    rule: str
    detail: str

    def format(self, sha: str | None = None) -> str:
        prefix = f"{sha[:8]} " if sha else ""
        return f"{prefix}[{self.rule}] {self.detail}"


def _strip_comments(message: str) -> str:
    # Same cleanup as git commit messages: lines starting with '#'
    # are stripped before the message is recorded.
    return "\n".join(
        line for line in message.splitlines() if not line.startswith("#")
    ).strip("\n")


def lint_message(message: str) -> list[Finding]:
    findings: list[Finding] = []
    text = _strip_comments(message)
    if not text.strip():
        return [Finding("empty-message", "commit message is empty")]

    lines = text.splitlines()
    subject = lines[0]

    if any(subject.startswith(p) for p in MERGE_PREFIXES):
        return []

    findings.extend(_lint_subject(subject))
    body = "\n".join(lines[1:]) if len(lines) > 1 else ""
    if body.strip():
        if len(lines) >= 2 and lines[1].strip():
            findings.append(Finding(
                "missing-body-separator",
                "body must be separated from subject by a blank line",
            ))
        findings.extend(_lint_body(body))
    return findings


def _lint_subject(subject: str) -> list[Finding]:
    findings: list[Finding] = []
    if len(subject) > 72:
        findings.append(Finding(
            "subject-too-long",
            f"subject is {len(subject)} chars (max 72)",
        ))
    if subject.rstrip().endswith("."):
        findings.append(Finding(
            "subject-trailing-period",
            "subject must not end with '.'",
        ))
    if EMOJI_RE.search(subject):
        findings.append(Finding("subject-emoji", "emoji not allowed in subject"))

    head = subject.split(":", 1)[0] if ":" in subject else subject
    head_lower = head.lower()
    for deprecated, replacement in DEPRECATED_TYPES.items():
        if head_lower == deprecated or head_lower.startswith(f"{deprecated}("):
            findings.append(Finding(
                "deprecated-type",
                f"use `{replacement}(...)` instead of `{deprecated}`",
            ))

    match = SUBJECT_RE.match(subject)
    if not match:
        findings.append(Finding(
            "subject-shape",
            "expected `<type>(<scope>): <subject>`; got: " + subject,
        ))
        return findings

    ctype = match.group("type")
    scope = match.group("scope")
    body_text = match.group("subject").strip()

    if ctype not in ALLOWED_TYPES:
        suggestion = DEPRECATED_TYPES.get(ctype.lower())
        hint = f" (did you mean `{suggestion}`?)" if suggestion else ""
        findings.append(Finding(
            "unknown-type",
            f"type `{ctype}` not in {sorted(ALLOWED_TYPES)}{hint}",
        ))

    if scope:
        scope_lower = scope.lower()
        if PLAN_SCOPE_RE.match(scope_lower):
            if ctype not in PLAN_SCOPE_TYPES:
                findings.append(Finding(
                    "plan-scope-misuse",
                    f"`planNN` scope only valid for {sorted(PLAN_SCOPE_TYPES)}; "
                    f"got `{ctype}({scope})`. Scope by subsystem instead.",
                ))
            if "-" in scope:
                findings.append(Finding(
                    "plan-scope-spelling",
                    f"use `plan{scope.split('-', 1)[1]}` (no hyphen) instead of `{scope}`",
                ))
        elif scope_lower in RESERVED_SCOPE_NAMES:
            findings.append(Finding(
                "reserved-scope",
                f"scope `{scope}` names a tool/agent, not a subsystem; rename",
            ))
        elif not SCOPE_SHAPE_RE.match(scope):
            findings.append(Finding(
                "scope-shape",
                f"scope `{scope}` must match {SCOPE_SHAPE_RE.pattern}",
            ))
    elif ctype in PLAN_SCOPE_TYPES:
        findings.append(Finding(
            "missing-plan-scope",
            f"`{ctype}` requires a `(planNN)` scope (e.g. `{ctype}(plan04): ...`)",
        ))

    body_lower = body_text.lower()
    for word in RESERVED_SUBJECT_WORDS:
        if re.search(rf"\b{re.escape(word)}\b", body_lower):
            findings.append(Finding(
                "reserved-subject-word",
                f"reserved word `{word}` in subject",
            ))
    return findings


def _lint_body(body: str) -> list[Finding]:
    findings: list[Finding] = []
    for pat in AI_ATTRIBUTION_PATTERNS:
        if pat.search(body):
            findings.append(Finding(
                "ai-attribution",
                f"AI-attribution line not allowed in body: matched `{pat.pattern}`",
            ))
    body_lower = body.lower()
    for phrase in REFERENTIAL_PHRASES:
        if phrase in body_lower:
            findings.append(Finding(
                "referential-phrase",
                f"referential phrase `{phrase}` belongs in the PR description, not the commit",
            ))
    return findings


def _run_git(*args: str) -> str:
    return subprocess.run(
        ["git", *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout


def _messages_in_range(rev_range: str) -> list[tuple[str, str]]:
    shas = _run_git("rev-list", "--no-merges", rev_range).split()
    out: list[tuple[str, str]] = []
    for sha in shas:
        msg = _run_git("log", "-1", "--format=%B", sha)
        out.append((sha, msg))
    return out


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    src = parser.add_mutually_exclusive_group()
    src.add_argument("file", nargs="?", help="path to commit message file")
    src.add_argument("--current", action="store_true", help="lint HEAD's message")
    src.add_argument("--range", dest="rev_range", help="lint commits in REV1..REV2")
    args = parser.parse_args(argv)

    failed = False

    if args.rev_range:
        for sha, msg in _messages_in_range(args.rev_range):
            for f in lint_message(msg):
                print(f.format(sha), file=sys.stderr)
                failed = True
    elif args.current:
        msg = _run_git("log", "-1", "--format=%B", "HEAD")
        for f in lint_message(msg):
            print(f.format(), file=sys.stderr)
            failed = True
    elif args.file:
        msg = open(args.file, encoding="utf-8").read()
        for f in lint_message(msg):
            print(f.format(), file=sys.stderr)
            failed = True
    else:
        parser.print_help(sys.stderr)
        return 2

    if failed:
        print(
            "\nSee .agents/skills/_shared/apxm-commit-message-rules.md",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
