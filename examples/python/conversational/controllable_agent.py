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

from apxm import ConversationalAgent, CompactionPolicy, hook, tool, Agent


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
@hook(on="session_start")  # gate-capable; runs once at session start
def announce(ctx):
    ctx.log(f"session start; budget={ctx.remaining_budget}")


@hook(on="pre_tool", match="lookup", mode="gate")  # Allow / Deny / EditArgs
def guard_lookup(ctx, call):
    if call.args["symbol"] == "FORBIDDEN":
        return ctx.deny("symbol not permitted")
    return ctx.edit_args({**call.args, "symbol": call.args["symbol"].upper()})


@hook(on="post_tool", match="*")
def redact(ctx, call, result):
    return ctx.replace_result(scrub_secrets(result))


@hook(on="pre_ask")  # context injection (dataflow system prompt)
def inject_context(ctx):
    ctx.prepend_system(ctx.read_agents_md() + "\n" + ctx.recall_window(n=4))


@hook(on="post_turn")  # rolling compaction — entirely program-controlled
def compact(ctx, reply):
    # Fold the running summary with the recent window into a new summary so
    # early facts survive once they slide out of `keep_recent`. `ctx.summarize`
    # calls the runtime LLM (a host call back into the runtime), and the
    # `conversation:summary` key is surfaced by recall ahead of recent turns.
    combined = (ctx.prior_summary() + "\n" + ctx.recall_window(n=8)).strip()
    if len(combined) > 600:  # proxy for CompactionPolicy.compact_at_tokens
        ctx.umem("conversation:summary", ctx.summarize(combined))


# ---- the agent: declares the LOOP + the turn body, all in-program ------------
agent = ConversationalAgent(
    persona="You are APXM Assistant. Be precise and concise.",
    memory_space="stm",
    tools=[lookup],
    tool_groups=["web"],
    skills=True,  # real search_skills discovery
    sub_agents=[researcher],  # resolved in the SAME artifact
    compaction=CompactionPolicy(keep_recent=4, compact_at_tokens=20_000),
    hooks=[announce, guard_lookup, redact, inject_context, compact],
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
