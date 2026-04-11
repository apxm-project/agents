#!/usr/bin/env python3
"""memory_rag_pipeline.py - Memory-augmented RAG

Recall from LTM, answer with context, verify, and store.

Usage: python3 -m examples.python.patterns.memory-rag.memory_rag_pipeline
"""

from apxm import compile, GraphRecorder


@compile()
def memory_rag_pipeline(g: GraphRecorder):
    """Memory-augmented RAG pattern with verification."""
    # User query
    query = g.ask(
        "query",
        "What are the trade-offs between Rust async runtimes: Tokio vs async-std vs smol?"
    )

    # Recall from memory (the runtime will make context available via AAM)
    recall_ltm = g.query_memory("recall_ltm", query="rust async runtimes comparison")

    # Generate answer using memory context
    # The runtime provides memory context through AAM, so we don't need to wire recall_ltm
    answer = g.think(
        "answer",
        "Answer this question, using any relevant context from memory.\n\n"
        "QUESTION:\n{query}\n\n"
        "Provide a thorough, accurate answer about Rust async runtimes."
    )
    recall_ltm >> answer  # Control dependency to ensure memory is queried first

    # Verify the answer
    verification = g.think(
        "verification",
        "Verify this answer is accurate, complete, and balanced.\n\n"
        "QUESTION:\n{query}\n\nANSWER:\n{answer}\n\n"
        "Is this correct? Any important omissions or errors?"
    )

    # Store the answer in memory
    mem = g.update_memory(
        "store_answer",
        data=answer,
        key="rust_async_runtimes_comparison"
    )
    answer | mem

    # Print outputs
    print1 = g.print("=== ANSWER ===\n{answer}")

    print2 = g.print("=== VERIFICATION ===\n{verification}")
    print1 >> print2
    mem >> print2

    g.done(print2)
    


if __name__ == "__main__":
    import asyncio

    result = asyncio.run(memory_rag_pipeline())
    print(result.content)
