"""Hook host-calls: a hook receives apxm primitives (LLM, tools, memory) and
decides what to do. ctx.ask / ctx.call / ctx.count_tokens / ctx.recall each emit
a host_call that the runtime services with the hook's own ExecutionContext.

Driven in-process by faking the runtime side (capture the host_call, feed back a
host_result keyed by method).
"""

import asyncio

from apxm import tool_worker as tw


def _fake_runtime(monkeypatch, by_method):
    """Replace _emit_line so each host_call is answered immediately (single
    thread). `by_method` maps method -> (ok, value, error)."""
    captured: list[dict] = []

    def fake_emit(obj):
        if obj.get(tw.WIRE_FIELD_TYPE) == tw.WIRE_TYPE_HOST_CALL:
            captured.append(obj)
            ok, value, error = by_method[obj[tw.WIRE_FIELD_METHOD]]
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


def test_ctx_ask_roundtrips_llm(monkeypatch):
    cap = _fake_runtime(monkeypatch, {tw.HOST_METHOD_LLM_ASK: (True, "the answer", None)})
    ctx = tw._HookCtx({}, req_id="r1")

    out = ctx.ask("hello", system="sys")

    assert out == "the answer"
    call = cap[0]
    assert call[tw.WIRE_FIELD_METHOD] == tw.HOST_METHOD_LLM_ASK
    assert call[tw.WIRE_FIELD_PARENT_REQUEST_ID] == "r1"
    assert call[tw.WIRE_FIELD_PARAMS] == {"prompt": "hello", "system": "sys"}


def test_ctx_call_invokes_named_tool(monkeypatch):
    cap = _fake_runtime(monkeypatch, {tw.HOST_METHOD_TOOL_CALL: (True, 42, None)})
    ctx = tw._HookCtx({}, req_id="r1")

    out = ctx.call("count_tokens", text="abc")

    assert out == 42
    assert cap[0][tw.WIRE_FIELD_METHOD] == tw.HOST_METHOD_TOOL_CALL
    assert cap[0][tw.WIRE_FIELD_PARAMS] == {"name": "count_tokens", "args": {"text": "abc"}}


def test_ctx_count_tokens_convenience(monkeypatch):
    _fake_runtime(monkeypatch, {tw.HOST_METHOD_TOOL_CALL: (True, 7, None)})
    ctx = tw._HookCtx({}, req_id="r1")
    assert ctx.count_tokens("some text") == 7


def test_ctx_count_tokens_decodes_apxm_value_wrappers(monkeypatch):
    for wrapped in ({"Integer": 7}, {"Number": {"Integer": 7}}, {"value": "7"}):
        _fake_runtime(monkeypatch, {tw.HOST_METHOD_TOOL_CALL: (True, wrapped, None)})
        ctx = tw._HookCtx({}, req_id="r1")
        assert ctx.count_tokens("some text") == 7


def test_ctx_recall_reads_user_key(monkeypatch):
    cap = _fake_runtime(monkeypatch, {tw.HOST_METHOD_MEM_READ: (True, "prior summary", None)})
    ctx = tw._HookCtx({}, req_id="r1")

    out = ctx.recall("my:summary")

    assert out == "prior summary"
    assert cap[0][tw.WIRE_FIELD_METHOD] == tw.HOST_METHOD_MEM_READ
    assert cap[0][tw.WIRE_FIELD_PARAMS] == {"key": "my:summary"}


def test_host_call_raises_on_error(monkeypatch):
    _fake_runtime(monkeypatch, {tw.HOST_METHOD_LLM_ASK: (False, None, "backend down")})
    ctx = tw._HookCtx({}, req_id="r1")
    try:
        ctx.ask("hello")
    except RuntimeError as exc:
        assert "backend down" in str(exc)
    else:
        raise AssertionError("expected RuntimeError on host error")


def test_host_call_unavailable_without_parent():
    # No req_id bound (e.g. an offline unit context): host calls are unavailable.
    ctx = tw._HookCtx({})
    try:
        ctx.ask("hello")
    except RuntimeError as exc:
        assert "no parent request" in str(exc)
    else:
        raise AssertionError("expected RuntimeError without a parent request")


def test_post_ask_hook_receives_reply():
    seen = {}

    def hook(ctx, reply):
        seen["reply"] = reply

    out = tw._invoke_hook(hook, "post_ask", {"reply": "answer"}, req_id="r1")

    assert out == {}
    assert seen == {"reply": "answer"}


def test_async_post_ask_hook_is_awaited():
    seen = {}

    async def hook(ctx, reply):
        await asyncio.sleep(0)
        seen["reply"] = reply
        return ctx.deny("async hook decision")

    out = tw._invoke_hook(hook, "post_ask", {"reply": "answer"}, req_id="r1")

    assert out == {"decision": "deny", "reason": "async hook decision"}
    assert seen == {"reply": "answer"}
