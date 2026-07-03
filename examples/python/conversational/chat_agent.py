#!/usr/bin/env python3
"""chat_agent.py - A professional conversational multi-agent agent.

This is the agent *body* run once per user message by the host turn-loop
(`apxm chat`). The REPL threads the running transcript in as the single
`conversation` parameter and a stable `session_id`, so server-side memory
accrues across turns (see ExecutionContext::memory_scope). The runtime handles
the full per-turn lifetime automatically: dispatcher middleware (timeout, loop
guard, token budget), the internal model->tool->model loop, response parsing,
token accounting, and token-by-token streaming over SSE.

Run it (requires a running apxm-server; see this dir's README for a mock-backed
local server):

    dekk agents execute examples/python/conversational/chat_agent.py --emit-air > chat.air
    apxm chat --air chat.air --server http://127.0.0.1:18800

Pipeline per turn:
  recall (qmem) -> plan (reason) -> tool-using answer (ask) ->
  delegate research (spawn_agent + delegate) -> synthesize (ask) ->
  remember (umem + fence).
"""

from apxm import DependencyType, GraphRecorder, ToolGroup, compile

# Single source of truth for the assistant persona.
PERSONA = (
    "You are APXM Assistant, a precise, helpful conversational agent. "
    "Consult prior context before answering, use tools only when they add "
    "information, delegate specialized subtasks to sub-agents, and keep replies "
    "concise and well-structured. If you are unsure, say so."
)


@compile()
def chat_agent(g: GraphRecorder, conversation: str):
    """One conversational turn.

    Parameters
    ----------
    conversation : str
        The full running transcript, ending in ``User: <msg>\\nAssistant:``,
        supplied by ``apxm chat`` each turn.
    """
    # 1. Recall session-scoped memory from prior turns.
    history = g.query_memory(name="recall", query="conversation summary", space="stm")

    # 2. Plan intent from persona + transcript + recalled context.
    plan = g.reason(
        name="plan",
        prompt=(
            f"{PERSONA}\n\nDecide how to respond to the latest user message.\n"
            "Conversation:\n{conversation}\n\nContext:\n{history}"
        ),
    )

    # 3. Answer with tools (the `web` group is self-enabling, least privilege).
    answer = g.ask(
        name="answer",
        prompt=(
            f"{PERSONA}\n\nProduce a grounded answer to the latest user message, "
            "using tools if they help.\n{conversation}\n\nPlan:\n{plan}"
        ),
        capability_groups=[ToolGroup.WEB],
    )

    # 4. Multi-agent: spawn an inline researcher sub-agent and delegate a
    #    focused subtask; its result (`research`) is woven into the final reply.
    spawn = g.spawn_agent(
        agent_name="researcher",
        system_prompt=(
            "You are a focused research sub-agent. Extract the most relevant "
            "supporting facts from the delegated task and the supplied draft. "
            "Return concise findings only."
        ),
    )
    research = g.delegate(
        name="research",
        target_agent="researcher",
        task_spec="Gather supporting facts for the latest user message.",
    )
    g.add_edge(spawn, research, dependency=DependencyType.CONTROL)
    g.add_edge(answer, research)

    # 6. Synthesize the user-facing reply.
    reply = g.ask(
        name="reply",
        prompt=(
            f"{PERSONA}\n\nWrite the final reply to the user, incorporating the "
            "research.\nAnswer: {answer}\nResearch: {research}"
        ),
    )

    # 7. Persist a memory for the next turn, fenced so the write is ordered.
    note = g.update_memory(
        name="remember",
        data="conversation summary updated this turn",
        key="conversation summary",
        space="stm",
    )
    commit = g.fence(name="commit")
    g.add_edge(note, commit)

    g.done(source=reply)
    # Reference unused locals so linters see the full wiring.
    _ = (history, plan, research)


if __name__ == "__main__":
    print(chat_agent._graph.to_air())
