#!/usr/bin/env python3
"""Simple APXM workflow example demonstrating basic API usage."""

from apxm import GraphRecorder


def main():
    """Build a simple ask->think->reason workflow."""
    g = GraphRecorder("simple_pipeline")

    # Add a parameter
    g.param("question", "str")

    # Build a pipeline of reasoning steps
    ask = g.ask("initial_response", "Answer this question: {0}")
    think = g.think("deeper_analysis", "Think deeply about: {0}")
    reason = g.reason("final_answer", "Reason through the logic: {0}")

    # Connect with control flow
    ask >> think >> reason

    # Get the graph
    graph = g.to_graph()

    print("Simple Workflow:")
    print("=" * 60)
    print(f"Name: {graph.name}")
    print(f"Parameters: {graph.parameters}")
    print(f"Nodes: {len(graph.nodes)}")
    print(f"Edges: {len(graph.edges)}")
    print()
    print(graph.to_json(indent=2))


if __name__ == "__main__":
    main()
