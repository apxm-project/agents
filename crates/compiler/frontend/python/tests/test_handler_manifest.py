"""Portable Python handler-manifest coverage."""

from __future__ import annotations

import json

from apxm.constants import ENV_APXM_PYTHON_TOOLS_OUT
from apxm.execution import write_python_tools_manifest
from apxm.handler_manifest import (
    HANDLER_MANIFEST_SOURCE_DIRECTORY,
    HANDLER_MANIFEST_VERSION,
    HandlerKind,
    HandlerLanguage,
    tool_descriptor,
)
from apxm.tools import tool


@tool
def echo(value: str) -> str:
    """Return a caller-provided value."""
    return value


def test_python_handler_manifest_embeds_artifact_local_source(
    monkeypatch,
    tmp_path,
) -> None:
    """Python manifests carry source text without an author checkout path."""
    descriptor = tool_descriptor(
        handler_id=echo.handler_id,
        fn=echo.fn,
        name=echo.name,
        description=echo.description,
        schema=echo.schema,
    )
    output = tmp_path / "handlers.json"
    monkeypatch.setenv(ENV_APXM_PYTHON_TOOLS_OUT, str(output))

    write_python_tools_manifest([descriptor])

    manifest = json.loads(output.read_text(encoding="utf-8"))
    assert manifest["version"] == HANDLER_MANIFEST_VERSION
    handler = manifest["handlers"][0]
    assert handler["kind"] == HandlerKind.TOOL.value
    assert handler["language"] == HandlerLanguage.PYTHON.value
    assert handler["source"]["artifact_path"].startswith(
        f"{HANDLER_MANIFEST_SOURCE_DIRECTORY}/"
    )
    assert "def echo" in handler["source"]["content"]
    assert str(tmp_path) not in handler["source"]["content"]
