"""Handoff demo — two agents with explicit execution handoff.

Agent A (researcher) gathers information, then hands off to Agent B
(writer) who uses the context to produce a final response.

Usage:
    python3 examples/python/native-tools/handoff_demo.py
"""

from apxm import Agent, compile, run


researcher = Agent(
    name="researcher",
    instructions="You are a research assistant. Gather key facts.",
)

writer = Agent(
    name="writer",
    instructions="You are a technical writer. Produce a clear summary.",
)


@compile()
def research_and_write(g, topic: str):
    """Researcher gathers info, then hands off to writer for final output."""
    r = researcher.bind(g)
    w = writer.bind(g)

    # Researcher does initial work
    research = r.ask(f"Research the following topic: {topic}")

    # Handoff from researcher to writer with state transfer
    result = r.handoff(writer, f"Summarize this research: {{{research.name}}}")

    g.done(result)


if __name__ == "__main__":
    output = run(research_and_write, "The history of agent architectures", mock=True)
    print(output.content)
