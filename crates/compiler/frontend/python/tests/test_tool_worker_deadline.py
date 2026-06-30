"""Deadline = resource limit: a timed-out handler must be cancelled.

Regression guard for the `asyncio.shield` bug, where a timed-out tool handler
reported a timeout but kept running to completion in the background (the
deadline observed but did not enforce). Drives `_handle_call` directly with an
injected slow handler and asserts (a) a structured timeout result is emitted
and (b) the handler is actually cancelled, so its compute is reclaimed.

Written as plain `asyncio.run` tests so they need no pytest async plugin.
"""

import asyncio

from apxm import tool_worker as tw


def _drain_lines(monkeypatch):
    """Capture everything the worker writes, for both async + sync writers."""
    lines: list[dict] = []

    async def _async(obj):
        lines.append(obj)

    def _sync(obj):
        lines.append(obj)

    monkeypatch.setattr(tw, "_write_line_async", _async)
    monkeypatch.setattr(tw, "_write_line", _sync)
    return lines


def test_timed_out_handler_is_cancelled(monkeypatch):
    state = {"completed": False}

    async def slow():
        await asyncio.sleep(0.5)  # far longer than the deadline
        state["completed"] = True
        return "should never be observed"

    tw._set_registry({"slow": slow})
    lines = _drain_lines(monkeypatch)

    async def scenario():
        await tw._handle_call(
            {
                tw.WIRE_FIELD_TYPE: tw.WIRE_TYPE_CALL,
                tw.WIRE_FIELD_REQUEST_ID: "r1",
                tw.WIRE_FIELD_TOOL_ID: "slow",
                tw.WIRE_FIELD_ARGS: {},
                tw.WIRE_FIELD_DEADLINE_MS: 30,
            }
        )
        # Wait past the handler's own sleep: a cancelled handler never completes.
        await asyncio.sleep(0.6)

    asyncio.run(scenario())

    # A structured timeout result was emitted.
    assert len(lines) == 1
    result = lines[0]
    assert result[tw.WIRE_FIELD_OK] is False
    assert result[tw.WIRE_FIELD_ERROR][tw.WIRE_FIELD_ERROR_KIND] == tw.WIRE_ERROR_TIMEOUT

    # The handler was actually cancelled. Under the `shield` bug this flips True.
    assert state["completed"] is False


def test_handler_within_deadline_completes(monkeypatch):
    async def quick():
        await asyncio.sleep(0.01)
        return "ok"

    tw._set_registry({"quick": quick})
    lines = _drain_lines(monkeypatch)

    asyncio.run(
        tw._handle_call(
            {
                tw.WIRE_FIELD_TYPE: tw.WIRE_TYPE_CALL,
                tw.WIRE_FIELD_REQUEST_ID: "r2",
                tw.WIRE_FIELD_TOOL_ID: "quick",
                tw.WIRE_FIELD_ARGS: {},
                tw.WIRE_FIELD_DEADLINE_MS: 5000,
            }
        )
    )

    assert len(lines) == 1
    assert lines[0][tw.WIRE_FIELD_OK] is True
    assert lines[0][tw.WIRE_FIELD_VALUE] == "ok"
