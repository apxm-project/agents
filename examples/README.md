# APXM Examples

## Directory Structure

```
examples/
├── python/         Authoring examples in the Python frontend
├── basics/         Precompiled artifacts and `.air` snapshots
├── multi-agent/    Precompiled council and coordination workflows
├── acp-agents/     Precompiled ACP protocol workflows
├── patterns/       Advanced graph patterns and artifacts
└── workflows/      Production workflow artifacts
```

## Python Authoring

| File | Description |
|------|-------------|
| `python/basics/hello.py` | Minimal greeting workflow that emits graph JSON |
| `python/basics/tool_use.py` | Capability registration and tool invocation |
| `python/patterns/iterative-refine/iterative_refine.py` | Unrolled refinement loop |
| `python/patterns/plan-fan-out/plan_fan_out.py` | Plan once, fan out, then synthesize |

## Precompiled Artifacts

Compiled `.apxmobj` files are retained for larger workflows that have not been
ported to first-party Python examples yet. `.air` files are debug snapshots only.

## ACP Agents

Spawn+communicate and INV-style ACP workflows are available as `.apxmobj`
artifacts under `examples/acp-agents/`.

## Patterns

See the Python pattern examples for authoring references, and the sibling
artifact directories for runnable compiled workflows.

## Workflows

Large workflow directories now primarily contain compiled `.apxmobj` artifacts.
The legacy DSL sources were removed as part of the Python frontend migration.

## Running Examples

```bash
# Emit a graph from Python
PYTHONPATH=crates/apxm-frontend/python \
  python3 examples/python/basics/hello.py > /tmp/hello.json

# Validate and execute the JSON graph
dekk apxm validate /tmp/hello.json
dekk apxm execute /tmp/hello.json

# Compile to an artifact
dekk apxm compile /tmp/hello.json -o /tmp/hello.apxmobj

# Run a precompiled artifact
dekk apxm run examples/basics/hello.apxmobj
```

See `docs/guides/getting-started.md` for detailed tutorials.
