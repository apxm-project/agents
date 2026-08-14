"""Execute the Capability handlers the Coder package ships.

A package that ships handlers and a runtime that cannot dispatch them is worse
than a package that ships none: the compile gate calls the id granted, so the
mistake surfaces as a runtime that says the Capability does not exist. This
script holds the two halves together end to end, through the shipped
`apxm execute-canonical` and nothing else.

Three things are checked, and none of them stands in for the handler:

* Supplied with the package, a `capability.invoke` naming `edit` or `test`
  returns the value that handler's own body computes.
* Supplied with no package, the same AIR is refused at admission — at the gate,
  never at dispatch, which is the failure mode this whole surface exists to
  prevent.
* Coder's own `src/main.ts` clears the same admission gate its two shipped
  Capabilities used to fail, so the grant the compile gate hands out is one the
  runtime can honour rather than one it will later deny.
"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

EXAMPLE_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = EXAMPLE_ROOT.parents[2]
FIXTURES = REPOSITORY_ROOT / "tools/tests/fixtures"
WORKSPACE = REPOSITORY_ROOT / ".apxm/coder-example"
PACKAGE = EXAMPLE_ROOT.relative_to(REPOSITORY_ROOT)


def digest(path: Path) -> str:
    """The `sha256:<hex>` an Invocation Admission pins a document by."""
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def apxm_binary() -> Path:
    """The built CLI the dekk command puts on disk before running this."""
    target = subprocess.run(
        [sys.executable, "tools/scripts/cargo.py", "target-dir"],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    binary = Path(target) / "debug/apxm"
    if not binary.is_file():
        raise SystemExit(f"missing apxm binary: {binary}")
    return binary


def compile_air(script: str, name: str) -> Path:
    """Compile one of the package's authored programs to canonical AIR."""
    completed = subprocess.run(
        ["npm", "--prefix", str(PACKAGE), "run", script],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        raise SystemExit(f"{script} failed:\n{completed.stdout}\n{completed.stderr}")
    WORKSPACE.mkdir(parents=True, exist_ok=True)
    path = WORKSPACE / f"{name}.air.json"
    path.write_text(completed.stdout.strip().splitlines()[-1] + "\n", encoding="utf-8")
    return path


def write_invocation_admission(air_path: Path, invocation_id: str) -> Path:
    """Pin this run to the AIR, release, and provenance documents it executes.

    The port-binding and resource-ceiling digests are the canonical local
    profile's own, so they are read from the checked-in fixture rather than
    restated here; everything else is computed from the bytes being admitted.
    """
    reference = json.loads(
        (FIXTURES / "canonical-capability-execute.invocation-admission.json").read_text()
    )
    admission = {
        "schema_version": "apxm.invocation-admission",
        "invocation_id": invocation_id,
        "artifact_digest": digest(air_path),
        "release_digest": digest(FIXTURES / "canonical-execute.release.json"),
        "port_bindings_digest": reference["port_bindings_digest"],
        "resource_ceiling_digest": reference["resource_ceiling_digest"],
        "provenance_digest": digest(FIXTURES / "canonical-execute.provenance.json"),
    }
    path = air_path.with_suffix("").with_suffix(".invocation-admission.json")
    path.write_text(json.dumps(admission, indent=2) + "\n", encoding="utf-8")
    return path


def execute(air_path: Path, admission_path: Path, package: bool) -> subprocess.CompletedProcess:
    """Run the compiled AIR through exact Invocation Admission."""
    command = [
        str(apxm_binary()),
        "--json",
        "execute-canonical",
        str(air_path),
        "--invocation-admission",
        str(admission_path),
        "--release",
        str(FIXTURES / "canonical-execute.release.json"),
        "--provenance",
        str(FIXTURES / "canonical-execute.provenance.json"),
    ]
    if package:
        command += ["--package", str(PACKAGE)]
    return subprocess.run(
        command,
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        env={**os.environ},
    )


def capability_results(result: dict) -> dict[str, str]:
    """The result text every completed capability.invoke returned, by node."""
    results = {}
    for outcome in result["results"]["node_outcomes"]:
        if outcome.get("kind") != "capability.invoke":
            continue
        assert outcome["outcome"]["status"] == "completed", outcome
        results[outcome["node_id"]] = outcome["outcome"]["result"]
    return results


def test_a_shipped_handler_runs_when_the_package_supplies_it() -> None:
    """Both shipped Capabilities return what their own handler bodies compute."""
    air = compile_air("compile:proposals", "proposals")
    admission = write_invocation_admission(air, "invocation.coder-proposals.1")
    completed = execute(air, admission, package=True)
    if completed.returncode != 0:
        raise SystemExit(f"execute-canonical failed:\n{completed.stdout}\n{completed.stderr}")
    result = json.loads(completed.stdout)
    assert result["status"] == "completed", result
    assert result["commit"]["status"] == "committed", result
    assert result["stats"]["failed_nodes"] == 0, result

    results = capability_results(result)
    assert len(results) == 2, results
    values = [json.loads(text) for text in results.values()]
    edit = next(value for value in values if "file_path" in value)
    prepared = next(value for value in values if "command" in value)
    # `mutates` and `executes` are computed by the package's handler bodies and
    # by nothing else, so reading them back is reading the handler's own work.
    assert edit == {
        "file_path": "src/main.ts",
        "before": "const answer = 1;",
        "after": "const answer = 2;",
        "mutates": False,
    }, edit
    assert prepared == {"command": "npm test", "executes": False, "mutates": False}, prepared


def test_the_same_program_is_refused_at_the_gate_without_the_package() -> None:
    """With no supplied implementation the refusal is admission, not dispatch."""
    air = WORKSPACE / "proposals.air.json"
    admission = WORKSPACE / "proposals.invocation-admission.json"
    completed = execute(air, admission, package=False)
    assert completed.returncode != 0, completed.stdout
    combined = completed.stdout + completed.stderr
    assert "cannot admit node" in combined, combined
    assert "granted by nothing this invocation admits" in combined, combined


def test_the_agent_itself_clears_the_capability_admission_gate() -> None:
    """Coder's own program no longer names a Capability nothing can dispatch.

    It still cannot run to completion here: every Capability argument in
    `src/main.ts` derives from the program's input or from a model response,
    and neither is bound by canonical local execution. What it must not do any
    more is fail because `edit` and `test` resolve to nothing.
    """
    air = compile_air("compile", "coder")
    admission = write_invocation_admission(air, "invocation.coder-agent.1")
    completed = execute(air, admission, package=True)
    combined = completed.stdout + completed.stderr
    assert "granted by nothing this invocation admits" not in combined, combined
    assert "'edit'" not in combined and "'test'" not in combined, combined


def main() -> None:
    tests = [
        test_a_shipped_handler_runs_when_the_package_supplies_it,
        test_the_same_program_is_refused_at_the_gate_without_the_package,
        test_the_agent_itself_clears_the_capability_admission_gate,
    ]
    for test in tests:
        test()
        print(f"PASS {test.__name__}")


if __name__ == "__main__":
    main()
