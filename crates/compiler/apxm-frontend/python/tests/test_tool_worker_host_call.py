"""Hook host-calls: ctx.ask / ctx.summarize call the runtime LLM over the bridge.

A lifecycle hook runs in the worker subprocess, but it can still drive an
`llm.ask` by emitting a `host_call` that the runtime services with the hook's
own ExecutionContext. These tests drive the round-trip in-process by faking the
runtime side (capture the host_call, feed back a host_result).
"""

from apxm import tool_worker as tw


def _fake_runtime(monkeypatch, *, ok=True, value="SUMMARY", error=None):
    """Replace _emit_line so a host_call is answered immediately, single-thread."""
    captured: dict = {}

    def fake_emit(obj):
        if obj.get(tw.WIRE_FIELD_TYPE) == tw.WIRE_TYPE_HOST_CALL:
            captured["call"] = obj
            tw._deliver_host_result(
                {
                    tw.WIRE_FIELD_REQUEST_ID: obj[tw.WIRE_FIELD_REQUEST_ID],
                    tw.WIRE_FIELD_OK: ok,
                    tw.WIRE_FIELD_VALUE: value,
                    tw.WIRE_FIELD_ERROR: None if error is None else {tw.WIRE_FIELD_MESSAGE: error},
                }
            )

    monkeypatch.setattr(tw, "_emit_line", fake_emit)
    return captured


def test_ctx_ask_roundtrips_host_call(monkeypatch):
    captured = _fake_runtime(monkeypatch, value="the answer")
    ctx = tw._HookCtx({}, req_id="r1")

    out = ctx.ask("hello", system="sys")

    assert out == "the answer"
    call = captured["call"]
    assert call[tw.WIRE_FIELD_METHOD] == tw.HOST_METHOD_LLM_ASK
    assert call[tw.WIRE_FIELD_PARENT_REQUEST_ID] == "r1"
    assert call[tw.WIRE_FIELD_PARAMS]["prompt"] == "hello"
    assert call[tw.WIRE_FIELD_PARAMS]["system"] == "sys"


def test_ctx_ask_raises_on_host_error(monkeypatch):
    _fake_runtime(monkeypatch, ok=False, error="backend down")
    ctx = tw._HookCtx({}, req_id="r1")
    try:
        ctx.ask("hello")
    except RuntimeError as exc:
        assert "backend down" in str(exc)
    else:
        raise AssertionError("expected RuntimeError on host error")


def test_summarize_uses_llm_when_available(monkeypatch):
    _fake_runtime(monkeypatch, value="a tight summary")
    ctx = tw._HookCtx({}, req_id="r1")
    assert ctx.summarize("a very long conversation ...") == "a tight summary"


def test_summarize_degrades_without_host():
    # No req_id bound: ctx.ask is unavailable, so summarize falls back to a
    # heuristic truncation rather than failing the turn.
    ctx = tw._HookCtx({})
    long_text = "x" * 500
    assert ctx.summarize(long_text) == long_text[:280]
