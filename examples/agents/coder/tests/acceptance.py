"""Deterministic acceptance fixture for the coder and explorer packages."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path


PROFILE_ROOT = Path(__file__).resolve().parents[1]
EXPLORER_ROOT = PROFILE_ROOT.parent / "explorer"
FIXTURE_ROOT = PROFILE_ROOT.parent / "coder-fixture"


def load_toml(path: Path) -> dict:
    return tomllib.loads(path.read_text())


def capabilities(root: Path) -> dict[str, dict]:
    result = {}
    for path in sorted((root / "capabilities").glob("*/capability.toml")):
        result[path.parent.name] = load_toml(path)
    return result


def permissions(root: Path) -> dict[str, dict]:
    result = {}
    for path in sorted((root / "capabilities").glob("*/permission.toml")):
        result[path.parent.name] = load_toml(path)
    return result


def assert_inside(root: Path, relative: str) -> Path:
    candidate = (root / relative).resolve()
    candidate.relative_to(root.resolve())
    return candidate


def require_approval(root: Path, capability: str, approved: bool) -> None:
    policy = permissions(root)[capability]
    if policy["decision"] == "ask" and not approved:
        raise PermissionError(f"{capability} requires host approval")


def read_file(root: Path, relative: str) -> str:
    path = assert_inside(root, relative)
    if "read" not in capabilities(PROFILE_ROOT):
        raise PermissionError("read is not declared")
    return path.read_text()


def propose_edit(root: Path, relative: str, before: str, after: str) -> dict:
    path = assert_inside(root, relative)
    if "edit" not in capabilities(PROFILE_ROOT):
        raise PermissionError("edit is not declared")
    current = path.read_text()
    if current != before:
        raise ValueError("edit source does not match the explored file")
    return {"file_path": relative, "before": before, "after": after, "mutates": False}


def apply_edit(root: Path, proposal: dict, approved: bool) -> None:
    require_approval(PROFILE_ROOT, "write", approved)
    path = assert_inside(root, str(proposal["file_path"]))
    if path.read_text() != proposal["before"]:
        raise ValueError("workspace changed after exploration")
    path.write_text(str(proposal["after"]))


def prepare_test() -> dict:
    return {"command": [sys.executable, "-m", "unittest", "discover", "-s", "tests"]}


def run_test(root: Path, approved: bool) -> subprocess.CompletedProcess[str]:
    require_approval(PROFILE_ROOT, "bash", approved)
    return subprocess.run(
        prepare_test()["command"],
        cwd=root,
        text=True,
        capture_output=True,
        check=False,
    )


def save_checkpoint(path: Path, step: str) -> None:
    path.write_text(json.dumps({"step": step}, sort_keys=True) + "\n")


def load_checkpoint(path: Path) -> dict:
    return json.loads(path.read_text())


def copied_fixture() -> Path:
    temp_root = Path(tempfile.mkdtemp(prefix="apxm-coder-fixture-"))
    target = temp_root / "repo"
    shutil.copytree(FIXTURE_ROOT, target)
    return target


def test_profile_declares_controls() -> None:
    coder = load_toml(PROFILE_ROOT / "agent.toml")
    hierarchy = load_toml(PROFILE_ROOT / "hierarchy.toml")
    coder_caps = capabilities(PROFILE_ROOT)
    coder_perms = permissions(PROFILE_ROOT)
    assert coder["id"] == "coder"
    assert hierarchy["permitted_children"] == ["explorer"]
    assert coder["runtime"]["tool_output_max_tokens"] == "8000"
    assert coder["runtime"]["tool_output_trim"] == "deterministic-prefix"
    assert coder["runtime"]["resume_hook"] == "session_start"
    assert coder_caps["bash"]["scope"] == "session_workspace"
    assert coder_caps["write"]["scope"] == "session_workspace"
    assert coder_caps["bash"]["requires_approval"] is True
    assert coder_caps["write"]["requires_approval"] is True
    for name in ("read", "write", "edit", "test"):
        assert coder_caps[name]["root"] == "."
    assert coder_perms["bash"]["decision"] == "ask"
    assert coder_perms["write"]["decision"] == "ask"
    assert coder_perms["edit"]["decision"] == "allow"
    assert coder_perms["test"]["decision"] == "allow"


def test_explore_approved_edit_test_green() -> None:
    repo = copied_fixture()
    before = read_file(repo, "calculator.py")
    failed = run_test(repo, approved=True)
    assert failed.returncode != 0
    proposal = propose_edit(repo, "calculator.py", before, before.replace("left - right", "left + right"))
    assert proposal["mutates"] is False
    apply_edit(repo, proposal, approved=True)
    passed = run_test(repo, approved=True)
    assert passed.returncode == 0, passed.stdout + passed.stderr


def test_denied_unapproved_mutations_fail_closed() -> None:
    repo = copied_fixture()
    original = (repo / "calculator.py").read_text()
    proposal = propose_edit(repo, "calculator.py", original, original.replace("-", "+"))
    try:
        apply_edit(repo, proposal, approved=False)
    except PermissionError:
        pass
    else:
        raise AssertionError("unapproved write must fail closed")
    assert (repo / "calculator.py").read_text() == original
    try:
        run_test(repo, approved=False)
    except PermissionError:
        pass
    else:
        raise AssertionError("unapproved bash must fail closed")


def test_explorer_cannot_mutate() -> None:
    explorer_caps = capabilities(EXPLORER_ROOT)
    assert set(explorer_caps) == {"read"}
    assert all(entry["read_only"] for entry in explorer_caps.values())
    for forbidden in ("write", "edit", "bash", "test"):
        assert forbidden not in explorer_caps


def test_restart_state_can_resume() -> None:
    repo = copied_fixture()
    checkpoint = repo / ".coder-checkpoint.json"
    before = read_file(repo, "calculator.py")
    proposal = propose_edit(repo, "calculator.py", before, before.replace("left - right", "left + right"))
    save_checkpoint(checkpoint, "approved-edit-ready")
    restored = load_checkpoint(checkpoint)
    assert restored == {"step": "approved-edit-ready"}
    apply_edit(repo, proposal, approved=True)
    passed = run_test(repo, approved=True)
    assert passed.returncode == 0, passed.stdout + passed.stderr


def main() -> None:
    tests = [
        test_profile_declares_controls,
        test_explore_approved_edit_test_green,
        test_denied_unapproved_mutations_fail_closed,
        test_explorer_cannot_mutate,
        test_restart_state_can_resume,
    ]
    for test in tests:
        test()
        print(f"PASS {test.__name__}")


if __name__ == "__main__":
    main()
