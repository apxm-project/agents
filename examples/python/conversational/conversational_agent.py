#!/usr/bin/env python3
"""conversational_agent.py

    A conversational agent is an APXM program
    (a loop + middleware + prompts that do
     context injection, skill discovery and tool execution).

That sentence is the whole idea. The CLI (`apxm chat`) is a *thin host*; the
agent's behaviour is this editable APXM program. Swap the program and you have a
different agent — you do not touch the CLI.

How the four pieces map onto the stack
--------------------------------------
  LOOP        the host turn-loop (`apxm chat`) drives one execution of this
              graph per user message: one DAG = one turn. It threads the running
              transcript in as `conversation` and a stable `session_id`, so
              server-side memory accrues across turns.

  MIDDLEWARE  cross-cutting layers that wrap every turn, registered with the
              runtime (OperationMiddleware), not hand-written into each graph.
              Context MANAGEMENT lives here — it is the lifecycle of the context,
              not the agent's logic:
                * context-injection — assembles the system prompt from AGENTS.md
                  + beliefs and fills it into the ASK;
                * conversation-context — recalls the recent transcript window
                  pre-ASK and appends the turn post-ASK (history is session
                  MEMORY, not a host-threaded string);
                * compaction — summarises older turns when the budget is hit;
                * token-budget / timeout / loop-guard — already built in;
                * window fit / pruning — driven over the runtime ContextStack.
              They compose: add or remove a concern without editing the body.
              Rule of thumb: context MANAGEMENT is middleware; deliberate task
              MEMORY (below) is the program.

  PROMPTS     the persona/system prompt and the per-turn prompt below.

The body then performs the three verbs explicitly:

  CONTEXT INJECTION   recall session memory (the middleware additionally injects
                      AGENTS.md / beliefs into the system prompt).
  SKILL DISCOVERY     find the most relevant skill *by its description*
                      (embedder-backed search over installed skills) rather than
                      hard-coding a skill id.
  TOOL EXECUTION      answer with tools enabled; the runtime runs the internal
                      model -> tool -> model loop and streams tokens over SSE.

Run it (requires a running apxm-server; see this dir's README for a mock backend):

    dekk apxm execute examples/python/conversational/conversational_agent.py --emit-air > agent.air
    apxm chat --air agent.air --server http://127.0.0.1:18800

Validate without a server or binary:

    PYTHONPATH=crates/compiler/apxm-frontend/python \
        python3 examples/python/conversational/conversational_agent.py --validate
"""

from apxm import GraphRecorder, compile, tool

# PROMPTS — single source of truth for the assistant persona. The context-
# injection middleware prepends AGENTS.md / memory / beliefs to this at runtime.
PERSONA = (
    "You are APXM Assistant, a precise, helpful conversational agent. "
    "Consult recalled context before answering, prefer a relevant skill when one "
    "fits, use tools only when they add information, and keep replies concise "
    "and well-structured. If you are unsure, say so."
)


# TOOL EXECUTION + SKILL DISCOVERY are both just capabilities the program calls.
@tool
def find_skill(request: str) -> str:
    """Find the installed skill most relevant to a request.

    Ranks installed skills by their *description* (embedder-backed similarity
    over the skill catalogue) and returns the best match's id + summary, or an
    empty result if none fit. This is description-based discovery: the agent
    never hard-codes a skill id.
    """
    # Stub: the real capability queries /v1/skills and ranks by description.
    return f"[best skill for: {request}]"


@compile()
def conversational_agent(g: GraphRecorder, conversation: str):
    """One conversational turn — the agent body the host loop runs per message.

    Parameters
    ----------
    conversation : str
        The full running transcript, ending in ``User: <msg>\\nAssistant:``,
        supplied by ``apxm chat`` each turn.
    """
    # TASK MEMORY (deliberate) — the program chooses to recall a relevant fact.
    # NOTE: the conversation *history window*, compaction, and token budget are
    # NOT here — they are context MANAGEMENT handled by middleware. This qmem is
    # the agent's own decision to look something up, which is program logic.
    # (System-prompt context injection from AGENTS.md/beliefs is also middleware.)
    history = g.query_memory(name="recall", query="relevant prior facts", space="stm")

    # SKILL DISCOVERY — pick the relevant skill by description (not by id).
    skill = g.invoke_tool(find_skill, request="the latest user message")

    # TOOL EXECUTION — answer with tools; the runtime drives the model->tool loop.
    # `{history}` and `{skill}` auto-wire data edges from the nodes above.
    answer = g.ask(
        name="answer",
        prompt=(
            "{conversation}\n\n"
            "Recalled context:\n{history}\n\n"
            "Most relevant skill (by description):\n{skill}"
        ),
        system_prompt=PERSONA,      # middleware may enrich this at runtime
        tool_groups=["web"],        # self-enabling, least-privilege tool group
    )

    # TASK MEMORY — the agent deliberately records a fact for later turns, fenced
    # so the write is ordered. (The conversation transcript itself is appended by
    # the conversation-context middleware, not here — that is context management.)
    note = g.update_memory(
        name="remember",
        data="a fact worth keeping from this turn",
        key="relevant prior facts",
        space="stm",
    )
    g.add_edge(answer, note)
    commit = g.fence(name="commit")
    g.add_edge(note, commit)

    # The answer is the user-visible reply.
    g.done(source=answer)
    _ = (history, skill)  # referenced via templates; keep linters quiet


if __name__ == "__main__":
    import sys

    if "--validate" in sys.argv:
        from apxm.ir import validate_against_apxm

        result = validate_against_apxm(conversational_agent._graph)
        print("VALID" if result.valid else "INVALID")
        for err in result.errors:
            print(f"  ERROR: {err}")
        for warn in result.warnings:
            print(f"  WARN:  {warn}")
        sys.exit(0 if result.valid else 1)

    print(conversational_agent._graph.to_air())
