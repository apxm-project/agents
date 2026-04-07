#!/usr/bin/env python3
"""memory_rag_pipeline.py - Memory-augmented RAG

Recall from LTM, answer with context, verify, and store.

Usage: python3 -m examples.python.patterns.memory-rag.memory_rag_pipeline
"""

from apxm.graph import compile, GraphRecorder


@compile()
def memory_rag_pipeline(g: GraphRecorder):
    """Memory-augmented RAG pattern with verification."""
    # User query
    query = g.ask(
        "query",
        template="What are the trade-offs between Rust async runtimes: Tokio vs async-std vs smol?"
    )

    # Recall from memory
    recall_ltm = g.query_memory("recall_ltm", query="rust async runtimes comparison")

    # Generate answer using memory context
    answer = g.think(
        "answer",
        template="Answer this question, using the memory recall as context if relevant.\n\n"
        "QUESTION:\n{0}\n\nMEMORY RECALL:\n{1}\n\n"
        "Provide a thorough, accurate answer."
    )
    query | answer
    recall_ltm | answer

    # Verify the answer
    verification = g.think(
        "verification",
        template="Verify this answer is accurate, complete, and balanced.\n\n"
        "QUESTION:\n{0}\n\nANSWER:\n{1}\n\n"
        "Is this correct? Any important omissions or errors?"
    )
    query | verification
    answer | verification

    # Store the answer in memory
    mem = g.update_memory(
        "store_answer",
        data="{0}",
        key="rust_async_runtimes_comparison"
    )
    answer | mem

    # Print outputs
    print1 = g.print_("print_answer", message="=== ANSWER ===\n{0}")
    answer | print1

    print2 = g.print_("print_verification", message="=== VERIFICATION ===\n{0}")
    verification | print2
    print1 >> print2
    mem >> print2

    g.return_("result", source=print2)
    


if __name__ == "__main__":
    import json
    print(memory_rag_pipeline._graph.to_air())
