#!/usr/bin/env python3
"""review.py — draft a PR review by running the project's checklist + LLM.

Reads a PR diff via `gh pr diff`, runs three project-specific checks
(no-legacy-vllm reference scan, CHANGELOG [Unreleased] touched-when-
public-surface-changes, claim-discipline when docs/claims/* is modified),
and sends the diff + check results through the HAL adapter to draft a
review comment. Human-reviewer agreement rate on the draft's findings
(accept / modify / reject) is the headline metric.

Pure stdlib + gh + urllib.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import textwrap
from datetime import datetime, timezone
from pathlib import Path
from urllib import error as urllib_error
from urllib import request as urllib_request

REPO_ROOT = Path(__file__).resolve().parents[2]

_TOOLS_SCRIPT_DIR = str(REPO_ROOT / "tools" / "scripts")
if _TOOLS_SCRIPT_DIR not in sys.path:
    sys.path.insert(0, _TOOLS_SCRIPT_DIR)

from apxm_vllm_contract import WireKey  # noqa: E402
MANIFEST_DIR = REPO_ROOT / ".apxm" / "evaluation" / "agentic" / "dogfood"

REVIEW_SYSTEM_PROMPT = textwrap.dedent("""
    You are APXM's PR review assistant. Read the diff and the project
    checklist results below. Draft a SHORT review comment that:
    1. Acknowledges what the PR does in one sentence.
    2. Flags ANY checklist item the diff missed, citing the file/line
       where applicable.
    3. Asks at most ONE focused question about an unclear design choice.

    No praise, no preamble. Output Markdown. Be terse — the human
    reviewer is busy.
""").strip()


def _gh(*args: str) -> str:
    proc = subprocess.run(["gh", *args], check=True, capture_output=True, text=True)
    return proc.stdout


def _pr_diff(pr_ref: str, repo: str | None) -> str:
    cmd = ["pr", "diff", pr_ref]
    if repo:
        cmd.extend(["--repo", repo])
    try:
        return _gh(*cmd)
    except subprocess.CalledProcessError as exc:
        print(f"[review] gh pr diff failed: {exc.stderr.strip()}", file=sys.stderr)
        return ""


def _pr_metadata(pr_ref: str, repo: str | None) -> dict:
    cmd = [
        "pr", "view", pr_ref,
        "--json", "number,title,body,author,additions,deletions,changedFiles,files",
    ]
    if repo:
        cmd.extend(["--repo", repo])
    try:
        return json.loads(_gh(*cmd))
    except (subprocess.CalledProcessError, json.JSONDecodeError) as exc:
        print(f"[review] gh pr view failed: {exc}", file=sys.stderr)
        return {}


# ──────────────────────────────────────────────────────────────────────────────
# Project-specific checklist
# ──────────────────────────────────────────────────────────────────────────────


def _check_no_legacy_vllm(touched_files: list[str]) -> tuple[bool, str]:
    """Did any touched file end up in a region the no-legacy-vllm lint
    cares about (crates/ or tools/)? If yes, the PR description should
    confirm `check_no_legacy_vllm.py` was rerun. We can't enforce
    that here, but the LLM can ask."""
    relevant = [
        p for p in touched_files
        if p.startswith("crates/") or p.startswith("tools/") or p.startswith("deploy/")
    ]
    if not relevant:
        return (True, "no-legacy-vllm: no files in lint scope.")
    return (
        True,
        f"no-legacy-vllm: PR touches {len(relevant)} file(s) in lint scope. "
        f"Confirm `python3 tools/scripts/check_no_legacy_vllm.py` is clean.",
    )


def _check_changelog_touched(touched_files: list[str]) -> tuple[bool, str]:
    """If the PR touches public surface (crates root, dekk CLI, docs),
    CHANGELOG.md [Unreleased] should be updated. Heuristic only."""
    public_paths = [
        p for p in touched_files
        if p.startswith("crates/")
        or p.startswith("tools/scripts/")
        or p.startswith("docs/")
        or p == "README.md"
        or p == "CHANGELOG.md"
    ]
    changelog_touched = "CHANGELOG.md" in touched_files
    if not public_paths:
        return (True, "changelog: no public surface touched; CHANGELOG not required.")
    if changelog_touched:
        return (True, "changelog: CHANGELOG.md updated alongside public surface changes.")
    return (
        False,
        f"changelog: PR touches {len(public_paths)} public-surface file(s) "
        "but CHANGELOG.md is NOT updated. Add a `[Unreleased]` entry.",
    )


def _check_claim_discipline(touched_files: list[str], pr_body: str) -> tuple[bool, str]:
    """When the PR touches docs/claims/* or .apxm/docs/claims/*, the
    description should cite the pre-registration that earned the claim."""
    claim_paths = [
        p for p in touched_files
        if "/claims/" in p or p.startswith(".apxm/docs/claims/")
    ]
    if not claim_paths:
        return (True, "claim-discipline: no claim files touched.")
    has_preregistration_ref = bool(
        re.search(r"docs/preregistrations/", pr_body or "", re.IGNORECASE)
    )
    if has_preregistration_ref:
        return (
            True,
            f"claim-discipline: PR touches {len(claim_paths)} claim file(s) "
            "AND cites a pre-registration. Good.",
        )
    return (
        False,
        f"claim-discipline: PR touches {len(claim_paths)} claim file(s) "
        "but the description does NOT cite docs/preregistrations/*. Per "
        "Every claim must reference a pre-committed measurement protocol.",
    )


def _run_checks(metadata: dict) -> list[tuple[bool, str]]:
    """Run the project-specific checks; return (ok, summary) list."""
    files = [f.get("path", "") for f in metadata.get("files", [])]
    body = metadata.get("body", "") or ""
    return [
        _check_no_legacy_vllm(files),
        _check_changelog_touched(files),
        _check_claim_discipline(files, body),
    ]


def _draft_via_hal(
    hal_endpoint: str,
    model: str,
    metadata: dict,
    diff: str,
    checks: list[tuple[bool, str]],
    *,
    timeout_s: float = 90.0,
) -> tuple[str, str]:
    """Send diff + checklist results to the HAL adapter; return
    (draft_markdown, fields_honored_header)."""
    # Cap diff at 24 KB — long PRs would otherwise push past max_model_len.
    diff_cap = diff[:24000]
    title = metadata.get("title", "(no title)")
    additions = metadata.get("additions", 0)
    deletions = metadata.get("deletions", 0)
    checklist_block = "\n".join(
        f"- {'✓' if ok else '✗'} {msg}" for ok, msg in checks
    )
    user_prompt = (
        f"PR title: {title}\n"
        f"PR size: +{additions}/-{deletions} lines\n\n"
        f"## Project checklist results\n{checklist_block}\n\n"
        f"## Diff (truncated to 24 KB)\n```diff\n{diff_cap}\n```"
    )

    payload = {
        "model": model,
        "messages": [
            {"role": "system", "content": REVIEW_SYSTEM_PROMPT},
            {"role": "user", "content": user_prompt},
        ],
        "max_tokens": 512,
        "vllm_xargs": {
            "apxm": {
                WireKey.REUSE_GROUP.value: "dogfood-review-cohort",
                WireKey.PIN_POLICY.value: {WireKey.PIN_MODE.value: "prefix", WireKey.PIN_TTL_MS.value: 30000},
            }
        },
    }
    data = json.dumps(payload).encode("utf-8")
    req = urllib_request.Request(
        f"{hal_endpoint.rstrip('/')}/v1/chat/completions",
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib_request.urlopen(req, timeout=timeout_s) as resp:
            body = resp.read()
            honored = resp.headers.get("x-apxm-fields-honored", "")
    except (urllib_error.URLError, TimeoutError) as exc:
        return (f"_(hal call failed: {exc})_", "")
    try:
        envelope = json.loads(body)
        content = (
            envelope.get("choices", [{}])[0].get("message", {}).get("content", "")
            or envelope.get("content", "")
        )
    except (json.JSONDecodeError, IndexError, KeyError):
        content = ""
    return (content or "_(hal returned empty content)_", honored)


def _render_review(metadata: dict, checks: list[tuple[bool, str]], draft: str) -> str:
    title = metadata.get("title", "(no title)")
    num = metadata.get("number", "?")
    lines = [f"# Draft review for PR #{num}: {title}", ""]
    lines.append("## Project checklist")
    for ok, msg in checks:
        lines.append(f"- {'✓' if ok else '✗'} {msg}")
    lines.append("")
    lines.append("## LLM-drafted comment")
    lines.append(draft)
    return "\n".join(lines).rstrip() + "\n"


def _write_manifest(
    *,
    timestamp: datetime,
    pr_ref: str,
    metadata: dict,
    checks: list[tuple[bool, str]],
    draft_path: Path,
    hal_endpoint: str,
    model: str,
    fields_honored: str,
) -> Path:
    MANIFEST_DIR.mkdir(parents=True, exist_ok=True)
    run_dir = MANIFEST_DIR / timestamp.strftime("%Y%m%dT%H%M%SZ")
    run_dir.mkdir(parents=True, exist_ok=True)
    manifest = {
        "command": "review",
        "started_at": timestamp.isoformat(),
        "pr_ref": pr_ref,
        "pr_number": metadata.get("number"),
        "pr_title": metadata.get("title"),
        "additions": metadata.get("additions", 0),
        "deletions": metadata.get("deletions", 0),
        "files_changed": metadata.get("changedFiles", 0),
        "checks": [{"ok": ok, "msg": msg} for ok, msg in checks],
        "fields_honored": [f for f in fields_honored.split(",") if f] if fields_honored else [],
        "hal_endpoint": hal_endpoint,
        "model": model,
        "draft_path": str(draft_path.relative_to(REPO_ROOT)),
    }
    path = run_dir / "review.json"
    path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return path


def _parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("pr", help="PR number, URL, or branch.")
    p.add_argument("--repo", help="GitHub repo (owner/name). Defaults to the cwd's gh remote.")
    p.add_argument("--hal-endpoint", required=True, help="HAL adapter URL.")
    p.add_argument("--model", required=True, help="Bare served-model name.")
    p.add_argument("--out", type=Path, help="Write review draft to this path.")
    p.add_argument("--no-manifest", action="store_true")
    return p.parse_args()


def main() -> int:
    args = _parse_args()
    diff = _pr_diff(args.pr, args.repo)
    metadata = _pr_metadata(args.pr, args.repo)
    if not metadata:
        print(f"[review] could not load metadata for PR {args.pr}", file=sys.stderr)
        return 1
    if not diff:
        print(f"[review] empty diff for PR {args.pr}", file=sys.stderr)
        return 1
    print(
        f"[review] PR #{metadata.get('number')} +{metadata.get('additions')}"
        f"/-{metadata.get('deletions')} across "
        f"{metadata.get('changedFiles')} files",
        file=sys.stderr,
    )

    checks = _run_checks(metadata)
    draft, fields_honored = _draft_via_hal(
        args.hal_endpoint, args.model, metadata, diff, checks
    )
    rendered = _render_review(metadata, checks, draft)
    timestamp = datetime.now(timezone.utc)

    if args.out:
        args.out.write_text(rendered, encoding="utf-8")
        draft_path = args.out.resolve()
        print(f"[review] wrote draft to {draft_path}", file=sys.stderr)
    else:
        sys.stdout.write(rendered)
        draft_path = MANIFEST_DIR / timestamp.strftime("%Y%m%dT%H%M%SZ") / "draft.md"
        draft_path.parent.mkdir(parents=True, exist_ok=True)
        draft_path.write_text(rendered, encoding="utf-8")

    if not args.no_manifest:
        manifest_path = _write_manifest(
            timestamp=timestamp,
            pr_ref=args.pr,
            metadata=metadata,
            checks=checks,
            draft_path=draft_path,
            hal_endpoint=args.hal_endpoint,
            model=args.model,
            fields_honored=fields_honored,
        )
        print(f"[review] manifest: {manifest_path}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
