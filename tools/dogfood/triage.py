#!/usr/bin/env python3
"""triage.py — classify recent GitHub issues via the HAL adapter.

Plan 05 IMPL surface — see `tools/dogfood/README.md`. Pulls the last
N days of GitHub issues via `gh issue list`, sends each one through
the HAL adapter to an LLM for {bug, feature, question, noise}
classification + suggested assignees, and emits a Markdown digest
plus a per-run manifest.

Ground truth (Plan 05): human-applied issue labels and assignee fields
after the digest is published. Agreement rate is the headline number.

Pure stdlib + gh + urllib. Requires:
- `gh` authenticated against the target repo
- HAL adapter running on the URL passed via --hal-endpoint (e.g.
  http://127.0.0.1:18080 — see tools/hal_adapter/server.py)
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import textwrap
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib import error as urllib_error
from urllib import request as urllib_request

REPO_ROOT = Path(__file__).resolve().parents[2]
MANIFEST_DIR = REPO_ROOT / ".apxm" / "evaluation" / "agentic" / "dogfood"

# Categories the classifier is asked to choose from. Mirroring the
# canonical Keep-a-Changelog categories means an issue's classification
# can feed directly into the release-notes synthesizer (sister command).
CATEGORIES = ["bug", "feature", "question", "noise"]

CLASSIFY_SYSTEM_PROMPT = textwrap.dedent("""
    You are APXM's issue-triage assistant. Classify each GitHub issue
    into exactly one category and suggest at most one reviewer login
    (or "" if nobody is the obvious owner).

    Reply with a single JSON object: {"category": "...", "assignee": "...",
    "rationale": "one short sentence"}. No prose outside the JSON.

    Categories:
    - bug: reproducible defect with a behavior contract.
    - feature: enhancement request or new capability.
    - question: usage question, not a defect.
    - noise: spam, duplicate, off-topic, or already-resolved.
""").strip()


def _gh_issue_list(repo: str | None, days: int, state: str, limit: int) -> list[dict]:
    """Run `gh issue list --json ...` and return parsed records."""
    cmd = [
        "gh", "issue", "list",
        "--state", state,
        "--limit", str(limit),
        "--json", "number,title,body,author,labels,assignees,createdAt,updatedAt",
    ]
    if days > 0:
        # Use --search "created:>{date}" to bound the window. Avoids
        # pulling the entire backlog on a noisy repo.
        from datetime import timedelta
        since = (datetime.now(timezone.utc) - timedelta(days=days)).strftime("%Y-%m-%d")
        cmd.extend(["--search", f"created:>{since}"])
    if repo:
        cmd.extend(["--repo", repo])
    try:
        proc = subprocess.run(cmd, check=True, capture_output=True, text=True)
    except subprocess.CalledProcessError as exc:
        print(
            f"[triage] gh issue list failed (status {exc.returncode}): "
            f"{exc.stderr.strip()}",
            file=sys.stderr,
        )
        return []
    except FileNotFoundError:
        print("[triage] `gh` not installed — install GitHub CLI.", file=sys.stderr)
        return []
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        print(f"[triage] gh output not JSON: {exc}", file=sys.stderr)
        return []


def _classify_via_hal(
    hal_endpoint: str,
    model: str,
    issue: dict,
    *,
    timeout_s: float = 60.0,
) -> dict[str, str]:
    """Send one issue to the HAL adapter; return the parsed classification.

    On any failure (timeout, malformed JSON, HTTP error), return a
    classification of `unknown` so the dogfooding run continues — the
    digest then reports the unknowns honestly rather than crashing."""
    title = issue.get("title", "(no title)")
    body = (issue.get("body") or "")[:4000]  # cap body length
    labels = ", ".join(l["name"] for l in (issue.get("labels") or []))
    user_prompt = (
        f"Issue title: {title}\n\n"
        f"Existing labels: {labels or '(none)'}\n\n"
        f"Body (truncated to 4 KB):\n{body}"
    )

    payload = {
        "model": model,
        "messages": [
            {"role": "system", "content": CLASSIFY_SYSTEM_PROMPT},
            {"role": "user", "content": user_prompt},
        ],
        "max_tokens": 256,
        # APXM hints: group all triage classifications into one cohort
        # so the pin path can engage on the shared system prompt.
        "vllm_xargs": {
            "apxm": {
                "reuse_group": "dogfood-triage-cohort",
                "pin_policy": {"mode": "prefix", "ttl_ms": 30000},
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
        return {
            "category": "unknown",
            "assignee": "",
            "rationale": f"hal call failed: {exc}",
            "fields_honored": "",
        }
    try:
        envelope = json.loads(body)
        content = (
            envelope.get("choices", [{}])[0].get("message", {}).get("content", "")
            or envelope.get("content", "")
        )
        parsed = json.loads(content) if content else {}
    except (json.JSONDecodeError, IndexError, KeyError):
        parsed = {}
    return {
        "category": str(parsed.get("category", "unknown") or "unknown"),
        "assignee": str(parsed.get("assignee", "") or ""),
        "rationale": str(parsed.get("rationale", "") or ""),
        "fields_honored": honored,
    }


def _render_digest(issues: list[dict], results: list[dict]) -> str:
    """Markdown digest grouped by category."""
    by_cat: dict[str, list[tuple[dict, dict]]] = {c: [] for c in CATEGORIES + ["unknown"]}
    for issue, result in zip(issues, results):
        by_cat.setdefault(result["category"], []).append((issue, result))

    lines = ["# Issue triage digest", ""]
    for cat in CATEGORIES + ["unknown"]:
        items = by_cat.get(cat, [])
        if not items:
            continue
        lines.append(f"## {cat} ({len(items)})")
        lines.append("")
        for issue, result in items:
            num = issue.get("number", "?")
            title = issue.get("title", "(no title)")
            suggested = result["assignee"] or "—"
            rationale = result["rationale"]
            lines.append(f"- **#{num}** {title}")
            lines.append(f"  - suggested assignee: `{suggested}`")
            lines.append(f"  - rationale: {rationale}")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def _write_manifest(
    *,
    timestamp: datetime,
    repo: str | None,
    days: int,
    issues: list[dict],
    results: list[dict],
    digest_path: Path,
    hal_endpoint: str,
    model: str,
) -> Path:
    MANIFEST_DIR.mkdir(parents=True, exist_ok=True)
    run_dir = MANIFEST_DIR / timestamp.strftime("%Y%m%dT%H%M%SZ")
    run_dir.mkdir(parents=True, exist_ok=True)
    manifest = {
        "command": "triage",
        "started_at": timestamp.isoformat(),
        "repo": repo or "(default)",
        "days_window": days,
        "issues_scanned": len(issues),
        "by_category": {
            c: sum(1 for r in results if r["category"] == c)
            for c in CATEGORIES + ["unknown"]
        },
        "hal_endpoint": hal_endpoint,
        "model": model,
        "digest_path": str(digest_path.relative_to(REPO_ROOT)),
        # x-apxm-fields-honored evidence union'd across requests.
        "fields_honored_observed": sorted(
            {f for r in results for f in (r["fields_honored"].split(",") if r["fields_honored"] else [])}
        ),
    }
    path = run_dir / "triage.json"
    path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return path


def _parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--repo", help="GitHub repo (owner/name); defaults to current directory's gh remote.")
    p.add_argument("--days", type=int, default=14, help="Issues created in the last N days. 0 = no time bound.")
    p.add_argument("--state", default="open", choices=["open", "closed", "all"])
    p.add_argument("--limit", type=int, default=30, help="Max issues to triage in one run.")
    p.add_argument("--hal-endpoint", required=True, help="HAL adapter URL (e.g. http://127.0.0.1:18080).")
    p.add_argument("--model", required=True, help="Bare served-model name (matches HAL --model).")
    p.add_argument("--out", type=Path, help="Write digest to this path. Default: print + manifest sidecar.")
    p.add_argument("--no-manifest", action="store_true")
    return p.parse_args()


def main() -> int:
    args = _parse_args()
    issues = _gh_issue_list(args.repo, args.days, args.state, args.limit)
    if not issues:
        print(f"[triage] no issues matched", file=sys.stderr)
        return 0
    print(f"[triage] classifying {len(issues)} issues via {args.hal_endpoint}", file=sys.stderr)

    results: list[dict] = []
    for i, issue in enumerate(issues, start=1):
        result = _classify_via_hal(args.hal_endpoint, args.model, issue)
        results.append(result)
        if i % 5 == 0 or i == len(issues):
            print(f"[triage] classified {i}/{len(issues)}", file=sys.stderr)

    digest = _render_digest(issues, results)
    timestamp = datetime.now(timezone.utc)

    if args.out:
        args.out.write_text(digest, encoding="utf-8")
        digest_path = args.out.resolve()
        print(f"[triage] wrote digest to {digest_path}", file=sys.stderr)
    else:
        sys.stdout.write(digest)
        digest_path = MANIFEST_DIR / timestamp.strftime("%Y%m%dT%H%M%SZ") / "digest.md"
        digest_path.parent.mkdir(parents=True, exist_ok=True)
        digest_path.write_text(digest, encoding="utf-8")

    if not args.no_manifest:
        manifest_path = _write_manifest(
            timestamp=timestamp,
            repo=args.repo,
            days=args.days,
            issues=issues,
            results=results,
            digest_path=digest_path,
            hal_endpoint=args.hal_endpoint,
            model=args.model,
        )
        print(f"[triage] manifest: {manifest_path}", file=sys.stderr)

    print(
        f"[triage] done — {len(issues)} issues, "
        f"{sum(1 for r in results if r['category'] != 'unknown')} classified, "
        f"{sum(1 for r in results if r['category'] == 'unknown')} unknown",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
