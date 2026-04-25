#!/usr/bin/env python3
"""memory_rag_pipeline.py - Memory-augmented RAG

Recall from LTM, answer with context, verify, and store.

Usage: dekk apxm execute examples/python/memory/rag_pipeline.py
"""

from apxm import compile, GraphRecorder


@compile()
def memory_rag_pipeline(g: GraphRecorder):
    """Memory-augmented RAG pattern with verification."""
    # User query
    query = g.ask(
        name="query",
        prompt="What are the trade-offs between Rust async runtimes: Tokio vs async-std vs smol?"
    )

    # Recall from memory (the runtime will make context available via AAM)
    recall_ltm = g.query_memory("recall_ltm", query="rust async runtimes comparison")

    # Generate answer using memory context
    # The runtime provides memory context through AAM, so we don't need to wire recall_ltm
    answer = g.think(
        name="answer",
        prompt="Answer this question, using any relevant context from memory.\n\n"
        "QUESTION:\n{query}\n\n"
        "Provide a thorough, accurate answer about Rust async runtimes."
    )
    g.add_edge(recall_ltm, answer, dependency="Control")  # Ensure memory is queried first

    # Verify the answer
    verification = g.think(
        name="verification",
        prompt="Verify this answer is accurate, complete, and balanced.\n\n"
        "QUESTION:\n{query}\n\nANSWER:\n{answer}\n\n"
        "Is this correct? Any important omissions or errors?"
    )

    # Store the answer in memory
    mem = g.update_memory(
        "store_answer",
        data=answer,
        key="rust_async_runtimes_comparison"
    )
    g.add_edge(answer, mem)

    # Print outputs
    print1 = g.print(message="=== ANSWER ===\n{answer}")

    print2 = g.print(message="=== VERIFICATION ===\n{verification}")
    g.add_edge(print1, print2, dependency="Control")
    g.add_edge(mem, print2, dependency="Control")

    g.done(print2)
    


if __name__ == "__main__":
    import apxm

    result = apxm.run(memory_rag_pipeline())
    print(result.content)
