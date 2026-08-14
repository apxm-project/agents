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

        golden = temp / "clean_agent.py"
        golden.write_text(
            "from apxm_program import Agent, Context, Model, Tool\n"
            "\n"
            "Weather = Tool[object, object]('weather.capability')\n"
            "Planner = Model[object, object]('planner.model')\n"
            "\n"
            "\n"
            "@Context\n"
            "class Trip:\n"
            "    legs: tuple = ()\n"
            "\n"
            "\n"
            "@Agent(input='TripRequest', output='TripPlan', context=Trip)\n"
            "async def Plan(agent, request):\n"
            "    while True:\n"
            "        if request is not None:\n"
            "            forecast = await Weather(request)\n"
            "        plan = await Planner(request)\n"
            "        agent.context = Trip()\n"
            "        return plan\n"
            "\n"
            "\n"
            "if __name__ == '__main__':\n"
            "    graph = Plan.frontend_graph()\n"
            "    assert graph['schema_version'] == 'apxm.frontend-graph'\n"
            "    assert Plan.diagnostics() is None\n"
            "    kinds = {c['intent_kind'] for c in graph['call_intents']}\n"
            "    assert kinds == {'tool_invocation', 'model_invocation'}, kinds\n"
            "    air = __import__('json').loads(Plan.canonical_air())\n"
            "    ops = {op['op'] for op in air['semantic_operations']}\n"
            "    assert ops == {'capability.invoke', 'model.call'}, ops\n"
        )
        clean_author = run(str(python), str(golden), cwd=temp)
        assert clean_author.returncode == 0, clean_author.stderr

        root_absence = run(
            str(python),
            "-c",
            (
                "import apxm_program; "
                "removed = ('AgentProgram', 'AgentFacade', 'FIVE_OPS', "
                "'OP_MODEL_CALL', 'OP_CAPABILITY_INVOKE', 'ConversationalAgent', "
                "'Gao', 'canonical_air_json', 'lower', 'verify', "
                "'StructuredTaskScope'); "
                "assert all(not hasattr(apxm_program, name) for name in removed), "
                "[n for n in removed if hasattr(apxm_program, n)]"
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
