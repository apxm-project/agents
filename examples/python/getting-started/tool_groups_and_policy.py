#!/usr/bin/env python3
"""Typed graph/node policy defaults for tool groups and budgets."""

from apxm import GraphRecorder, NodePolicy, ToolGroup, compile


@compile(default_policy=NodePolicy(tool_groups=[ToolGroup.WEB], token_budget=256))
def research_brief(g: GraphRecorder, topic: str):
    plan = g.ask(
        name="plan",
        prompt=f"Create a short research plan for {{topic}}.",
    )
    brief = g.ask(
        name="brief",
        prompt="Write the brief using the plan: {plan}",
        policy=NodePolicy(tool_groups=[ToolGroup.FILE_READ], token_budget=128),
    )
    g.done(brief)


if __name__ == "__main__":
    print(research_brief._air_text)
