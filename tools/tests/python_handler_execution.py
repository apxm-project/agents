"""Execute the Capability handler the Python fixture package ships.

The sibling of `examples/agents/coder/tests/execution.py`, for the other
frontend. A Python declaration that compiles but cannot be dispatched is the
same defect the TypeScript gate exists to catch: the compile gate calls the id
granted, so the mistake surfaces as a runtime that says the Capability does not
exist.

Three things are checked, and none of them stands in for the handler:

* Supplied with the package, a `capability.invoke` naming `normalize` returns
  the value that Python handler's own body computes.
* Supplied with no package, the same AIR is refused at admission — at the gate,
  never at dispatch.
* A Python handler passes through the same chokepoint the builtins and the
  TypeScript handlers do, so a bad argument is refused by the argument schema
  rather than reaching the handler body.
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
FIXTURES = REPOSITORY_ROOT / "tools/tests/fixtures"
PACKAGE = FIXTURES / "python-handler-package"
WORKSPACE = REPOSITORY_ROOT / ".apxm/python-handler-fixture"


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


def run(*arguments: str) -> subprocess.CompletedProcess:
    """Run the shipped CLI from the repository root."""
    return subprocess.run(
        [str(apxm_binary()), *arguments],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        env={**os.environ},
    )


def compile_program() -> Path:
    """Compile the package's program to canonical AIR.

    The package's manifest and chain are checked in and gated by
    `check-agent-packages`, so nothing is rebuilt here: this runs against the
    bytes the repository ships, and `--package` verifies that chain itself.
    """
    compiled = run("compile-service-canonical", str(PACKAGE))
    if compiled.returncode != 0:
        raise SystemExit(f"compile failed:\n{compiled.stdout}\n{compiled.stderr}")
    WORKSPACE.mkdir(parents=True, exist_ok=True)
    air = WORKSPACE / "program.air.json"
    air.write_text(compiled.stdout.strip() + "\n", encoding="utf-8")
    return air


def write_invocation_admission(air: Path, invocation_id: str) -> Path:
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
        "artifact_digest": digest(air),
        "release_digest": digest(FIXTURES / "canonical-execute.release.json"),
        "port_bindings_digest": reference["port_bindings_digest"],
        "resource_ceiling_digest": reference["resource_ceiling_digest"],
        "provenance_digest": digest(FIXTURES / "canonical-execute.provenance.json"),
    }
    path = WORKSPACE / f"{air.name.split('.')[0]}.invocation-admission.json"
    path.write_text(json.dumps(admission, indent=2) + "\n", encoding="utf-8")
    return path


def execute(air: Path, admission: Path, package: bool | Path) -> subprocess.CompletedProcess:
    """Run the compiled AIR through exact Invocation Admission."""
    arguments = [
        "--json",
        "execute-canonical",
        str(air),
        "--invocation-admission",
        str(admission),
        "--release",
        str(FIXTURES / "canonical-execute.release.json"),
        "--provenance",
        str(FIXTURES / "canonical-execute.provenance.json"),
    ]
    if package is True:
        arguments += ["--package", str(PACKAGE)]
    elif isinstance(package, Path):
        arguments += ["--package", str(package)]
    return run(*arguments)


def agent_action(action: str, package: Path) -> subprocess.CompletedProcess:
    """Run one package lifecycle action against a temporary package copy."""
    return subprocess.run(
        [str(apxm_binary()), "agent", action, str(package)],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        env={**os.environ},
    )


def copy_package(destination: Path) -> Path:
    """Copy a checked-in package so lifecycle tests never dirty the fixture."""
    package = destination / PACKAGE.name
    shutil.copytree(PACKAGE, package)
    return package


def set_package_permission(package: Path, decision: str) -> None:
    """Add one typed package-layer permission to a temporary package."""
    manifest = package / "agent.toml"
    manifest.write_text(
        manifest.read_text(encoding="utf-8")
        + f'\n[permissions]\nnormalize = {{ decision = "{decision}", reason = "Package test policy." }}\n',
        encoding="utf-8",
    )


def capability_results(result: dict) -> list[str]:
    """The result text every completed capability.invoke returned."""
    results = []
    for outcome in result["results"]["node_outcomes"]:
        if outcome.get("kind") != "capability.invoke":
            continue
        assert outcome["outcome"]["status"] == "completed", outcome
        results.append(outcome["outcome"]["result"])
    return results


def test_a_shipped_python_handler_runs_when_the_package_supplies_it() -> None:
    """The shipped Capability returns what its own Python handler body computes."""
    air = compile_program()
    admission = write_invocation_admission(air, "invocation.python-handler-fixture.1")
    completed = execute(air, admission, package=True)
    if completed.returncode != 0:
        raise SystemExit(f"execute-canonical failed:\n{completed.stdout}\n{completed.stderr}")
    result = json.loads(completed.stdout)
    assert result["status"] == "completed", result
    assert result["commit"]["status"] == "committed", result
    assert result["stats"]["failed_nodes"] == 0, result

    results = capability_results(result)
    assert len(results) == 1, results
    # `words` and the collapsed label are computed by the package's handler body
    # and by nothing else, so reading them back is reading the handler's work.
    assert json.loads(results[0]) == {
        "label": "shipped by python",
        "words": 3,
        "mutates": False,
    }, results


def test_the_same_program_is_refused_at_the_gate_without_the_package() -> None:
    """With no supplied implementation the refusal is admission, not dispatch."""
    air = WORKSPACE / "program.air.json"
    admission = WORKSPACE / "program.invocation-admission.json"
    completed = execute(air, admission, package=False)
    assert completed.returncode != 0, completed.stdout
    combined = completed.stdout + completed.stderr
    assert "cannot admit node" in combined, combined
    assert "granted by nothing this invocation admits" in combined, combined


def test_an_undeclared_argument_is_refused_by_the_shared_chokepoint() -> None:
    """A Python handler is gated by the argument schema, not by its own body.

    The declaration closes the object, so an argument it never declared is
    refused before the handler runs. That refusal is the capability system's,
    which is the whole point of registering a package handler as an executor
    rather than dispatching it down a path of its own.
    """
    air = json.loads((WORKSPACE / "program.air.json").read_text())
    for assembly in air["value_assemblies"]:
        fields = assembly.get("expression", {}).get("fields")
        if fields is None:
            continue
        fields.append({"name": "undeclared", "value": {"kind": "string", "value": "x"}})
    path = WORKSPACE / "undeclared.air.json"
    path.write_text(json.dumps(air) + "\n", encoding="utf-8")
    admission = write_invocation_admission(path, "invocation.python-handler-fixture.2")
    completed = execute(path, admission, package=True)
    combined = completed.stdout + completed.stderr
    assert "Input validation failed" in combined, combined
    assert "'undeclared' was unexpected" in combined, combined
    assert "shipped by python" not in combined, combined


def test_package_integrity_refuses_a_post_build_edit() -> None:
    """The package lifecycle rejects bytes changed after integrity sealing."""
    with tempfile.TemporaryDirectory(prefix="apxm-python-package-") as directory:
        package = copy_package(Path(directory))
        verified = agent_action("verify", package)
        assert verified.returncode == 0, verified.stdout + verified.stderr

        manifest = package / "agent.toml"
        manifest.write_text(
            manifest.read_text(encoding="utf-8") + "\n# changed after build\n",
            encoding="utf-8",
        )
        refused = agent_action("verify", package)
        combined = refused.stdout + refused.stderr
        assert refused.returncode != 0, combined
        assert "failed integrity verification" in combined, combined


def test_package_ask_is_projected_and_fails_without_consent() -> None:
    """A typed package Ask reaches the manifest and stops before the worker."""
    with tempfile.TemporaryDirectory(prefix="apxm-python-package-") as directory:
        package = copy_package(Path(directory))
        set_package_permission(package, "ask")
        built = agent_action("build", package)
        assert built.returncode == 0, built.stdout + built.stderr

        manifest = json.loads(
            (package / "capabilities/handlers/tools.json").read_text(encoding="utf-8")
        )
        [handler] = manifest["handlers"]
        assert handler["name"] == "normalize"
        assert handler["requires_approval"] is True, manifest
        verified = agent_action("verify", package)
        assert verified.returncode == 0, verified.stdout + verified.stderr

        air = WORKSPACE / "program.air.json"
        admission = WORKSPACE / "program.invocation-admission.json"
        completed = execute(air, admission, package)
        combined = completed.stdout + completed.stderr
        assert completed.returncode == 0, combined
        result = json.loads(completed.stdout)
        assert result["status"] == "completed", result
        [outcome] = result["results"]["node_outcomes"]
        assert outcome["outcome"]["status"] == "failed", result
        assert "ask" in outcome["outcome"]["message"], result
        assert "shipped by python" not in outcome["outcome"]["message"], result


def test_package_deny_refuses_emitting_an_executable_handler() -> None:
    """A typed package Deny cannot be hidden by emitting a worker descriptor."""
    with tempfile.TemporaryDirectory(prefix="apxm-python-package-") as directory:
        package = copy_package(Path(directory))
        set_package_permission(package, "deny")
        refused = agent_action("build", package)
        combined = refused.stdout + refused.stderr
        assert refused.returncode != 0, combined
        assert "denied and cannot be emitted" in combined, combined


def main() -> None:
    tests = [
        test_a_shipped_python_handler_runs_when_the_package_supplies_it,
        test_the_same_program_is_refused_at_the_gate_without_the_package,
        test_an_undeclared_argument_is_refused_by_the_shared_chokepoint,
        test_package_integrity_refuses_a_post_build_edit,
        test_package_ask_is_projected_and_fails_without_consent,
        test_package_deny_refuses_emitting_an_executable_handler,
    ]
    for test in tests:
        test()
        print(f"PASS {test.__name__}")


if __name__ == "__main__":
    main()
