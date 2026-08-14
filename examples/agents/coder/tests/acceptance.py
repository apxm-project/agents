"""Static acceptance checks for the focused Coder package surface."""

from __future__ import annotations

import json
import tomllib
from pathlib import Path


CODER_ROOT = Path(__file__).resolve().parents[1]


def load_toml(path: Path) -> dict:
    return tomllib.loads(path.read_text())


def test_coder_is_two_manifests_plus_the_handlers_it_ships() -> None:
    agent = load_toml(CODER_ROOT / "agent.toml")

    assert sorted(path.name for path in CODER_ROOT.glob("*.toml")) == [
        "agent.toml",
        "integrity.toml",
    ]
    # The capability surface is the handlers the package ships, not an
    # inventory file restating them.
    shipped = sorted(
        path.parent.name for path in CODER_ROOT.glob("capabilities/*/handler.ts")
    )
    assert shipped == ["edit", "test"]
    assert not list(CODER_ROOT.glob("capabilities/**/*.toml"))
    # Every decision Coder makes is the default one, so it states none.
    assert "permissions" not in agent
    assert "capabilities" not in agent
    assert "allowed_agent_skills" not in agent
    assert "hooks" not in agent


def test_coder_tools_are_typed_and_non_mutating() -> None:
    manifest = json.loads((CODER_ROOT / "capabilities/handlers/tools.json").read_text())
    tools_by_name = {entry["name"]: entry for entry in manifest["handlers"]}

    assert set(tools_by_name) == {"edit", "test"}
    # `requires_approval` is the resolved permission decision, carried from the
    # one place a package states permissions.
    assert all(entry["requires_approval"] is False for entry in tools_by_name.values())
    assert tools_by_name["edit"]["schema"]["required"] == ["file_path", "before", "after"]
    assert tools_by_name["test"]["schema"]["required"] == ["command"]
    assert all(entry["schema"]["additionalProperties"] is False for entry in tools_by_name.values())


def main() -> None:
    tests = [
        test_coder_is_two_manifests_plus_the_handlers_it_ships,
        test_coder_tools_are_typed_and_non_mutating,
    ]
    for test in tests:
        test()
        print(f"PASS {test.__name__}")


if __name__ == "__main__":
    main()
