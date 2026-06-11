# APXM Examples

## Why APXM?

APXM compiles agent workflows into optimized execution plans. Instead of writing
imperative coordination code, you declare *what* your agents should do and the
compiler figures out *how* to run it efficiently:

- **Implicit parallelism** -- independent nodes run concurrently without manual threading
- **Compiler optimizations** -- prompt construction, dead context elimination, template specialization, scheduling hints, and graph metrics
- **Multi-agent coordination** -- native spawn, communicate, and team primitives
- **Multi-provider routing** -- assign the right model to each task (fast, powerful, local)

## Quick Start

```bash
# From project root
dekk apxm doctor
dekk apxm compile examples/python/getting-started/tool_use.py -o /tmp/apxm-tool-use.apxmobj
```

Use Dekk for normal runs. It sets up the APXM environment consistently across
machines.

## Dependencies

Required:

- `dekk apxm install --no-interactive`
- `dekk apxm doctor`

Optional, depending on the example:

- A registered LLM backend for examples that execute `ask`, `think`, or
  `reason` nodes against a real model.
- Generated typed ACP profile imports for examples that spawn coding agents. Verify
  them with `dekk apxm agent list` and `dekk apxm agent test <name>`.
- Node/npm plus the relevant authenticated agent CLI when using APXM ACP
  profiles. The checked-in `claude` profile launches
  `npx -y @agentclientprotocol/claude-agent-acp@^0.24.2`; the checked-in
  `codex` profile launches `npx @zed-industries/codex-acp@^0.10.0`.
- The APXM vLLM fork for `self-hosted/vllm_graph_smoke.py`; see
  `docs/backends/vllm.md`.
- `jq` and `rg` only for inspection/reporting commands that mention
  them.

For backend-free validation, prefer compile-only checks or examples that set
`mock=True` in Python. Do not bake a model id into a public example; register a
backend with `dekk apxm backend ...` and select it from the APXM backend
registry.

Most examples use the Python frontend. Dekk can compile Python examples directly:

```bash
dekk apxm compile examples/python/getting-started/hello.py -o hello.apxmobj
dekk apxm run hello.apxmobj
```

> **First time?** See the [docs/](../docs/README.md) overview, then run
> `dekk apxm doctor` to verify your environment.

## Learning Path

1. **[getting-started/](python/getting-started/)** -- First contact: hello world, tool use
2. **[parallelism/](python/parallelism/)** -- Fan-out patterns, implicit DAG scheduling
3. **[optimization/](python/optimization/)** -- Compiler passes, graph hints, and metrics
4. **[multi-agent/](python/multi-agent/)** -- Spawn, communicate, team coordination
5. **[multi-provider/](python/multi-provider/)** -- Route tasks to different models
6. **[memory/](python/memory/)** -- Three-tier memory and RAG
7. **[patterns/](python/patterns/)** -- Reusable workflow patterns
8. **[real-world/](python/real-world/)** -- Complete production workflows
9. **[native-tools/](python/native-tools/)** -- Native Python agent/tool handoff
10. **[workflows/](workflows/goals/)** -- Native `.apxmw` workflow coordination, event loops, resume, and cancel
11. **[self-hosted/](python/self-hosted/)** -- APXM building APXM and optional vLLM demos

See [python/README.md](python/README.md) for the full API reference and structure.
