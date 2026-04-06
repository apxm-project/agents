#!/usr/bin/env python3
"""Example: Multi-agent research workflow using APXM Python frontend.

This example demonstrates:
- @compile decorator with parameter derivation
- Named parameter placeholders ({topic} syntax)
- AgentHandle sugar for spawn().ask() chaining
- Team sugar for workflow-local teams
- Graph serialization to JSON
"""

from apxm import GraphRecorder, compile


@compile()
def research_workflow(g: GraphRecorder, topic: str):
    """Collaborative research workflow with multiple specialized agents.

    Parameters
    ----------
    topic : str
        The research topic to investigate.
    """
    # Create a research team
    team = g.team("research_team")

    # Spawn specialized agents
    researcher = team.add("researcher", profile="claude", mode="normal")
    critic = team.add("critic", profile="claude", mode="extended")
    synthesizer = team.add("synthesizer", profile="claude", mode="normal")

    # Parallel research and critique
    researcher.ask(f"Research the topic: {{topic}}. Provide detailed findings.")
    critic.ask(f"Critically analyze existing research on: {{topic}}. Identify gaps and limitations.")

    # Wait for both to complete
    sync = team.wait_all("research_sync")

    # Synthesize results
    synthesis = g.communicate(
        "synthesis_request",
        target_agent="synthesizer",
        message=f"Synthesize the research findings and critique into a comprehensive report on {{topic}}."
    )

    # Control flow: sync completes before synthesis
    sync >> synthesis

    # Return final synthesis
    return_node = g.return_("final_output", source=synthesis)


def main():
    """Generate and print the workflow graph."""
    # Access the compiled graph
    graph = research_workflow._graph

    print("Generated Workflow Graph:")
    print("=" * 60)
    print(f"Name: {graph.name}")
    print(f"Parameters: {[(p.name, p.type_name) for p in graph.parameters]}")
    print(f"Nodes: {len(graph.nodes)}")
    print(f"Edges: {len(graph.edges)}")
    print()

    # Print graph as JSON
    print("JSON Representation:")
    print("=" * 60)
    print(graph.to_json(indent=2))

    # Print as .air format
    print()
    print("AIR (Agent Intermediate Representation):")
    print("=" * 60)
    print(graph.to_air())


if __name__ == "__main__":
    main()
