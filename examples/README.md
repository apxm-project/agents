# APXM Examples

All examples use the Python frontend (`apxm.graph`). See [python/README.md](python/README.md) for the full guide.

## Quick Start

```bash
cd crates/apxm-frontend/python
export PYTHONPATH="$PWD:$PYTHONPATH"

# Emit canonical .air from a Python workflow
python3 ../../examples/python/basics/hello.py > ../../examples/python/basics/hello.air

# Execute the Python file directly through the CLI
apxm execute ../../examples/python/basics/hello.py

# Or hand the emitted .air file to the compiler/runtime
apxm execute ../../examples/python/basics/hello.air
```
`examples/python/*.py` now emit `.air` by default when run directly, and the CLI execute path
accepts `.py` inputs by running Python, capturing that `.air`, and feeding it to the compiler.

## Structure

```
examples/python/
├── basics/          # Core: ask, think, tool use
├── acp-agents/      # ACP spawn + communicate patterns
├── multi-agent/     # Multi-agent coordination
├── patterns/        # Reusable patterns (worker pool, negotiation, etc.)
└── workflows/       # Complex multi-phase workflows
```
