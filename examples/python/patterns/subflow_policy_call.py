#!/usr/bin/env python3
"""Attach a stricter policy to the FLOW_CALL node that invokes a subflow."""

from apxm import GraphRecorder, NodePolicy, ToolGroup, compile


@compile(default_policy=NodePolicy(capability_groups=[ToolGroup.WEB], token_budget=256))
def research_step(g: GraphRecorder, topic: str):
    notes = g.ask(name="notes", prompt=f"Collect notes about {{topic}}.")
    g.done(notes)


@compile(default_policy=NodePolicy(capability_groups=[ToolGroup.WEB], token_budget=512))
def pipeline(g: GraphRecorder, topic: str):
    research = g.call(
        research_step,
        topic=topic,
        node_policy=NodePolicy(capability_groups=[ToolGroup.FILE_READ], token_budget=128),
    )
    final = g.ask(
        name="final",
        prompt="Turn these notes into a concise answer: {research}",
    )
    g.done(final)


if __name__ == "__main__":
    print(pipeline._air_text)
