"""Packed Python frontend clean-consumer imports and removed-surface absence."""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

PACKAGE_DIR = Path(__file__).resolve().parents[1]
REMOVED_MODULES = ("conversational", "gao")


def run(*args: str, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=cwd,
        check=False,
        capture_output=True,
        text=True,
    )


def test_packed_wheel_exposes_only_generic_agent_program_surface() -> None:
    with tempfile.TemporaryDirectory() as raw_temp:
        temp = Path(raw_temp)
        wheel_dir = temp / "wheel"
        wheel_dir.mkdir()
        built = run(
            sys.executable,
            "-m",
            "pip",
            "wheel",
            "--no-deps",
            "--no-build-isolation",
            "--wheel-dir",
            str(wheel_dir),
            str(PACKAGE_DIR),
            cwd=temp,
        )
        assert built.returncode == 0, built.stderr
        wheel = next(wheel_dir.glob("*.whl"))
        assert "none-any" not in wheel.name

        venv = temp / "venv"
        created = run(sys.executable, "-m", "venv", str(venv), cwd=temp)
        assert created.returncode == 0, created.stderr
        python = venv / "bin" / "python"
        installed = run(
            str(python),
            "-m",
            "pip",
            "install",
            "--no-deps",
            str(wheel),
            cwd=temp,
        )
        assert installed.returncode == 0, installed.stderr

        generic_import = run(
            str(python),
            "-c",
            (
                "from apxm_program import AgentProgram, Hook, StructuredTaskScope; "
                "program = AgentProgram(program_id='clean', input_type_ref='Input', "
                "output_type_ref='Output'); "
                "empty = lambda body: None; "
                "program.branch('region.branch', empty, empty); "
                "program.switch('region.switch', (empty,)); "
                "program.loop('region.loop', empty); "
                "program.parallel('region.parallel', empty); "
                "program.try_catch('region.try', 'region.catch', empty, empty); "
                "program.throw_region('region.throw'); "
                "program.return_region('region.return'); "
                "program.yield_region('region.yield'); "
                "assert program.build_graph()['schema_version'] == "
                "'apxm.frontend-graph.v1'; "
                "assert program.verify() is None"
            ),
            cwd=temp,
        )
        assert generic_import.returncode == 0, generic_import.stderr

        root_absence = run(
            str(python),
            "-c",
            (
                "import apxm_program; "
                "removed = ('ConversationalAgent', 'Gao', 'TurnSpec', "
                "'SpecialistComposition'); "
                "assert all(not hasattr(apxm_program, name) for name in removed)"
            ),
            cwd=temp,
        )
        assert root_absence.returncode == 0, root_absence.stderr

        for module in REMOVED_MODULES:
            subpath_absence = run(
                str(python),
                "-c",
                f"import apxm_program.{module}",
                cwd=temp,
            )
            assert subpath_absence.returncode != 0
