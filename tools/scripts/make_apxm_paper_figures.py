#!/usr/bin/env python3
"""Generate APXM paper figures from accepted local evidence artifacts."""

from __future__ import annotations

import csv
import html
import json
from pathlib import Path
from typing import Iterable


ROOT = Path(__file__).resolve().parents[2]
FIG_DIR = ROOT / "docs" / "paper" / "figures"
DATA_DIR = ROOT / ".apxm" / "evaluation" / "paper" / "20260521"
PRIORITY_ROOT = ROOT / ".apxm" / "evaluation" / "apxm-priority-lane" / "runs"
ACCEPTED_RUNS = [
    ("Sequential", PRIORITY_ROOT / "20260521T040617Z-c16-bg16"),
    ("Interleaved", PRIORITY_ROOT / "20260521T124230Z-c16-bg16-interleaved"),
    (
        "Fresh service",
        PRIORITY_ROOT / "20260521T140100Z-c16-bg16-interleaved-iter10",
    ),
    (
        "Fresh repeat 2",
        PRIORITY_ROOT / "20260521T233817Z-c16-bg16-interleaved-repeat2",
    ),
]
COMBINED_ANALYSIS = (
    ROOT
    / ".apxm"
    / "evaluation"
    / "apxm-priority-lane"
    / "combined"
    / "20260522T001900Z-fresh-service-repeats"
    / "combined-priority-lane-analysis.json"
)


def esc(value: object) -> str:
    return html.escape(str(value), quote=True)


def write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def svg(width: int, height: int, body: str) -> str:
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img">
  <style>
    text {{ font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; fill: #1f2933; }}
    .title {{ font-size: 22px; font-weight: 700; }}
    .subtitle {{ font-size: 13px; fill: #52606d; }}
    .label {{ font-size: 12px; fill: #334e68; }}
    .small {{ font-size: 11px; fill: #627d98; }}
    .axis {{ stroke: #9fb3c8; stroke-width: 1; }}
    .grid {{ stroke: #d9e2ec; stroke-width: 1; }}
    .apxm {{ fill: #2f80ed; }}
    .flat {{ fill: #d64545; }}
    .neutral {{ fill: #f0b429; }}
    .good {{ fill: #2f9e44; }}
    .bad {{ fill: #d64545; }}
    .card {{ fill: #ffffff; stroke: #bcccdc; stroke-width: 1.2; rx: 8; }}
  </style>
{body}
</svg>
"""


def load_summary() -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    for label, run_dir in ACCEPTED_RUNS:
        with (run_dir / "summary.csv").open(newline="", encoding="utf-8") as f:
            row = next(csv.DictReader(f))
        rows.append(
            {
                "label": label,
                "path": str(run_dir.relative_to(ROOT)),
                "paired_rows": int(row["paired_rows"]),
                "mean_ratio": float(row["mean_ratio"]),
                "mean_ci_low": float(row["mean_ci_low"]),
                "mean_ci_high": float(row["mean_ci_high"]),
                "p95_ratio": float(row["p95_ratio"]),
                "p95_ci_low": float(row["p95_ci_low"]),
                "p95_ci_high": float(row["p95_ci_high"]),
                "verdict": row["verdict"],
            }
        )
    return rows


def load_latest() -> tuple[dict[str, object], dict[str, object], list[dict[str, str]]]:
    latest = ACCEPTED_RUNS[-1][1]
    report = json.loads((latest / "report.json").read_text(encoding="utf-8"))
    enhanced = json.loads(
        (latest / "priority-lane-enhanced-analysis.json").read_text(encoding="utf-8")
    )
    with (latest / "priority-lane-batch-tradeoffs.csv").open(
        newline="", encoding="utf-8"
    ) as f:
        tradeoffs = list(csv.DictReader(f))
    return report, enhanced, tradeoffs


def load_combined() -> dict[str, object] | None:
    if not COMBINED_ANALYSIS.exists():
        return None
    return json.loads(COMBINED_ANALYSIS.read_text(encoding="utf-8"))


def architecture_svg() -> str:
    boxes = [
        ("Plans + skills", "Typed AIS operations, skill bundles, task graphs"),
        ("APXM compiler", "AIR/MLIR, critical-path marking, dispatch lowering"),
        ("Compiled artifact", ".apxmobj with graph, metadata, provenance"),
        ("APXM runtime", "Scheduler, sandbox, router, observability"),
        ("LLM backends", "APXM-vLLM, local frontier, cloud frontier"),
    ]
    x = 90
    y = 88
    w = 760
    h = 76
    gap = 28
    parts = [
        '<rect width="940" height="560" fill="#f7f9fb"/>',
        '<text x="60" y="44" class="title">APXM compiler and runtime path</text>',
        '<text x="60" y="66" class="subtitle">Agent programs become optimized graph artifacts, then execute through graph-aware dispatch.</text>',
    ]
    colors = ["#e0f2fe", "#dcfce7", "#fef3c7", "#ede9fe", "#fee2e2"]
    for idx, ((title, desc), color) in enumerate(zip(boxes, colors)):
        yy = y + idx * (h + gap)
        parts.append(
            f'<rect x="{x}" y="{yy}" width="{w}" height="{h}" rx="8" fill="{color}" stroke="#829ab1"/>'
        )
        parts.append(f'<text x="{x + 26}" y="{yy + 31}" font-size="18" font-weight="700">{esc(title)}</text>')
        parts.append(f'<text x="{x + 26}" y="{yy + 54}" class="label">{esc(desc)}</text>')
        if idx < len(boxes) - 1:
            ax = x + w / 2
            ay1 = yy + h + 5
            ay2 = yy + h + gap - 5
            parts.append(f'<line x1="{ax}" y1="{ay1}" x2="{ax}" y2="{ay2}" stroke="#486581" stroke-width="2"/>')
            parts.append(f'<path d="M {ax - 6} {ay2 - 7} L {ax} {ay2 + 1} L {ax + 6} {ay2 - 7}" fill="none" stroke="#486581" stroke-width="2"/>')
    parts.append('<text x="90" y="530" class="small">Figure generated from paper figure script; design summary follows APXM narrative and thesis docs.</text>')
    return svg(940, 560, "\n".join("  " + p for p in parts))


def priority_results_svg(rows: list[dict[str, object]], combined: dict[str, object] | None) -> str:
    width, height = 960, 560
    left, top, chart_w, chart_h = 90, 104, 730, 330
    max_ratio = 1.2

    def y_for(value: float) -> float:
        return top + chart_h - (value / max_ratio) * chart_h

    parts = [
        '<rect width="960" height="560" fill="#ffffff"/>',
        '<text x="52" y="42" class="title">Priority-lane result progression</text>',
        '<text x="52" y="66" class="subtitle">APXM/flat ratios below 1.0 favor APXM. Bars show tenant-level bootstrap intervals.</text>',
    ]
    for tick in [0, 0.25, 0.5, 0.75, 1.0]:
        yy = y_for(tick)
        parts.append(f'<line x1="{left}" y1="{yy:.1f}" x2="{left + chart_w}" y2="{yy:.1f}" class="grid"/>')
        parts.append(f'<text x="{left - 44}" y="{yy + 4:.1f}" class="small">{tick:.2f}</text>')
    parts.append(f'<line x1="{left}" y1="{y_for(1):.1f}" x2="{left + chart_w}" y2="{y_for(1):.1f}" stroke="#d64545" stroke-width="2" stroke-dasharray="6 5"/>')
    parts.append(f'<line x1="{left}" y1="{top}" x2="{left}" y2="{top + chart_h}" class="axis"/>')
    parts.append(f'<line x1="{left}" y1="{top + chart_h}" x2="{left + chart_w}" y2="{top + chart_h}" class="axis"/>')

    group_w = chart_w / len(rows)
    bar_w = 54
    for i, row in enumerate(rows):
        cx = left + group_w * i + group_w / 2
        for j, metric in enumerate(["mean", "p95"]):
            ratio = float(row[f"{metric}_ratio"])
            low = float(row[f"{metric}_ci_low"])
            high = float(row[f"{metric}_ci_high"])
            bx = cx - bar_w - 10 if metric == "mean" else cx + 10
            yy = y_for(ratio)
            bh = top + chart_h - yy
            color = "#2f80ed" if metric == "mean" else "#2f9e44"
            parts.append(f'<rect x="{bx:.1f}" y="{yy:.1f}" width="{bar_w}" height="{bh:.1f}" fill="{color}" rx="4"/>')
            parts.append(f'<line x1="{bx + bar_w/2:.1f}" y1="{y_for(low):.1f}" x2="{bx + bar_w/2:.1f}" y2="{y_for(high):.1f}" stroke="#1f2933" stroke-width="2"/>')
            parts.append(f'<line x1="{bx + 11:.1f}" y1="{y_for(low):.1f}" x2="{bx + bar_w - 11:.1f}" y2="{y_for(low):.1f}" stroke="#1f2933" stroke-width="2"/>')
            parts.append(f'<line x1="{bx + 11:.1f}" y1="{y_for(high):.1f}" x2="{bx + bar_w - 11:.1f}" y2="{y_for(high):.1f}" stroke="#1f2933" stroke-width="2"/>')
            parts.append(f'<text x="{bx + bar_w/2:.1f}" y="{yy - 8:.1f}" class="small" text-anchor="middle">{ratio:.3f}</text>')
        parts.append(f'<text x="{cx:.1f}" y="{top + chart_h + 32}" class="label" text-anchor="middle">{esc(row["label"])}</text>')
        parts.append(f'<text x="{cx:.1f}" y="{top + chart_h + 51}" class="small" text-anchor="middle">n={row["paired_rows"]}</text>')

    parts.append('<rect x="735" y="90" width="14" height="14" fill="#2f80ed" rx="3"/><text x="756" y="102" class="small">Mean focus finish</text>')
    parts.append('<rect x="735" y="112" width="14" height="14" fill="#2f9e44" rx="3"/><text x="756" y="124" class="small">p95 focus finish</text>')
    parts.append('<line x1="735" y1="142" x2="749" y2="142" stroke="#d64545" stroke-width="2" stroke-dasharray="6 5"/><text x="756" y="146" class="small">No effect</text>')
    if combined:
        tenant = combined["tenant_level"]  # type: ignore[index]
        mean = tenant["focus_node_finish_ms_mean"]  # type: ignore[index]
        p95 = tenant["focus_node_finish_ms_p95"]  # type: ignore[index]
        batch = combined["batch_level"]["focus_node_finish_ms_mean"]  # type: ignore[index]
        parts.append(
            '<text x="52" y="506" class="small">'
            f'Combined fresh-service evidence: tenant mean {float(mean["ratio"]):.3f} '
            f'[{float(mean["ci_low"]):.3f}, {float(mean["ci_high"]):.3f}], '
            f'p95 {float(p95["ratio"]):.3f} [{float(p95["ci_low"]):.3f}, {float(p95["ci_high"]):.3f}].'
            '</text>'
        )
        parts.append(
            '<text x="52" y="526" class="small">'
            f'Batch-level combined focus CI now excludes 1.0: '
            f'{float(batch["ratio"]):.3f} [{float(batch["ci_low"]):.3f}, {float(batch["ci_high"]):.3f}].'
            '</text>'
        )
    else:
        parts.append('<text x="52" y="520" class="small">Primary public result: combined fresh-service repeats, mean 0.661 [0.598, 0.729], p95 0.532 [0.320, 0.590].</text>')
    return svg(width, height, "\n".join("  " + p for p in parts))


def tradeoff_svg(tradeoffs: list[dict[str, str]], combined: dict[str, object] | None) -> str:
    width, height = 960, 560
    left, top, chart_w, chart_h = 82, 105, 770, 330
    values = [
        (
            int(r["iteration"]),
            float(r["focus_node_finish_ms_mean_ratio"]),
            float(r["batch_wall_ms_ratio"]),
        )
        for r in tradeoffs
    ]
    max_y = 1.9

    def x_for(iteration: int) -> float:
        return left + ((iteration - 1) / 9) * chart_w

    def y_for(value: float) -> float:
        return top + chart_h - (value / max_y) * chart_h

    def poly(points: Iterable[tuple[float, float]]) -> str:
        return " ".join(f"{x:.1f},{y:.1f}" for x, y in points)

    focus_points = [(x_for(i), y_for(focus)) for i, focus, _ in values]
    wall_points = [(x_for(i), y_for(wall)) for i, _, wall in values]
    parts = [
        '<rect width="960" height="560" fill="#ffffff"/>',
        '<text x="52" y="42" class="title">Fresh-service per-iteration tradeoffs</text>',
        '<text x="52" y="66" class="subtitle">Some warm batches converge or favor flat; aggregate focus-lane latency still favors APXM.</text>',
    ]
    for tick in [0, 0.5, 1.0, 1.5]:
        yy = y_for(tick)
        parts.append(f'<line x1="{left}" y1="{yy:.1f}" x2="{left + chart_w}" y2="{yy:.1f}" class="grid"/>')
        parts.append(f'<text x="{left - 42}" y="{yy + 4:.1f}" class="small">{tick:.1f}</text>')
    parts.append(f'<line x1="{left}" y1="{y_for(1):.1f}" x2="{left + chart_w}" y2="{y_for(1):.1f}" stroke="#d64545" stroke-width="2" stroke-dasharray="6 5"/>')
    parts.append(f'<line x1="{left}" y1="{top}" x2="{left}" y2="{top + chart_h}" class="axis"/>')
    parts.append(f'<line x1="{left}" y1="{top + chart_h}" x2="{left + chart_w}" y2="{top + chart_h}" class="axis"/>')
    parts.append(f'<polyline points="{poly(focus_points)}" fill="none" stroke="#2f80ed" stroke-width="3"/>')
    parts.append(f'<polyline points="{poly(wall_points)}" fill="none" stroke="#f0b429" stroke-width="3"/>')
    for i, focus, wall in values:
        parts.append(f'<circle cx="{x_for(i):.1f}" cy="{y_for(focus):.1f}" r="4.5" fill="#2f80ed"/>')
        parts.append(f'<circle cx="{x_for(i):.1f}" cy="{y_for(wall):.1f}" r="4.5" fill="#f0b429"/>')
        parts.append(f'<text x="{x_for(i):.1f}" y="{top + chart_h + 28}" class="small" text-anchor="middle">{i}</text>')
    parts.append('<rect x="670" y="90" width="14" height="14" fill="#2f80ed" rx="3"/><text x="691" y="102" class="small">Focus-node mean ratio</text>')
    parts.append('<rect x="670" y="112" width="14" height="14" fill="#f0b429" rx="3"/><text x="691" y="124" class="small">Batch-wall ratio</text>')
    if combined:
        batch = combined["batch_level"]["focus_node_finish_ms_mean"]  # type: ignore[index]
        wall = combined["batch_level"]["batch_wall_ms"]  # type: ignore[index]
        parts.append(
            '<text x="52" y="500" class="small">'
            f'Combined fresh-service focus CI is '
            f'[{float(batch["ci_low"]):.3f}, {float(batch["ci_high"]):.3f}] across 20 batches; '
            f'batch-wall ratio {float(wall["ratio"]):.3f} '
            f'[{float(wall["ci_low"]):.3f}, {float(wall["ci_high"]):.3f}].'
            '</text>'
        )
    else:
        parts.append('<text x="52" y="500" class="small">Batch-level focus CI is [0.510, 1.002]; repeat across fresh services/nodes before claiming batch-level generality.</text>')
    return svg(width, height, "\n".join("  " + p for p in parts))


def regime_svg() -> str:
    rows = [
        ("Priority lane", "combined mean 0.661, batch CI < 1", "Positive", "#dcfce7"),
        ("Constrained KV pin", "CI [0.6179, 0.7431]", "Positive", "#dcfce7"),
        ("Warm cache matrix", "CI [0.076, 0.823]", "Positive", "#dcfce7"),
        ("Mooncake default", "1.073, CI crosses 1", "Null", "#fef3c7"),
        ("LooGLE default", "1.001, CI crosses 1", "Null", "#fef3c7"),
        ("ShareGPT default", "1.223 flat wins", "Negative", "#fee2e2"),
        ("Review Council", "1.222 flat wins", "Negative", "#fee2e2"),
        ("Current pin demo", "1.064 flat wins", "Negative", "#fee2e2"),
    ]
    parts = [
        '<rect width="980" height="600" fill="#ffffff"/>',
        '<text x="52" y="42" class="title">APXM evidence is a regime map, not a universal speedup</text>',
        '<text x="52" y="66" class="subtitle">Positive cells appear when APXM hints match backend pressure and the measured metric.</text>',
    ]
    x0, y0, w, h, gap = 60, 102, 860, 46, 14
    for idx, (name, evidence, status, color) in enumerate(rows):
        y = y0 + idx * (h + gap)
        parts.append(f'<rect x="{x0}" y="{y}" width="{w}" height="{h}" rx="8" fill="{color}" stroke="#bcccdc"/>')
        parts.append(f'<text x="{x0 + 22}" y="{y + 29}" font-size="14" font-weight="700">{esc(name)}</text>')
        parts.append(f'<text x="{x0 + 270}" y="{y + 29}" class="label">{esc(evidence)}</text>')
        parts.append(f'<text x="{x0 + 720}" y="{y + 29}" class="label" font-weight="700">{esc(status)}</text>')
    parts.append('<text x="60" y="580" class="small">Claim boundary: APXM priority hints improve user-visible critical-lane latency under contention; do not claim broad batch-wall speedup.</text>')
    return svg(980, 600, "\n".join("  " + p for p in parts))


def main() -> None:
    FIG_DIR.mkdir(parents=True, exist_ok=True)
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    rows = load_summary()
    report, enhanced, tradeoffs = load_latest()
    combined = load_combined()
    figure_data = {
        "accepted_priority_lane_runs": rows,
        "latest_report": report,
        "latest_enhanced": enhanced,
        "latest_tradeoffs": tradeoffs,
        "combined_fresh_service_analysis": combined,
    }
    write(DATA_DIR / "figure-data.json", json.dumps(figure_data, indent=2) + "\n")
    write(FIG_DIR / "apxm-system-architecture.svg", architecture_svg())
    write(FIG_DIR / "priority-lane-results.svg", priority_results_svg(rows, combined))
    write(FIG_DIR / "priority-lane-batch-tradeoffs.svg", tradeoff_svg(tradeoffs, combined))
    write(FIG_DIR / "apxm-regime-map.svg", regime_svg())
    print(f"Wrote figures to {FIG_DIR.relative_to(ROOT)}")
    print(f"Wrote data summary to {(DATA_DIR / 'figure-data.json').relative_to(ROOT)}")


if __name__ == "__main__":
    main()
