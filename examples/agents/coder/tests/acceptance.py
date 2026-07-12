"""Static acceptance checks for the Coder and Explorer package surfaces."""

from __future__ import annotations

import json
import tomllib
from pathlib import Path


CODER_ROOT = Path(__file__).resolve().parents[1]
EXPLORER_ROOT = CODER_ROOT.parent / "explorer"


def load_toml(path: Path) -> dict:
    return tomllib.loads(path.read_text())


def folder_entries(root: Path, file_name: str) -> dict[str, dict]:
    return {
        path.parent.name: load_toml(path)
        for path in sorted((root / "capabilities").glob(f"*/{file_name}"))
    }


def test_coder_declares_only_read_only_capabilities() -> None:
    agent = load_toml(CODER_ROOT / "agent.toml")
    capabilities = folder_entries(CODER_ROOT, "capability.toml")
    permissions = folder_entries(CODER_ROOT, "permission.toml")

    assert set(capabilities) == {"edit", "read", "test"}
    assert set(permissions) == set(capabilities)
    assert agent["capabilities"] == ["edit", "read", "test"]
    assert all(entry["read_only"] is True for entry in capabilities.values())
    assert all(entry == {"capability": name, "decision": "allow"} for name, entry in permissions.items())
    assert set(agent["runtime"]) == {"loop", "memory_space", "session_prefix"}
    assert "hooks" not in agent


def test_coder_tools_are_typed_and_non_mutating() -> None:
    tools = json.loads((CODER_ROOT / "capabilities/handlers/tools.json").read_text())
    tools_by_name = {entry["name"]: entry for entry in tools}

    assert set(tools_by_name) == {"edit", "test"}
    assert all(entry["read_only"] is True for entry in tools_by_name.values())
    assert all(entry["requires_approval"] is False for entry in tools_by_name.values())
    assert tools_by_name["edit"]["schema"]["required"] == ["file_path", "before", "after"]
    assert tools_by_name["test"]["schema"]["required"] == ["command"]
    assert all(entry["schema"]["additionalProperties"] is False for entry in tools_by_name.values())


def test_explorer_is_read_only_and_linked_to_coder() -> None:
    capabilities = folder_entries(EXPLORER_ROOT, "capability.toml")
    permissions = folder_entries(EXPLORER_ROOT, "permission.toml")
    coder_hierarchy = load_toml(CODER_ROOT / "hierarchy.toml")
    explorer_hierarchy = load_toml(EXPLORER_ROOT / "hierarchy.toml")

    assert set(capabilities) == {"read"}
    assert capabilities["read"]["read_only"] is True
    assert permissions == {"read": {"capability": "read", "decision": "allow"}}
    assert coder_hierarchy["permitted_children"] == ["explorer"]
    assert explorer_hierarchy["parent"] == "coder"


def main() -> None:
    tests = [
        test_coder_declares_only_read_only_capabilities,
        test_coder_tools_are_typed_and_non_mutating,
        test_explorer_is_read_only_and_linked_to_coder,
    ]
    for test in tests:
        test()
        print(f"PASS {test.__name__}")


if __name__ == "__main__":
    main()
