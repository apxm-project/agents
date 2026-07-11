from __future__ import annotations

import json

from apxm import tool


@tool(name="prepare_validation")
def prepare_validation(workflow_name: str, artifact: str) -> str:
    """Prepare a validation request for a workflow artifact."""

    return json.dumps(
        {
            "workflow_name": workflow_name,
            "artifact_preview": artifact[:2000],
            "next_action": "ask_user_before_write_or_execute",
        },
        sort_keys=True,
    )


@tool(name="explain_permission")
def explain_permission(capability_id: str, permission: str) -> str:
    """Explain the approval required by a capability."""

    requires_approval = permission in {"write", "execute", "deploy"}
    return json.dumps(
        {
            "capability": capability_id,
            "permission": permission,
            "requires_approval": requires_approval,
        },
        sort_keys=True,
    )
