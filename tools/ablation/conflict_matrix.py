"""Pass-conflict matrix (Phase B Task 12).

For each suspicious pass pair, run both orderings against the stress
corpus and flag any (pair, graph) cell where the final op count diverges
by more than one op.

Suspicious pairs (from compiler-audit.md):
    (cse, fuse-ask-ops)                          -- both eliminate redundancy
    (template-specialization, prompt-canonicalization)  -- both rewrite templates

Exit code:
    0   no order-dependent pair found.
    1   at least one ordering produced a different final_ops.
    2   environment/setup failure (missing dekk, missing graph, etc.).

The plan originally calls for `--emit-metrics`; that flag is only
partially implemented today, so we reuse `--emit-diagnostics` (which
is what the Task 11 harness already drives) and read
``pass_summary.final_ops``.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from .runner import BENCH_DIR, STRESS_GRAPHS

PAIRS = [
    ("cse", "fuse-ask-ops"),
    ("template-specialization", "prompt-canonicalization"),
]

# A minimum viable preamble that gets the .air through normalization and
# template materialization before we test the contested pair. Without
# build-prompt the template-specialization pair has nothing to rewrite.
PREAMBLE = ["normalize", "build-prompt"]


def _run(graph: Path, passes: list[str], diag_out: Path, artifact_out: Path) -> int:
    cmd = [
        "dekk",
        "apxm",
        "compile",
        str(graph),
        "--pass-list",
        ",".join(passes),
        "-o",
        str(artifact_out),
        "--emit-diagnostics",
        str(diag_out),
    ]
    subprocess.check_call(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    diag = json.loads(diag_out.read_text())
    return int(diag["pass_summary"]["final_ops"])


def main() -> int:
    if shutil.which("dekk") is None:
        print("error: `dekk` not on PATH; cannot drive `dekk apxm compile`", file=sys.stderr)
        return 2
    for g in STRESS_GRAPHS:
        if not (BENCH_DIR / g).exists():
            print(f"error: missing stress graph {BENCH_DIR / g}", file=sys.stderr)
            return 2

    rows: list[tuple[str, str, str, int, int]] = []
    with tempfile.TemporaryDirectory(prefix="apxm-conflict-") as tmp:
        tmpdir = Path(tmp)
        for a, b in PAIRS:
            for g in STRESS_GRAPHS:
                ab_diag = tmpdir / f"ab_{a}_{b}_{g}.json"
                ab_obj = tmpdir / f"ab_{a}_{b}_{g}.apxmobj"
                ba_diag = tmpdir / f"ba_{a}_{b}_{g}.json"
                ba_obj = tmpdir / f"ba_{a}_{b}_{g}.apxmobj"
                ab = _run(BENCH_DIR / g, PREAMBLE + [a, b], ab_diag, ab_obj)
                ba = _run(BENCH_DIR / g, PREAMBLE + [b, a], ba_diag, ba_obj)
                rows.append((a, b, g, ab, ba))

    flagged = [r for r in rows if abs(r[3] - r[4]) > 1]

    print("# Pass-conflict matrix\n")
    print("| pair | graph | AB final_ops | BA final_ops | divergent? |")
    print("|---|---|---:|---:|:---:|")
    for a, b, g, x, y in rows:
        marker = "YES" if abs(x - y) > 1 else "no"
        print(f"| {a} ↔ {b} | {g} | {x} | {y} | {marker} |")

    if flagged:
        print(
            f"\n## Order-dependent pairs (|ops_after_AB - ops_after_BA| > 1):"
        )
        for a, b, g, x, y in flagged:
            print(f"- {a} ↔ {b} on {g}: AB={x} BA={y}")
        return 1

    print("\nNo order-dependent pairs detected.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
