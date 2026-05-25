#!/usr/bin/env python3
"""Idempotently install APXM git hooks.

Hooks installed:

- `commit-msg`: runs `tools/scripts/check_commit_message.py` against the
  staged message. Blocks commits that fail the lint.
- `prepare-commit-msg`: prepends a short template reminder to empty
  messages, pointing at `_shared/apxm-commit-message-rules.md`.

Re-running this script is safe — hooks carry a marker line that lets us
detect a previous APXM install and overwrite only those. A
user-customized hook (no marker) is left alone and reported.
"""

from __future__ import annotations

import argparse
import os
import stat
import subprocess
import sys
from pathlib import Path


MARKER = "# apxm-managed-hook"
REPO_ROOT = Path(__file__).resolve().parents[2]


COMMIT_MSG_HOOK = f"""#!/usr/bin/env bash
{MARKER}
exec python3 "$(git rev-parse --show-toplevel)/tools/scripts/check_commit_message.py" "$1"
"""


PREPARE_COMMIT_MSG_HOOK = f"""#!/usr/bin/env bash
{MARKER}
msg_file="$1"
src="$2"
case "$src" in
  message|template|merge|squash|commit) exit 0 ;;
esac
if [ -n "$(grep -v '^\\s*#' "$msg_file" | tr -d '[:space:]')" ]; then
  exit 0
fi
{{
  echo "# <type>(<scope>): <subject>      (max 72 chars, imperative)"
  echo "#"
  echo "# Types: feat fix perf refactor docs test chore bench eval prereg"
  echo "# Scope: subsystem name; planNN only for prereg/eval."
  echo "# Body explains WHY. No AI-attribution lines."
  echo "# Rules: .agents/skills/_shared/apxm-commit-message-rules.md"
  cat "$msg_file"
}} > "$msg_file.apxm.tmp" && mv "$msg_file.apxm.tmp" "$msg_file"
"""


HOOKS: dict[str, str] = {
    "commit-msg": COMMIT_MSG_HOOK,
    "prepare-commit-msg": PREPARE_COMMIT_MSG_HOOK,
}


HOOKS_DIR_REL = "tools/git-hooks"


def _ensure_hooks_path_config() -> None:
    out = subprocess.run(
        ["git", "config", "--local", "--get", "core.hooksPath"],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
    )
    current = out.stdout.strip()
    if current == HOOKS_DIR_REL:
        return
    subprocess.run(
        ["git", "config", "--local", "core.hooksPath", HOOKS_DIR_REL],
        check=True,
        cwd=REPO_ROOT,
    )
    if current:
        print(
            f"[install-hooks] core.hooksPath: {current!r} -> {HOOKS_DIR_REL!r}",
            file=sys.stderr,
        )
    else:
        print(f"[install-hooks] core.hooksPath: set to {HOOKS_DIR_REL!r}", file=sys.stderr)


def _hooks_dir() -> Path:
    path = REPO_ROOT / HOOKS_DIR_REL
    path.mkdir(parents=True, exist_ok=True)
    return path


def install(force: bool) -> int:
    _ensure_hooks_path_config()
    hooks_dir = _hooks_dir()
    rc = 0
    for name, body in HOOKS.items():
        dest = hooks_dir / name
        if dest.exists():
            existing = dest.read_text(encoding="utf-8", errors="replace")
            if MARKER not in existing and not force:
                print(
                    f"[install-hooks] SKIP {dest}: customized (no APXM marker); "
                    f"re-run with --force to overwrite",
                    file=sys.stderr,
                )
                rc = 2
                continue
            if existing == body:
                print(f"[install-hooks] OK   {dest}: up to date")
                continue
        dest.write_text(body, encoding="utf-8")
        dest.chmod(dest.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
        print(f"[install-hooks] WROTE {dest}")
    return rc


def uninstall() -> int:
    hooks_dir = _hooks_dir()
    for name in HOOKS:
        dest = hooks_dir / name
        if not dest.exists():
            continue
        existing = dest.read_text(encoding="utf-8", errors="replace")
        if MARKER not in existing:
            print(f"[install-hooks] SKIP {dest}: not APXM-managed", file=sys.stderr)
            continue
        dest.unlink()
        print(f"[install-hooks] REMOVED {dest}")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--force", action="store_true", help="overwrite customized hooks")
    parser.add_argument("--uninstall", action="store_true", help="remove APXM-managed hooks")
    args = parser.parse_args(argv)
    if args.uninstall:
        return uninstall()
    return install(args.force)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
