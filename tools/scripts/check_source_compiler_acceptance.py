#!/usr/bin/env python3
"""Run the focused source/frontend/compiler acceptance gates with one report."""

from __future__ import annotations

import argparse
import hashlib
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
    log_digest: str
    error: str | None = None


def acceptance_steps() -> tuple[AcceptanceStep, ...]:
    """The exact gates that prove the issue-35 source/compiler acceptance."""

    return (
        AcceptanceStep(
            "check-frontend-codegen",
            "AIS and frontend generated metadata stay byte-identical to the owner definitions",
            ("dekk", "agents", "check-frontend-codegen"),
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
            "test-program-source",
            "FrontendGraph, AIR, artifact, and source-map lowering stay closed and deterministic",
            ("dekk", "agents", "test-program-source"),
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
            "test-typescript-frontend",
            "the public TypeScript frontend package stays closed, typed, and deterministic",
            ("dekk", "agents", "test-typescript-frontend"),
        ),
        AcceptanceStep(
            "check-source-compiler-boundary",
            "source and compiler paths contain no downstream dependency or rejected alternate AIR path",
            ("dekk", "agents", "check-source-compiler-boundary"),
        ),
        AcceptanceStep(
            "compile-service-canonical",
            "the canonical CLI compile path emits apxm.air.v1 from repository-owned source",
            ("dekk", "agents", "compile-service-canonical"),
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
    error: str | None = None
    try:
        completed = runner(
            step.command,
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
        )
    except OSError as exception:
        completed = subprocess.CompletedProcess(
            step.command,
            127,
            stdout="",
            stderr=str(exception),
        )
        error = str(exception)
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
    combined_output = "\n".join(line.rstrip() for line in combined_output.splitlines())
    if combined_output:
        combined_output += "\n"
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
        log_digest="sha256:" + hashlib.sha256(combined_output.encode("utf-8")).hexdigest(),
        error=error,
    )


def run_acceptance(
    logs_dir: Path,
    *,
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> dict[str, object]:
    """Run every source/compiler gate and return one structured report."""

    steps = [run_step(step, logs_dir, runner=runner) for step in acceptance_steps()]
    passed = all(step.status == "passed" for step in steps)
    revision = subprocess.run(
        ("git", "rev-parse", "HEAD"),
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    return {
        "schema_version": "apxm.source-compiler-acceptance.v1",
        "generated_at": datetime.now(UTC).replace(microsecond=0).isoformat(),
        "source_revision": revision,
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
