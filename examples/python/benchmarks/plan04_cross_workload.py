#!/usr/bin/env python3
"""cross_workload.py — stitch per-workload CSVs into a combined cross-system benchmark output.

Reads per-workload CSVs from Mooncake/ShareGPT/LooGLE/pin_demo,
adds `workload` and `paper_alignment` columns, computes per-(workload, arm)
bootstrap-CI ratios, and writes a combined CSV plus a per-workload
comparison summary.

Each --input is `workload-name:path-to-csv`. Arm columns (apxm-on /
flat-http) must be present in the source CSVs.
"""
from __future__ import annotations

import argparse
import csv
import json
import random
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]

_TOOLS_SCRIPT_DIR = str(REPO_ROOT / "tools" / "scripts")
if _TOOLS_SCRIPT_DIR not in sys.path:
    sys.path.insert(0, _TOOLS_SCRIPT_DIR)

from apxm_vllm_contract import ArmName  # noqa: E402

# Per-workload mapping table cites the paper each workload tests against.
WORKLOAD_TO_PAPER_ALIGNMENT = {
    "mooncake": "Mooncake (FAST 2025) — TTFT SLO satisfaction; production trace replay",
    "sharegpt": "AttentionStore (ATC 2024) — multi-turn KV reuse hit rate",
    "loogle":   "Mooncake/SGLang — long-shared-context fan-out",
    "pin_demo": "Parrot (OSDI 2024) — DAG critical-path latency, named cohort",
    "pfx_cancel": "Parrot (OSDI 2024) — cancellation effectiveness",
    "gsp":      "SGLang/RadixAttention — block-level prefix-cache hit rate diagnostic",
}


def _parse_inputs(raw_inputs: list[str]) -> list[tuple[str, Path]]:
    """Each input is `workload:path`. Returns [(workload, Path)]."""
    out: list[tuple[str, Path]] = []
    for entry in raw_inputs:
        if ":" not in entry:
            print(
                f"error: --input must be `workload:path`, got `{entry}`",
                file=sys.stderr,
            )
            raise SystemExit(2)
        workload, _, path_str = entry.partition(":")
        path = Path(path_str)
        if not path.is_file():
            print(f"error: input CSV not found: {path}", file=sys.stderr)
            raise SystemExit(2)
        out.append((workload, path))
    return out


def _read_csv(path: Path) -> list[dict]:
    with path.open(newline="") as f:
        return list(csv.DictReader(f))


def _bootstrap_ratio_ci(num: list[float], den: list[float], *, n: int = 10000, seed: int = 42):
    """Paired bootstrap of (mean(num) / mean(den))."""
    if not num or not den:
        return (float("nan"), float("nan"), float("nan"))
    rng = random.Random(seed)
    samples = []
    for _ in range(n):
        a = [rng.choice(num) for _ in num]
        b = [rng.choice(den) for _ in den]
        samples.append((sum(a) / len(a)) / (sum(b) / len(b)))
    samples.sort()
    mean = (sum(num) / len(num)) / (sum(den) / len(den))
    return (mean, samples[int(0.025 * n)], samples[int(0.975 * n)])


def _by_arm(rows: list[dict]) -> dict[str, list[float]]:
    out: dict[str, list[float]] = defaultdict(list)
    for r in rows:
        wall = r.get("batch_wall_ms")
        if not wall:
            continue
        try:
            out[r.get("arm", "unknown")].append(float(wall))
        except (TypeError, ValueError):
            continue
    return out


def _per_workload_summary(combined_rows: list[dict]) -> list[dict]:
    """One summary row per workload — apxm-on / flat-http ratio with CI."""
    by_workload: dict[str, list[dict]] = defaultdict(list)
    for r in combined_rows:
        by_workload[r.get("workload", "unknown")].append(r)
    summaries: list[dict] = []
    for workload, rows in sorted(by_workload.items()):
        arm_walls = _by_arm(rows)
        apxm = arm_walls.get(ArmName.APXM_ON.value, [])
        flat = arm_walls.get(ArmName.FLAT_HTTP.value, [])
        mean, lo, hi = _bootstrap_ratio_ci(apxm, flat)
        summaries.append({
            "workload": workload,
            "paper_alignment": WORKLOAD_TO_PAPER_ALIGNMENT.get(
                workload, "(no paper alignment recorded — add to WORKLOAD_TO_PAPER_ALIGNMENT)"
            ),
            "n_apxm_on": len(apxm),
            "n_flat_http": len(flat),
            "apxm_over_flat_ratio_mean": (
                f"{mean:.3f}" if mean == mean else "n/a"  # NaN check
            ),
            "apxm_over_flat_ratio_ci_low": (
                f"{lo:.3f}" if lo == lo else "n/a"
            ),
            "apxm_over_flat_ratio_ci_high": (
                f"{hi:.3f}" if hi == hi else "n/a"
            ),
            "verdict": _verdict(mean, lo, hi),
        })
    return summaries


def _verdict(mean: float, lo: float, hi: float) -> str:
    if mean != mean:  # NaN
        return "indeterminate (missing arm)"
    if hi < 1.0:
        return f"APXM win — {100*(1-mean):.1f}% mean reduction; CI excludes 1.0"
    if lo > 1.0:
        return f"flat-HTTP win — APXM {100*(mean-1):.1f}% slower; CI excludes 1.0"
    return f"indeterminate — central tendency {('APXM ' + str(round(100*(1-mean),1)) + '%') if mean<1 else ('flat ' + str(round(100*(mean-1),1)) + '%')}; CI crosses 1.0"


def _parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--input", "-i", action="append", required=True,
        metavar="WORKLOAD:CSV_PATH",
        help="One per (workload, csv) source. Repeat for each workload's arm CSVs.",
    )
    p.add_argument(
        "--output", type=Path, required=True,
        help="Combined CSV destination — includes per-row workload column.",
    )
    p.add_argument(
        "--summary", type=Path, default=None,
        help="Optional per-workload summary CSV with bootstrap-CI ratios.",
    )
    p.add_argument(
        "--manifest", type=Path, default=None,
        help="Optional JSON manifest sidecar (workload counts, paper alignment, ratios).",
    )
    return p.parse_args()


def main() -> int:
    args = _parse_args()
    sources = _parse_inputs(args.input)

    # Union of all field names across input CSVs, with workload prepended.
    combined_rows: list[dict] = []
    field_union: list[str] = ["workload", "paper_alignment"]
    seen = set(field_union)
    for workload, path in sources:
        rows = _read_csv(path)
        for r in rows:
            for k in r.keys():
                if k not in seen:
                    field_union.append(k)
                    seen.add(k)
            r["workload"] = workload
            r["paper_alignment"] = WORKLOAD_TO_PAPER_ALIGNMENT.get(
                workload, "(no paper alignment recorded)"
            )
            combined_rows.append(r)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=field_union, extrasaction="ignore")
        w.writeheader()
        for r in combined_rows:
            w.writerow(r)
    print(f"[plan04] wrote {len(combined_rows)} rows from {len(sources)} sources → {args.output}", file=sys.stderr)

    summaries = _per_workload_summary(combined_rows)
    summary_path = args.summary or args.output.with_name(args.output.stem + ".summary.csv")
    summary_fields = list(summaries[0].keys()) if summaries else []
    with summary_path.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=summary_fields)
        w.writeheader()
        for s in summaries:
            w.writerow(s)
    print(f"[plan04] per-workload summary → {summary_path}", file=sys.stderr)

    if args.manifest:
        manifest = {
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "sources": [
                {"workload": w, "csv": str(p), "rows": sum(1 for r in combined_rows if r.get("workload") == w)}
                for w, p in sources
            ],
            "workloads_covered": sorted({w for w, _ in sources}),
            "summaries": summaries,
        }
        args.manifest.parent.mkdir(parents=True, exist_ok=True)
        args.manifest.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        print(f"[plan04] manifest → {args.manifest}", file=sys.stderr)

    print("\n=== per-workload summary ===", file=sys.stderr)
    for s in summaries:
        print(f"  {s['workload']}: {s['verdict']}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
