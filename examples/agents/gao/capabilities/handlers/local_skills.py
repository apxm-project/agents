from __future__ import annotations

import json
from pathlib import Path

from apxm import tool


AGENT_ROOT = Path(__file__).resolve().parents[2]


@tool(name="list_local_skills")
def list_local_skills() -> str:
    """List local Gao skills."""

    skills = []
    for path in sorted((AGENT_ROOT / "skills").iterdir()):
        if path.is_dir():
            skills.append({"id": path.name, "path": f"skills/{path.name}/SKILL.md"})
    return json.dumps(skills, sort_keys=True)


@tool(name="read_local_skill")
def read_local_skill(skill_id: str) -> str:
    """Read one local Gao skill."""

    root = (AGENT_ROOT / "skills").resolve()
    path = (root / skill_id / "SKILL.md").resolve()
    if root not in path.parents:
        raise ValueError("skill path escapes the agent package")
    return path.read_text(encoding="utf-8")
