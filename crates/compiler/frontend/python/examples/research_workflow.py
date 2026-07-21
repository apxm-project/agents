#!/usr/bin/env python3
"""Example: Multi-agent research workflow using APXM Python frontend.

This example demonstrates:
- @compile decorator with parameter derivation
- Named parameter placeholders ({topic} syntax)
- AgentHandle sugar for spawn().ask() response nodes
- Team sugar for workflow-local teams
- Graph serialization to canonical .air
"""

from apxm import DependencyType, compile, GraphRecorder


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
    researcher = team.add("researcher", mode="normal")
    critic = team.add("critic", mode="extended")
    synthesizer = team.add("synthesizer", mode="normal")

    # Parallel research and critique
    researcher.ask("Research the topic: {topic}. Provide detailed findings.")
    critic.ask("Critically analyze existing research on: {topic}. Identify gaps and limitations.")

    # Wait for both to complete
    sync = team.wait_all("research_sync")

    # Synthesize results
    synthesis = synthesizer.ask(
        "Synthesize the research findings and critique into a comprehensive report on {topic}."
    )

    # Control flow: sync completes before synthesis
    g.add_edge(sync, synthesis, dependency=DependencyType.CONTROL)

    # Return final synthesis
    g.done(source=synthesis)


def main():
    """Generate and print the workflow graph."""
    # Access the compiled graph
    graph = research_workflow._graph

    print("Generated Workflow AIR:")
    print("=" * 60)
    print(f"Name: {graph.name}")
    print(f"Parameters: {[(p.name, p.type_name) for p in graph.parameters]}")
    print(f"Nodes: {len(graph.nodes)}")
    print(f"Edges: {len(graph.edges)}")
    print()

    # Print AIR emitted by the Python frontend.
    print("AIR:")
    print("=" * 60)
    print(research_workflow.to_air())


if __name__ == "__main__":
    main()
