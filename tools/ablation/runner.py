"""Compiler-pass ablation harness.

Toggle each registered pass off across the stress-graph corpus, parse the
emitted diagnostics JSON, and print a Markdown delta table.

Exit code:
    0   no pass shows zero firings on every graph; no disabled-pass run regresses
        ops_after by > 5% relative to the all-passes baseline.
    1   at least one pass earns zero firings everywhere (candidate for removal),
        or a regression threshold is exceeded.

Note: "fired" is currently approximated as ``ops_delta != 0 OR fired_count > 0``.
Once per-pass ``_fired_count`` IntegerAttrs are wired
into every C++ transform, ``fired_count`` alone becomes authoritative and we
can drop the ``ops_delta`` fallback.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
BENCH_DIR = REPO_ROOT / "examples" / "python" / "benchmarks" / "stress"

STRESS_GRAPHS = [
    "cse_stress.air",
    "priority_scheduling.air",
    "dead_context_stress.air",
]

# Mirrors the MLIR-side pass list dispatched at -O 2 by
# crates/compiler/apxm-compiler/src/passes/pipeline.rs::build_pass_list.
# Passes that live exclusively on the Rust side (vllm-hints, tool-binding,
# bind-tool-handlers) are dispatched on the AirModule before the MLIR
# PassManager runs, so they never appear in the per-pass diagnostics array
# and therefore are not toggleable via --disable-pass on .air inputs.
PASSES = [
    "normalize",
    "build-prompt",
    "template-specialization",
    "assign-priority",
    "dead-context-elimination",
    "canonicalizer",
    "cse",
    "symbol-dce",
]

# Passes whose fired_count is structurally unobservable via the
# `ais.<pass>_fired_count` attribute. Once every APXM-side
# transform writes that attr, so the only remaining exemptions are upstream
# MLIR passes (cse, symbol-dce, canonicalizer) which live outside our
# Transforms/ tree and we cannot patch. The dead-pass check skips these;
# the regression check (which compares ops_after) still covers them.
PRE_TASK7_UNINSTRUMENTED = {
    "cse",
    "symbol-dce",
    "canonicalizer",
}

REGRESSION_THRESHOLD = 0.05  # 5%


@dataclass
class RunResult:
    graph: str
    disabled: str | None  # None = baseline
    ops_after: int
    total_tokens_saved: int
    fired_counts: dict[str, int]
    ops_deltas: dict[str, int]


def _compile(graph: Path, disable: list[str], diag_out: Path, artifact_out: Path) -> dict:
    cmd = [
        "dekk",
        "apxm",
        "compile",
        str(graph),
        "-O",
        "2",
        "-o",
        str(artifact_out),
        "--emit-diagnostics",
        str(diag_out),
    ]
    for d in disable:
        cmd += ["--disable-pass", d]
    subprocess.check_call(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    return json.loads(diag_out.read_text())


def _collect(graph_name: str, disable: str | None, tmpdir: Path) -> RunResult:
    tag = disable or "baseline"
    diag_path = tmpdir / f"{graph_name}_{tag}.json"
    artifact_path = tmpdir / f"{graph_name}_{tag}.apxmobj"
    diag = _compile(
        BENCH_DIR / graph_name,
        [disable] if disable else [],
        diag_path,
        artifact_path,
    )
    fired = {p["pass_name"]: int(p.get("fired_count", 0)) for p in diag["pass_metrics"]}
    deltas = {p["pass_name"]: int(p.get("ops_delta", 0)) for p in diag["pass_metrics"]}
    return RunResult(
        graph=graph_name,
        disabled=disable,
        ops_after=int(diag["pass_summary"]["final_ops"]),
        total_tokens_saved=int(diag["pass_summary"].get("total_tokens_saved", 0)),
        fired_counts=fired,
        ops_deltas=deltas,
    )


def _did_fire(baseline: RunResult, pass_name: str) -> bool:
    """A pass "fired" if its fired_count > 0 OR it changed the op count.

    Pre-Task-7, only a subset of passes writes fired_count; the rest only show
    up via ops_delta. This OR keeps the harness honest until Task 7 lands the
    full per-pass _fired_count plumbing.
    """
    return baseline.fired_counts.get(pass_name, 0) > 0 or baseline.ops_deltas.get(pass_name, 0) != 0


def main() -> int:
    if shutil.which("dekk") is None:
        print("error: `dekk` not on PATH; cannot drive `dekk apxm compile`", file=sys.stderr)
        return 2
    for g in STRESS_GRAPHS:
        if not (BENCH_DIR / g).exists():
            print(f"error: missing stress graph {BENCH_DIR / g}", file=sys.stderr)
            return 2

    with tempfile.TemporaryDirectory(prefix="apxm-ablation-") as tmp:
        tmpdir = Path(tmp)
        baseline = {g: _collect(g, None, tmpdir) for g in STRESS_GRAPHS}
        results: list[RunResult] = []
        for pass_name in PASSES:
            for g in STRESS_GRAPHS:
                results.append(_collect(g, pass_name, tmpdir))

    dead_passes = [
        p
        for p in PASSES
        if p not in PRE_TASK7_UNINSTRUMENTED
        and all(not _did_fire(baseline[g], p) for g in STRESS_GRAPHS)
    ]

    regressions: list[tuple[str, str, float]] = []
    baseline_lookup = {g: baseline[g].ops_after for g in STRESS_GRAPHS}
    for r in results:
        b = baseline_lookup[r.graph]
        if b > 0:
            grow = (r.ops_after - b) / b
            if grow > REGRESSION_THRESHOLD:
                regressions.append((r.disabled or "?", r.graph, grow))

    print("# Ablation delta table\n")
    print("| pass disabled | graph | ops_after | Δ vs baseline | tokens_saved |")
    print("|---|---|---:|---:|---:|")
    for r in results:
        b = baseline_lookup[r.graph]
        d = r.ops_after - b
        print(
            f"| {r.disabled} | {r.graph} | {r.ops_after} | {d:+d} | {r.total_tokens_saved} |"
        )

    if dead_passes:
        print("\n## Passes that never fired across the corpus:")
        for p in dead_passes:
            print(f"- {p}")
    if regressions:
        print(f"\n## Disabling these passes regressed ops_after by > {REGRESSION_THRESHOLD * 100:.0f}%:")
        for d, g, grow in regressions:
            print(f"- disable {d} on {g}: +{grow * 100:.1f}%")

    return 1 if (dead_passes or regressions) else 0


if __name__ == "__main__":
    raise SystemExit(main())
