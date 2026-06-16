#!/usr/bin/env python3
"""controllable_agent.py — the whole agent in one program.

The feature acceptance fixture for specs/0001-agent-in-program. The *entire*
conversational agent — the loop, each turn, context management + compaction,
pre/post/session-start hooks, skill/tool discovery, and sub-agents — lives in
this one program. The host (`apxm chat` or the HTTP server) is a dumb pipe:
deliver the user message in, render streamed tokens out.

Validate without a server (passes once the Foundational phase lands):
    PYTHONPATH=crates/compiler/apxm-frontend/python \\
        python3 examples/python/conversational/controllable_agent.py --validate

Compile to one self-contained multi-flow artifact:
    dekk apxm execute examples/python/conversational/controllable_agent.py \\
        --emit-air > agent.air
"""

from apxm import (
    Agent,
    CompactionPolicy,
    ConversationalAgent,
    HookMode,
    LifecycleEvent,
    ToolGroup,
    hook,
    tool,
)


# ---- tools & sub-agents (server-path callable via PythonToolBridge) ----------
@tool
def lookup(symbol: str) -> str:
    """Look up a ticker price."""
    return f"[price of {symbol}]"


researcher = Agent(name="researcher", instructions="Gather supporting facts.")


def scrub_secrets(text: str) -> str:
    """Redact obvious secret-looking tokens from a tool result."""
    import re

    return re.sub(r"(?i)(api[_-]?key|token|secret)\s*[:=]\s*\S+", r"\1=<redacted>", text)


# ---- hooks: pre / post / session-start, ALL python callables -----------------
@hook(on=LifecycleEvent.SESSION_START)  # gate-capable; runs once at session start
def announce(ctx):
    ctx.log(f"session start; budget={ctx.remaining_budget}")


@hook(on=LifecycleEvent.PRE_TURN)
def before_turn(ctx):
    ctx.log("pre_turn")


@hook(on=LifecycleEvent.PRE_TOOL, match="lookup", mode=HookMode.GATE)
def guard_lookup(ctx, call):
    ctx.log("pre_tool:lookup")
    if call.args["symbol"] == "FORBIDDEN":
        return ctx.deny("symbol not permitted")
    return ctx.edit_args({**call.args, "symbol": call.args["symbol"].upper()})


@hook(on=LifecycleEvent.POST_TOOL, match="*")
def redact(ctx, call, result):
    ctx.log("post_tool")
    return ctx.replace_result(scrub_secrets(result))


@hook(on=LifecycleEvent.PRE_ASK)  # context injection (dataflow system prompt)
def inject_context(ctx):
    ctx.log("pre_ask")
    ctx.prepend_system(ctx.read_agents_md() + "\n" + ctx.recall_window(n=4))


@hook(on=LifecycleEvent.POST_ASK)
def after_ask(ctx, reply):
    ctx.log("post_ask")


# Author-owned compaction settings the hook below reads. compact_at_tokens is
# deliberately small here so compaction triggers within a short demo session;
# raise it for real use. APXM enforces none of this — the hook is the policy.
COMPACTION = CompactionPolicy(keep_recent=4, compact_at_tokens=300)


@hook(on=LifecycleEvent.POST_TURN)  # rolling compaction — ALL policy is the user's, here
def compact(ctx, reply):
    ctx.log("post_turn")
    # apxm hands the hook its primitives; the user decides everything:
    #   - the context to compact         (ctx.recall + ctx.recall_window)
    #   - WHEN to compact   (ctx.count_tokens vs the user's own threshold)
    #   - HOW to compact                 (ctx.ask, with the user's own prompt)
    #   - WHERE to keep it               (ctx.umem to the user's own key)
    prior = ctx.recall(COMPACTION.summary_key) or ""
    # Pull a wide window so a fact is folded into the summary before it slides
    # out of the turn's keep_recent recall (the user chooses how far back).
    context = (str(prior) + "\n" + ctx.recall_window(n=50)).strip()
    if ctx.count_tokens(context) > COMPACTION.compact_at_tokens:
        summary = ctx.ask(
            "Update the running summary below so a later turn loses no important "
            "fact, name, number, or decision. Return only the updated summary.\n\n"
            f"{context}",
            system="You maintain a compact, faithful running summary.",
        )
        ctx.umem(COMPACTION.summary_key, summary)


# ---- the agent: declares the LOOP + the turn body, all in-program ------------
agent = ConversationalAgent(
    persona="You are APXM Assistant. Be precise and concise.",
    memory_space="stm",
    tools=[lookup],
    tool_groups=[ToolGroup.WEB],
    skills=True,  # real search_skills discovery
    sub_agents=[researcher],  # resolved in the SAME artifact
    compaction=COMPACTION,  # same object the compact() hook reads — keys agree
    hooks=[announce, before_turn, guard_lookup, redact, inject_context, after_ask, compact],
    loop="in_graph",  # the conversation loop lives in the .air
)

main = agent.compile()  # → ONE self-contained multi-flow artifact


if __name__ == "__main__":
    import sys

    if "--validate" in sys.argv:
        result = main.validate()
        print("VALID" if result.valid else "INVALID")
        for err in result.errors:
            print(f"  ERROR: {err}")
        for warn in result.warnings:
            print(f"  WARN:  {warn}")
        sys.exit(0 if result.valid else 1)

    print(main.to_air())
