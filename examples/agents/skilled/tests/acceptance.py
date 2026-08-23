"""Compile the Skilled example and execute it, reading both declared skills.

A `Skill` declaration is only worth anything if the instructions it names can be
read at execution, through the same admitted path every other capability takes.
This script proves that end to end: it compiles the package's authored source to
canonical AIR, plays the host by publishing the program's declared skills into a
local discovery root, and runs the AIR through the shipped
`apxm execute-canonical`. Nothing here hand-authors AIR and nothing stands in
for the capability — `read_skill` returns the real bodies.

Publishing is host work by design. The runtime reads instructions from the
discovery roots its host configures, and a package's skills are not a root until
someone publishes them into one; the two `instruction_source` branches simply
say where the host finds the text. A file-carried skill is copied out of the
package, and a program-written one is materialized from the source the artifact
digest already covers.
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

EXAMPLE_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = EXAMPLE_ROOT.parents[2]
FIXTURES = REPOSITORY_ROOT / "tools/tests/fixtures"
WORKSPACE = REPOSITORY_ROOT / ".apxm/skilled-example"

#: The local-tier discovery root the canonical composition configures. It is
#: repository-relative, so this script publishes into the directory the CLI will
#: read when it runs from the repository root.
LOCAL_SKILL_ROOT = REPOSITORY_ROOT / ".apxm/skills"


def digest(path: Path) -> str:
    """The `sha256:<hex>` an Invocation Admission pins a document by."""
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def load_program():
    """Import the shipped example through the installed generic frontend."""
    sys.path.insert(0, str(EXAMPLE_ROOT / "python"))
    from agent import SkilledExample  # noqa: PLC0415 — the example is the subject

    return SkilledExample


def typescript_source():
    """Compile the TypeScript twin, when the TypeScript frontend is built.

    `test-skill-example` builds only the Python frontend, so the twin is
    compiled whenever the TypeScript one is on disk — which is what
    `test-frontend-examples` and `build-example-artifacts` leave behind.
    """
    if not (REPOSITORY_ROOT / "crates/compiler/frontend/typescript/dist/index.js").is_file():
        return None
    for command in (
        ["npm", "--prefix", str(EXAMPLE_ROOT), "ci", "--ignore-scripts",
         "--no-audit", "--no-fund"],
        ["npm", "--prefix", str(EXAMPLE_ROOT), "run", "build"],
    ):
        subprocess.run(command, cwd=REPOSITORY_ROOT, check=True, capture_output=True)
    completed = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            "import { buildSkilled } from './dist/agent.js';"
            "const p = buildSkilled();"
            "console.log(JSON.stringify({"
            "graph: p.frontendGraph(), air: p.canonicalAir(), diagnostics: p.diagnostics()"
            "}));",
        ],
        cwd=EXAMPLE_ROOT,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        raise SystemExit(f"TypeScript twin failed:\n{completed.stdout}\n{completed.stderr}")
    payload = json.loads(completed.stdout)
    assert payload["diagnostics"] is None, payload["diagnostics"]
    return payload["graph"], payload["air"]


def publish_declared_skills(graph: dict) -> list[str]:
    """Publish the skills only a host can publish.

    A file-carried skill needs no publishing: it lives at
    ``skills/<id>/SKILL.md`` inside the package, and ``--package`` makes the
    package's own directory a discovery root, so the bytes the integrity chain
    covers are the bytes served.

    A program-written skill has no file. Its instructions exist only in the
    compiled source bundle, which is digest-collapsed before execution, so the
    host materialises the body and supplies the card fields from the identity it
    already knows. That is the one case still needing a local root.
    """
    published: list[str] = []
    # A stale body from an earlier run would resolve in two roots at once, and
    # read_skill refuses an ambiguous id rather than guessing.
    if LOCAL_SKILL_ROOT.is_dir():
        shutil.rmtree(LOCAL_SKILL_ROOT)
    for requirement in graph["skill_requirements"]:
        skill_id = requirement["skill_id"]
        source = requirement["instruction_source"]
        if source["kind"] == "entry":
            published.append(skill_id)
            continue
        directory = LOCAL_SKILL_ROOT / skill_id
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "SKILL.md").write_text(
            f"---\nname: {skill_id}\n"
            f"description: Instructions the Skilled example writes in its own source.\n"
            f"---\n\n{source['text']}",
            encoding="utf-8",
        )
        published.append(skill_id)
    return published


def write_invocation_admission(air_path: Path) -> Path:
    """Pin this run to the AIR, release, and provenance documents it executes.

    The port-binding and resource-ceiling digests are the canonical local
    profile's own, so they are read from the checked-in fixture rather than
    restated here; everything else is computed from the bytes being admitted.
    """
    reference = json.loads(
        (FIXTURES / "canonical-skill-execute.invocation-admission.json").read_text()
    )
    admission = {
        "schema_version": "apxm.invocation-admission",
        "invocation_id": "invocation.skilled-example.1",
        "artifact_digest": digest(air_path),
        "release_digest": digest(FIXTURES / "canonical-execute.release.json"),
        "port_bindings_digest": reference["port_bindings_digest"],
        "resource_ceiling_digest": reference["resource_ceiling_digest"],
        "provenance_digest": digest(FIXTURES / "canonical-execute.provenance.json"),
    }
    path = WORKSPACE / "skilled.invocation-admission.json"
    path.write_text(json.dumps(admission, indent=2) + "\n", encoding="utf-8")
    return path


def apxm_binary() -> Path:
    """The built CLI the dekk command puts on disk before running this."""
    target = subprocess.run(
        [sys.executable, "tools/scripts/cargo.py", "target-dir"],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    binary = Path(target) / "debug/apxm-dev"
    if not binary.is_file():
        raise SystemExit(f"missing apxm binary: {binary}")
    return binary


def execute(air_path: Path, admission_path: Path) -> dict:
    """Run the compiled AIR through exact Invocation Admission."""
    completed = subprocess.run(
        [
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
            # Publishes the package's own skills/ as a discovery root, so a
            # file-carried Skill resolves from the package that ships it.
            "--package",
            str(EXAMPLE_ROOT),
        ],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        env={**os.environ},
    )
    if completed.returncode != 0:
        raise SystemExit(f"execute-canonical failed:\n{completed.stdout}\n{completed.stderr}")
    return json.loads(completed.stdout)


def skill_bodies(result: dict) -> list[str]:
    """The instruction documents every completed capability.invoke returned."""
    bodies = []
    for outcome in result["results"]["node_outcomes"]:
        if outcome.get("kind") != "capability.invoke":
            continue
        assert outcome["outcome"]["status"] == "completed", outcome
        bodies.append(outcome["outcome"]["result"])
    return bodies


def check_language(language: str, graph: dict, air: str) -> None:
    """Publish one source's declared skills and execute its compiled AIR."""
    # The declaration is visible where it has to be: in the graph, and in the
    # artifact's requirements as the authority to read instructions.
    declared = {
        requirement["skill_id"]: requirement["instruction_source"]["kind"]
        for requirement in graph["skill_requirements"]
    }
    assert declared == {"review": "entry", "tone": "inline"}, (language, declared)
    assert [
        requirement["capability_ref"] for requirement in graph["capability_requirements"]
    ] == ["read_skill"], language

    WORKSPACE.mkdir(parents=True, exist_ok=True)
    air_path = WORKSPACE / f"skilled-{language}.air.json"
    air_path.write_text(air, encoding="utf-8")

    published = publish_declared_skills(graph)
    assert sorted(published) == ["review", "tone"], published

    result = execute(air_path, write_invocation_admission(air_path))
    assert result["status"] == "completed", result
    assert result["commit"]["status"] == "committed", result

    bodies = skill_bodies(result)
    assert len(bodies) == 2, bodies
    joined = "\n".join(bodies)
    # The file-carried skill returns the document the package ships.
    assert "Read the change end to end before judging any part of it." in joined, joined
    # The program-written one returns the text the source bundle carries.
    assert "Answer in the register the author wrote in." in joined, joined


def main() -> None:
    """Compile, publish, execute, and check that both skills were read."""
    program = load_program()
    assert program.diagnostics() is None, program.diagnostics()
    slots = [
        requirement["typed_port_slot"]
        for requirement in program.artifact()["artifact_semantic_requirements"]
    ]
    assert slots == ["read_skill"], slots
    check_language("python", program.frontend_graph(), program.canonical_air())

    exercised = ["Python"]
    twin = typescript_source()
    if twin is not None:
        check_language("typescript", *twin)
        exercised.append("TypeScript")

    print(
        "PASS the Skilled example declares two skills and reads both through "
        f"read_skill ({', '.join(exercised)})"
    )


if __name__ == "__main__":
    main()
