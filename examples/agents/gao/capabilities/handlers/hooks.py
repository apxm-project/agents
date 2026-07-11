from __future__ import annotations

from apxm import HookMode, LifecycleEvent, hook


SUMMARY_KEY = "gao:conversation:summary"


@hook(on=LifecycleEvent.PRE_TURN)
def inject_context(ctx):
    return ctx.prepend_system(
        "You are Gao, an APXM architecture expert. Cite only local paper-search results."
    )


@hook(on=LifecycleEvent.PRE_ASK)
def inject_apxm_context(ctx):
    return ctx.prepend_system("Use explicit capabilities and return Studio-compatible drafts.")


@hook(on=LifecycleEvent.PRE_CAP, match="compose_workflow", mode=HookMode.GATE)
def gate_compose_workflow(ctx, call):
    if not str(call.args.get("name", "")).strip():
        return ctx.deny("workflow name is required")
    return ctx.allow()


@hook(on=LifecycleEvent.POST_CAP, match="*")
def redact_tool_results(ctx, call, result):
    text = str(result)
    for marker in ("api_key=", "token=", "secret="):
        text = text.replace(marker, f"{marker}<redacted>")
    return ctx.replace_result(text)


@hook(on=LifecycleEvent.POST_TURN)
def compact_conversation(ctx, reply):
    window = ctx.recall_window(n=40)
    if ctx.count_tokens(window) >= 20_000:
        ctx.umem(SUMMARY_KEY, ctx.ask(window, system="Summarize this APXM conversation."))
    return None
