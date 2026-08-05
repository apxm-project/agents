#!/usr/bin/env python3
"""Verify the checked-in P-018 no-build dossier and evidence stay exact."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from dataclasses import asdict, dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
EVIDENCE_PATH = REPO_ROOT / "docs" / "evidence" / "p018-g3-no-build.md"
DECISION_PATH = REPO_ROOT / "docs" / "plans" / "p018-hosted-durable-decision.md"
CHECKLIST_PATH = REPO_ROOT / "docs" / "plans" / "g3-admission-runtime-checklist.md"
COMMIT_LOCAL_MANIFEST = REPO_ROOT / "crates" / "runtime" / "commit-local" / "Cargo.toml"
DEFAULT_REPORT = (
    REPO_ROOT / ".apxm" / "evidence" / "p018-no-hosted-durable" / "report.json"
)
REPORT_SCHEMA = "apxm.p018-no-hosted-durable-verifier.v1"
ISSUE_NUMBER = 39
DIGEST_LINE = re.compile(r"- `(?P<path>[^`]+)` — `(?P<digest>sha256:[0-9a-f]{64})`")
STALE_DECISION_PHRASES = ("After this branch merges", "PR #41")


@dataclass(frozen=True)
class CheckResult:
    name: str
    status: str
    detail: str


def file_digest(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def render_path(path: Path) -> str:
    return str(path.relative_to(REPO_ROOT))


def parse_digest_manifest() -> dict[str, str]:
    evidence_text = EVIDENCE_PATH.read_text(encoding="utf-8")
    if "## Evidence digests" not in evidence_text:
        raise ValueError("docs/evidence/p018-g3-no-build.md is missing `## Evidence digests`")
    manifest: dict[str, str] = {}
    in_section = False
    for line in evidence_text.splitlines():
        if line.startswith("## "):
            in_section = line == "## Evidence digests"
            continue
        if not in_section:
            continue
        match = DIGEST_LINE.fullmatch(line.strip())
        if match is None:
            continue
        manifest[match.group("path")] = match.group("digest")
    if not manifest:
        raise ValueError("P-018 evidence digest manifest is empty")
    return manifest


def digest_checks() -> tuple[list[dict[str, str]], list[CheckResult]]:
    manifest = parse_digest_manifest()
    rows: list[dict[str, str]] = []
    checks: list[CheckResult] = []
    for relative_path in sorted(manifest):
        path = REPO_ROOT / relative_path
        if not path.is_file():
            checks.append(
                CheckResult(
                    name=f"digest:{relative_path}",
                    status="failed",
                    detail="artifact is missing",
                )
            )
            rows.append(
                {
                    "path": relative_path,
                    "expected_digest": manifest[relative_path],
                    "observed_digest": "missing",
                    "status": "failed",
                }
            )
            continue
        observed = file_digest(path)
        status = "passed" if observed == manifest[relative_path] else "failed"
        detail = "digest matches checked-in evidence" if status == "passed" else "digest drift"
        rows.append(
            {
                "path": relative_path,
                "expected_digest": manifest[relative_path],
                "observed_digest": observed,
                "status": status,
            }
        )
        checks.append(CheckResult(name=f"digest:{relative_path}", status=status, detail=detail))
    return rows, checks


def evidence_phrase_checks() -> list[CheckResult]:
    decision_text = DECISION_PATH.read_text(encoding="utf-8")
    checklist_text = CHECKLIST_PATH.read_text(encoding="utf-8")
    manifest_text = COMMIT_LOCAL_MANIFEST.read_text(encoding="utf-8")

    return [
        CheckResult(
            name="decision:no-build-status",
            status="passed" if "Status: **no-build**" in decision_text else "failed",
            detail="decision dossier records explicit no-build status",
        ),
        CheckResult(
            name="decision:no-hosted-service-boundary",
            status="passed"
            if "APXM does **not** ship a hosted" in decision_text
            else "failed",
            detail="decision dossier rejects hosted service, placeholder, and feature flag",
        ),
        CheckResult(
            name="decision:no-stale-branch-language",
            status="passed"
            if not any(phrase in decision_text for phrase in STALE_DECISION_PHRASES)
            else "failed",
            detail="decision dossier has no branch-local or stale PR dependency language",
        ),
        CheckResult(
            name="checklist:published-authority",
            status="passed"
            if "Published no-build decision on `main`" in checklist_text
            and "G3 gate blocker remains open" not in checklist_text
            else "failed",
            detail="G3 checklist points at published default-branch authority without restating #39 as an unpublished blocker",
        ),
        CheckResult(
            name="crate:no-hosted-service-manifest",
            status="passed"
            if "not a hosted durable service" in manifest_text
            else "failed",
            detail="owner-local crate manifest preserves the no-hosted-service boundary",
        ),
    ]


def build_report() -> dict[str, object]:
    digest_rows, checks = digest_checks()
    checks.extend(evidence_phrase_checks())
    overall_status = "passed" if all(check.status == "passed" for check in checks) else "failed"
    return {
        "schema_version": REPORT_SCHEMA,
        "issue": ISSUE_NUMBER,
        "decision": "no-build",
        "overall_status": overall_status,
        "checks": [asdict(check) for check in checks],
        "digests": digest_rows,
    }


def write_report(path: Path, report: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--report-json",
        type=Path,
        default=DEFAULT_REPORT,
        help="write the verifier report to this JSON file",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    report_path = args.report_json
    if not report_path.is_absolute():
        report_path = REPO_ROOT / report_path

    report = build_report()
    write_report(report_path, report)
    print(json.dumps(report, indent=2, sort_keys=True))
    print(f"\nWrote report to {render_path(report_path)}", flush=True)
    if report["overall_status"] != "passed":
        print("FAILED: P-018 no-build evidence drifted", file=sys.stderr)
        return 1
    print("OK: P-018 no-build evidence is exact", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
