from __future__ import annotations

import json

from apxm import tool


@tool(name="plan_workflow")
def plan_workflow(request: str) -> str:
    """Create a Studio workflow draft from a user request."""

    return json.dumps(
        {
            "request": request,
            "studio_canvas": {
                "format": "apxm_studio_workflow",
                "name": "gao_architecture_response",
                "description": request,
                "nodes": [
                    {
                        "id": "question",
                        "kind": "text",
                        "label": "Question",
                        "position": {"x": 40, "y": 100},
                        "config": {"text": request},
                    },
                    {
                        "id": "answer",
                        "kind": "llm",
                        "label": "Architecture answer",
                        "position": {"x": 380, "y": 100},
                        "config": {"prompt": "Answer the connected question: {question}"},
                    },
                    {
                        "id": "result",
                        "kind": "output",
                        "label": "Result",
                        "position": {"x": 760, "y": 100},
                        "config": {},
                    },
                ],
                "edges": [
                    {"source": "question", "target": "answer", "kind": "data"},
                    {"source": "answer", "target": "result", "kind": "data"},
                ],
            },
        },
        sort_keys=True,
    )
