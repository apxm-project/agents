# Memory

Three-tier memory: short-term (STM), long-term (LTM), and retrieval-augmented generation.

## Why This Matters

Agent workflows need context that spans beyond a single LLM call. APXM's AAM
(Agent Abstract Machine) provides structured memory operations that the compiler
can reason about and optimize.

## Examples

- **rag_pipeline.py** -- Query memory, inject context, update knowledge. `dekk agents execute examples/python/memory/rag_pipeline.py`

## Key API

```python
# Store to long-term memory
g.update_memory("store", data=analysis, key="project_analysis")

# Query memory for context
context = g.query_memory("recall", query="previous analyses")

# Use retrieved context in downstream nodes
context >> synthesis
```

## Learn More

- [docs/README.md](../../../docs/README.md) -- AAM memory architecture
