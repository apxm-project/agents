#!/usr/bin/env python3
"""Deterministic smoke gate for APXM Review Council outputs."""

from __future__ import annotations

import csv
import json
import re
import sys
from pathlib import Path
from typing import Any

ARM_FILES = {
    "apxm-on": "review.apxm-on.tenants.csv",
    "flat-http": "review.flat-http.tenants.csv",
}

SECTION_NAMES = [
    "headline workflow",
    "primary metrics",
    "honest negatives",
    "next run command",
]

PATH_RE = re.compile(
    r"(?:^|[\s`(])/?(?:docs|\.apxm|tools|examples|crates|deploy|external)/"
    r"[A-Za-z0-9._/\-]+",
    re.MULTILINE,
)
BULLET_RE = re.compile(r"(?m)^\s*(?:[-*]|\d+[.)])\s+\S+")
REFUSAL_RE = re.compile(
    r"\b(?:cannot comply|can't comply|i cannot|i can't|unable to assist|"
    r"as an ai language model)\b",
    re.IGNORECASE,
)


def _parse_stdout(raw: str) -> dict[str, Any] | None:
    if not raw:
        return None
    start = raw.find("{")
    if start < 0:
        return None
    try:
        obj = json.loads(raw[start:])
    except json.JSONDecodeError:
        return None
    return obj if isinstance(obj, dict) else None


def _result_text(obj: dict[str, Any]) -> str:
    content = obj.get("content")
    if content is not None:
        return _stringify_content(content)
    results = obj.get("results")
    if isinstance(results, dict):
        values = [str(value) for _, value in sorted(results.items()) if value is not None]
        return "\n\n".join(values)
    result = obj.get("result")
    return str(result) if result is not None else ""


def _stringify_content(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "\n\n".join(_stringify_content(item) for item in value)
    if isinstance(value, dict):
        return "\n\n".join(_stringify_content(value[key]) for key in sorted(value))
    return str(value)


def _assess_row(row: dict[str, str]) -> dict[str, Any]:
    obj = _parse_stdout(row.get("stdout_tail", ""))
    text = _result_text(obj) if obj is not None else ""
    text_lower = text.lower()
    missing_sections = [name for name in SECTION_NAMES if name not in text_lower]
    paths = sorted(
        set(match.group(0).strip(" `(").lstrip("/") for match in PATH_RE.finditer(text))
    )
    bullets = BULLET_RE.findall(text)
    usage = obj.get("llm_usage", {}) if isinstance(obj, dict) else {}
    output_tokens = usage.get("output_tokens") if isinstance(usage, dict) else None
    return {
        "iteration": int(float(row.get("iteration") or 0)),
        "variant": int(float(row.get("variant") or 0)),
        "returncode": int(float(row.get("returncode") or 0)),
        "parseable_stdout": obj is not None,
        "nonempty_output": bool(text.strip()),
        "missing_sections": missing_sections,
        "path_citations": len(paths),
        "bulletish_lines": len(bullets),
        "refusal": bool(REFUSAL_RE.search(text)),
        "output_tokens": output_tokens,
        "pass": (
            int(float(row.get("returncode") or 0)) == 0
            and obj is not None
            and bool(text.strip())
            and not missing_sections
            and len(paths) >= 1
            and not REFUSAL_RE.search(text)
        ),
    }


def _assess_arm(path: Path) -> dict[str, Any]:
    rows = list(csv.DictReader(path.open()))
    checks = [_assess_row(row) for row in rows]
    failures = [check for check in checks if not check["pass"]]
    output_tokens = [
        int(check["output_tokens"])
        for check in checks
        if isinstance(check.get("output_tokens"), int)
    ]
    return {
        "path": str(path),
        "rows": len(rows),
        "passed_rows": len(checks) - len(failures),
        "failed_rows": len(failures),
        "parseable_stdout_rows": sum(1 for check in checks if check["parseable_stdout"]),
        "nonempty_output_rows": sum(1 for check in checks if check["nonempty_output"]),
        "section_complete_rows": sum(1 for check in checks if not check["missing_sections"]),
        "path_citation_rows": sum(1 for check in checks if check["path_citations"] >= 1),
        "refusal_rows": sum(1 for check in checks if check["refusal"]),
        "max_output_tokens": max(output_tokens, default=None),
        "sample_failures": failures[:10],
        "pass": not failures and bool(checks),
    }


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: assess_apxm_review_council_quality.py <run-dir>", file=sys.stderr)
        return 2
    run_dir = Path(argv[1])
    arms = {
        arm: _assess_arm(run_dir / filename)
        for arm, filename in ARM_FILES.items()
    }
    payload = {
        "run_dir": str(run_dir),
        "gate": "apxm_review_council_quality_smoke_v1",
        "arms": arms,
        "pass": all(arm["pass"] for arm in arms.values()),
    }
    print(json.dumps(payload, indent=2, sort_keys=True))
    return 0 if payload["pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
