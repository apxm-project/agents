# APXM Examples

## Why APXM?

APXM compiles agent workflows into optimized execution plans. Instead of writing
imperative orchestration code, you declare *what* your agents should do and the
compiler figures out *how* to run it efficiently:

- **Implicit parallelism** -- independent nodes run concurrently without manual threading
- **Compiler optimizations** -- fusion, dead context elimination, shared prefix reuse
- **Multi-agent coordination** -- native spawn, communicate, and team primitives
- **Multi-provider routing** -- assign the right model to each task (fast, powerful, local)

## Quick Start

```bash
# From project root
dekk apxm execute examples/python/getting-started/hello.py
```

All examples use the Python frontend (`apxm.graph`). Each `.py` file emits
canonical `.air` (Agent IR) when run directly:

```bash
python3 examples/python/getting-started/hello.py > hello.air
dekk apxm compile hello.air -o hello.apxmobj
dekk apxm run hello.apxmobj
```

> **First time?** See [docs/getting-started.md](../docs/getting-started.md) for
> backend configuration and environment setup.

## Learning Path

1. **[getting-started/](python/getting-started/)** -- First contact: hello world, tool use
2. **[parallelism/](python/parallelism/)** -- Fan-out patterns, implicit DAG scheduling
3. **[optimization/](python/optimization/)** -- Compiler passes: fusion, DCE, shared prefix
4. **[multi-agent/](python/multi-agent/)** -- Spawn, communicate, team coordination
5. **[multi-provider/](python/multi-provider/)** -- Route tasks to different models
6. **[memory/](python/memory/)** -- Three-tier memory and RAG
7. **[patterns/](python/patterns/)** -- Reusable workflow patterns
8. **[real-world/](python/real-world/)** -- Complete production workflows
9. **[self-hosted/](python/self-hosted/)** -- APXM building APXM

See [python/README.md](python/README.md) for the full API reference and structure.
