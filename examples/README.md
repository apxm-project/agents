# APXM Examples

All examples use the Python frontend (`apxm.graph`). See [python/README.md](python/README.md) for the full guide.

## Quick Start

```bash
cd crates/apxm-frontend/python
export PYTHONPATH="$PWD:$PYTHONPATH"

# Run an example
python3 ../../examples/python/basics/hello.py

# Execute through the CLI (compiles Python → JSON → runtime)
apxm execute examples/python/basics/hello.py
```

## Structure

```
examples/python/
├── basics/          # Core: ask, think, tool use
├── acp-agents/      # ACP spawn + communicate patterns
├── multi-agent/     # Multi-agent coordination
├── patterns/        # Reusable patterns (worker pool, negotiation, etc.)
└── workflows/       # Complex multi-phase workflows
```
