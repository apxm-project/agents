#!/usr/bin/env python3
"""Run the focused source/frontend/compiler acceptance gates with one report."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Callable


REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_REPORT = (
    REPO_ROOT / ".apxm" / "compiler" / "source-compiler-acceptance" / "report.json"
)


@dataclass(frozen=True)
class AcceptanceStep:
    """One focused gate that proves part of the source/compiler contract."""

    gate_id: str
    description: str
    command: tuple[str, ...]


@dataclass(frozen=True)
class StepReport:
    """One executed gate and its bounded outcome."""

    gate_id: str
    description: str
    command: tuple[str, ...]
    status: str
    returncode: int
    duration_seconds: float
    log_path: str


def acceptance_steps() -> tuple[AcceptanceStep, ...]:
    """The exact gates that prove the issue-35 source/compiler acceptance."""

    return (
        AcceptanceStep(
            "owner-descriptor",
            "owner vectors, descriptor digests, and schema correspondence stay exact",
            ("dekk", "agents", "owner-descriptor"),
        ),
        AcceptanceStep(
            "check-contract-codegen",
            "generated contract clients stay byte-identical to the checked-in owner contracts",
            ("dekk", "agents", "check-contract-codegen"),
        ),
        AcceptanceStep(
            "check-frontend-surface",
            "Python and TypeScript export only the declared source-first frontend surface",
            ("dekk", "agents", "check-frontend-surface"),
        ),
        AcceptanceStep(
            "test-source-port",
            "the submitted-source compile port stays fail-closed across positive, negative, and boundary cases",
            ("dekk", "agents", "test-source-port"),
        ),
        AcceptanceStep(
            "test-program",
            "FrontendGraph, AIR, artifact, and source-map lowering stay closed and deterministic",
            ("dekk", "agents", "test-program"),
        ),
        AcceptanceStep(
            "test-compiler",
            "the compiler pipeline preserves the canonical source-to-artifact contract",
            ("dekk", "agents", "test-compiler"),
        ),
        AcceptanceStep(
            "check-frontend-parity",
            "Python and TypeScript authoring compile to closed-semantics-equivalent AIR",
            ("dekk", "agents", "check-frontend-parity"),
        ),
        AcceptanceStep(
            "test-external-source-package",
            "an external TypeScript package compiles through the public source boundary without aliases or fallbacks",
            ("dekk", "agents", "test-external-source-package"),
        ),
        AcceptanceStep(
            "compile-service-canonical",
            "the canonical CLI compile path emits apxm.air.v1 from repository-owned source",
            ("dekk", "agents", "compile-service-canonical"),
        ),
        AcceptanceStep(
            "execute-canonical",
            "the checked-in canonical apxm.air.v1 fixture executes through the canonical runtime",
            ("dekk", "agents", "execute-canonical"),
        ),
    )


def render_path(path: Path) -> str:
    """Render a report path relative to the repo when possible."""

    try:
        return str(path.relative_to(REPO_ROOT))
    except ValueError:
        return str(path)


def run_step(
    step: AcceptanceStep,
    logs_dir: Path,
    *,
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> StepReport:
    """Run one gate through the authority surface and record its outcome."""

    print(f"\n==> {step.gate_id}: {step.description}", flush=True)
    print(f"    $ {' '.join(step.command)}", flush=True)
    started = time.perf_counter()
    completed = runner(
        step.command,
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    duration = round(time.perf_counter() - started, 3)
    logs_dir.mkdir(parents=True, exist_ok=True)
    log_path = logs_dir / f"{step.gate_id}.log"
    combined_output = ""
    if completed.stdout:
        combined_output += completed.stdout
    if completed.stderr:
        if combined_output and not combined_output.endswith("\n"):
            combined_output += "\n"
        combined_output += completed.stderr
    log_path.write_text(combined_output, encoding="utf-8")
    if completed.stdout:
        print(completed.stdout, end="" if completed.stdout.endswith("\n") else "\n")
    if completed.stderr:
        print(completed.stderr, end="" if completed.stderr.endswith("\n") else "\n", file=sys.stderr)
    return StepReport(
        gate_id=step.gate_id,
        description=step.description,
        command=step.command,
        status="passed" if completed.returncode == 0 else "failed",
        returncode=completed.returncode,
        duration_seconds=duration,
        log_path=render_path(log_path),
    )


def run_acceptance(
    logs_dir: Path,
    *,
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> dict[str, object]:
    """Run every source/compiler gate and return one structured report."""

    steps = [run_step(step, logs_dir, runner=runner) for step in acceptance_steps()]
    passed = all(step.status == "passed" for step in steps)
    return {
        "schema_version": "apxm.source-compiler-acceptance.v1",
        "generated_at": datetime.now(UTC).replace(microsecond=0).isoformat(),
        "issue_scope": {
            "issue": 35,
            "plan_tags": ["P-002", "P-003", "P-004", "P-005", "G1", "G2"],
        },
        "overall_status": "passed" if passed else "failed",
        "steps": [asdict(step) for step in steps],
    }


def write_report(report: dict[str, object], path: Path) -> None:
    """Write one JSON report under the repository artifact root."""

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def parse_args() -> argparse.Namespace:
    """Parse the report destination for one acceptance run."""

    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--report-json",
        type=Path,
        default=DEFAULT_REPORT,
        help="write the structured report to this JSON file",
    )
    return parser.parse_args()


def main() -> int:
    """Run the acceptance bundle, write one report, and return its status."""

    args = parse_args()
    report_path = args.report_json
    if not report_path.is_absolute():
        report_path = REPO_ROOT / report_path

    report = run_acceptance(report_path.parent / "logs")
    write_report(report, report_path)

    print(f"\nWrote report to {render_path(report_path)}", flush=True)
    if report["overall_status"] != "passed":
        print("FAILED: one or more source/compiler acceptance gates failed", file=sys.stderr)
        return 1
    print("OK: source/compiler acceptance gates passed", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
