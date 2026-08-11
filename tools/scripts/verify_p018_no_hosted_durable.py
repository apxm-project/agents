#!/usr/bin/env python3
"""Verify the checked-in P-018 no-build dossier and evidence stay exact."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tomllib
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
REPORT_SCHEMA = "apxm.p018-no-hosted-durable-verifier.v2"
ISSUE_NUMBER = 39
DIGEST_LINE = re.compile(r"- `(?P<path>[^`]+)` — `(?P<digest>sha256:[0-9a-f]{64})`")
STALE_DECISION_PHRASES = (
    "After this branch merges",
    "PR #41",
    "now published in default-branch authority",
    "no longer unpublished",
)
P018_EVIDENCE_ARTIFACTS = (
    Path("docs/plans/p018-hosted-durable-decision.md"),
    Path("docs/plans/g3-admission-runtime-checklist.md"),
    Path("docs/evidence/p018-g3-no-build.md"),
)
P018_OWNER_SCOPE = Path("crates/runtime/commit-local")
OWNER_SCOPE_SUFFIXES = {".rs", ".toml"}
OWNER_DEPENDENCY_SECTIONS = ("dependencies", "dev-dependencies", "build-dependencies")
ALLOWED_OWNER_DEPENDENCIES = {
    "apxm-kernel",
    "apxm-program",
    "async-trait",
    "fs2",
    "serde",
    "serde_json",
    "sha2",
    "tempfile",
    "thiserror",
    "tokio",
}
ALLOWED_WORKSPACE_DEPENDENCY_KEYS = {"workspace", "features", "default-features", "optional"}
EVIDENCE_PRODUCT_PATTERN = re.compile(
    r"\b(?:downstream[-_](?:product|sdk|dependency)|"
    r"(?:product|vendor|customer)[-_](?:sdk|client|service|dependency))\b",
    re.I,
)
DOWNSTREAM_DEPENDENCY_PATTERN = re.compile(
    r"\b(?:downstream|vendor|customer)[-_][a-z0-9_-]+\b",
    re.I,
)

# These patterns are evaluated against production Rust after comments are removed.
# Decision prose can describe rejected capabilities without creating a capability.
OWNER_FORBIDDEN_PATTERNS = {
    "hosted-or-service": re.compile(
        r"\b(?:hosted|service|server)\b|\bHosted[A-Z][A-Za-z0-9_]*\b|"
        r"Hosted(?:Durable)?(?:Checkpoint|Output)(?:Service)?|"
        r"CheckpointHost(?:ed)?Service",
        re.I,
    ),
    "network-or-remote": re.compile(
        r"(?:\b(?:TcpListener|UdpSocket|TcpStream|UdpStream|SocketAddr|std::net|tokio::net|"
        r"axum|hyper|reqwest|tonic|sqlx|redis|postgres|mysql|listen|bind|endpoint|remote|"
        r"replicat(?:e|ion)|networked)\b|https?://)",
        re.I,
    ),
    "multi-tenant": re.compile(
        r"\b(?:multi[-_ ]tenant|tenant(?:[-_ ]id)?)\b|MultiTenant", re.I
    ),
    "downstream-or-product": re.compile(
        r"\b(?:downstream|vendor|customer)[-_][a-z0-9_-]+\b",
        re.I,
    ),
    "alias": re.compile(
        r"\b(?:alias|aliases|alias_to|retired_alias)\b|(?s:\buse\b[^;]*\bas\s+[A-Za-z_])|"
        r"\b[A-Za-z_][A-Za-z0-9_]*Alias[A-Za-z0-9_]*\b"
    ),
    "fallback-or-speculative": re.compile(
        r"\b(?:fallback|fallbacks|speculative|experimental)\b|"
        r"\b[A-Za-z_][A-Za-z0-9_]*(?:Fallback|Speculative)[A-Za-z0-9_]*\b",
        re.I,
    ),
    "feature-flag": re.compile(
        r"\bcfg\s*\(\s*feature\b|\bfeature[_ -]?flag\b|FeatureFlag", re.I
    ),
    "placeholder": re.compile(
        r"\b(?:placeholder|TODO|unimplemented!)\b|\b[A-Za-z_][A-Za-z0-9_]*Placeholder[A-Za-z0-9_]*\b",
        re.I,
    ),
    "second-writer": re.compile(
        r"\b(?:second|secondary|multi)[-_ ]writer\b|\b(?:try_lock_shared|lock_shared|shared_lock)\b|"
        r"(?:Second|Secondary|Multi)Writer",
        re.I,
    ),
}


def strip_rust_comments(text: str) -> str:
    """Remove Rust comments so negative boundary prose is not treated as code."""

    return re.sub(r"//[^\n]*|/\*.*?\*/", "", text, flags=re.S)


def _finding(path: Path, category: str, detail: str, root: Path) -> dict[str, str]:
    return {
        "path": path.relative_to(root).as_posix(),
        "category": category,
        "detail": detail,
    }


def _text_findings(
    path: Path, text: str, root: Path, *, strip_comments: bool = False
) -> list[dict[str, str]]:
    scanned = strip_rust_comments(text) if strip_comments else text
    findings: list[dict[str, str]] = []
    for category, pattern in OWNER_FORBIDDEN_PATTERNS.items():
        match = pattern.search(scanned)
        if match is not None:
            findings.append(_finding(path, category, f"forbidden token `{match.group(0)}`", root))
    return findings


@dataclass(frozen=True)
class CheckResult:
    name: str
    status: str
    detail: str


def owner_scope_findings(root: Path = REPO_ROOT) -> list[dict[str, str]]:
    """Return absence findings for every file in the bounded P-018 owner scope."""

    scope = root / P018_OWNER_SCOPE
    if not scope.is_dir():
        return [_finding(scope, "owner-scope", "owner scope is missing", root)]

    findings: list[dict[str, str]] = []
    scope_files = sorted(path for path in scope.rglob("*") if path.is_file())
    for path in scope_files:
        if path.suffix not in OWNER_SCOPE_SUFFIXES:
            findings.append(
                _finding(
                    path,
                    "owner-scope",
                    f"file type `{path.suffix or '<none>'}` is not covered by the authoritative scan",
                    root,
                )
            )
            continue
        if path.suffix == ".rs":
            findings.extend(
                _text_findings(path, path.read_text(encoding="utf-8"), root, strip_comments=True)
            )

    manifest_path = scope / "Cargo.toml"
    try:
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        return findings + [_finding(manifest_path, "manifest", str(error), root)]

    unexpected_sections = sorted(
        key
        for key in manifest
        if key not in {"package", "lints", *OWNER_DEPENDENCY_SECTIONS}
    )
    for section in unexpected_sections:
        findings.append(_finding(manifest_path, "manifest", f"unexpected manifest section `{section}`", root))
    if "features" in manifest:
        findings.append(_finding(manifest_path, "fallback-or-speculative", "crate feature surface is present", root))

    for section in OWNER_DEPENDENCY_SECTIONS:
        dependencies = manifest.get(section, {})
        if not isinstance(dependencies, dict):
            findings.append(_finding(manifest_path, "manifest", f"`{section}` is not a table", root))
            continue
        for name, spec in dependencies.items():
            if name not in ALLOWED_OWNER_DEPENDENCIES:
                category = (
                    "downstream-or-product"
                    if DOWNSTREAM_DEPENDENCY_PATTERN.search(name)
                    else "manifest"
                )
                findings.append(_finding(manifest_path, category, f"unapproved dependency `{name}`", root))
            if not isinstance(spec, dict):
                continue
            if "package" in spec:
                findings.append(_finding(manifest_path, "alias", f"dependency alias `{name}`", root))
            unexpected_keys = set(spec) - ALLOWED_WORKSPACE_DEPENDENCY_KEYS
            for key in sorted(unexpected_keys):
                category = "network-or-remote" if key in {"git", "registry", "path"} else "manifest"
                findings.append(_finding(manifest_path, category, f"unapproved dependency key `{key}`", root))
    return findings


def owner_absence_checks(root: Path = REPO_ROOT) -> list[CheckResult]:
    """Run the authoritative bounded absence check for the P-018 owner scope."""

    findings = owner_scope_findings(root)
    checks: list[CheckResult] = []
    categories = (*OWNER_FORBIDDEN_PATTERNS.keys(), "manifest", "owner-scope")
    for category in categories:
        matches = [finding for finding in findings if finding["category"] == category]
        checks.append(
            CheckResult(
                name=f"owner-scope:absence:{category}",
                status="failed" if matches else "passed",
                detail=(
                    "forbidden owner-scope surface: "
                    + "; ".join(f"{item['path']}: {item['detail']}" for item in matches)
                    if matches
                    else "no forbidden owner-scope surface found"
                ),
            )
        )

    filesystem_path = root / P018_OWNER_SCOPE / "src" / "filesystem.rs"
    lock_text = ""
    if filesystem_path.is_file():
        lock_text = strip_rust_comments(filesystem_path.read_text(encoding="utf-8"))
    lock_ok = "try_lock_exclusive" in lock_text and not re.search(
        r"\b(?:try_lock_shared|lock_shared|shared_lock)\b", lock_text
    )
    checks.append(
        CheckResult(
            name="owner-scope:single-writer-lock",
            status="passed" if lock_ok else "failed",
            detail="filesystem adapter has one exclusive owner lock and no shared lock"
            if lock_ok
            else "filesystem adapter must prove one exclusive owner lock",
        )
    )
    return checks


def file_digest(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def render_path(path: Path) -> str:
    try:
        return str(path.relative_to(REPO_ROOT))
    except ValueError:
        return str(path)


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
    evidence_text = EVIDENCE_PATH.read_text(encoding="utf-8")

    product_hits: list[str] = []
    for path in P018_EVIDENCE_ARTIFACTS:
        artifact_path = REPO_ROOT / path
        for match in EVIDENCE_PRODUCT_PATTERN.finditer(artifact_path.read_text(encoding="utf-8")):
            product_hits.append(f"{path}: {match.group(0)}")

    return [
        CheckResult(
            name="decision:no-build-status",
            status="passed"
            if "Status: **no-build** (publication pending merge)" in decision_text
            else "failed",
            detail="decision dossier records no-build status as a pre-merge publication",
        ),
        CheckResult(
            name="decision:no-hosted-service-boundary",
            status="passed"
            if "APXM does **not** ship a hosted" in decision_text
            else "failed",
            detail="decision dossier rejects hosted service, placeholder, and feature flag",
        ),
        CheckResult(
            name="decision:pre-merge-authority-boundary",
            status="passed"
            if re.search(
                r"publication becomes authoritative\s+on `main` only after this PR is merged",
                decision_text,
            )
            and not any(phrase in decision_text for phrase in STALE_DECISION_PHRASES)
            else "failed",
            detail="decision dossier distinguishes this PR publication from post-merge authority",
        ),
        CheckResult(
            name="checklist:post-merge-authority-target",
            status="passed"
            if "Post-merge target: **Published no-build decision on `main`**" in checklist_text
            and "G3 gate blocker remains open" not in checklist_text
            else "failed",
            detail="G3 checklist preserves the post-merge main publication target",
        ),
        CheckResult(
            name="evidence:post-merge-reverification",
            status="passed"
            if re.search(
                r"root coordinator reruns this verifier against\s+the merged Agents `main` ref",
                evidence_text,
            )
            and "issue #39 remains open" in evidence_text
            else "failed",
            detail="evidence requires post-merge verification against real main and an open issue",
        ),
        CheckResult(
            name="evidence:product-neutral",
            status="failed" if product_hits else "passed",
            detail=(
                "downstream product reference in P-018 evidence: " + "; ".join(product_hits)
                if product_hits
                else "P-018 evidence artifacts contain no downstream product names"
            ),
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
    checks.extend(owner_absence_checks())
    overall_status = "passed" if all(check.status == "passed" for check in checks) else "failed"
    return {
        "schema_version": REPORT_SCHEMA,
        "issue": ISSUE_NUMBER,
        "decision": "no-build",
        "overall_status": overall_status,
        "checks": [asdict(check) for check in checks],
        "digests": digest_rows,
        "scopes": {
            "owner": P018_OWNER_SCOPE.as_posix(),
            "evidence": [path.as_posix() for path in P018_EVIDENCE_ARTIFACTS],
        },
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
