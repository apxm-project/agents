# apxm — Python frontend for APXM

The Python frontend for [APXM](https://github.com/apxm-project/agents),
the graph-aware dispatch + scheduling layer for vLLM.

This package lets you author APXM workflows in Python and emit AIR that the
APXM compiler/runtime executes.

## Quick start

```bash
pip install apxm
```

```python
from apxm import compile, GraphRecorder

@compile()
def hello(g: GraphRecorder, name: str) -> dict:
    """Say hello with an LLM."""
    greeting = g.ask(name="greet", prompt="Greet {name} warmly.")
    g.done(greeting)

print(hello.to_air())
```

## What's in the package

| Module | What it does |
| ------ | ------------ |
| `apxm` (root) | Graph DSL: `GraphRecorder`, `compile`, `ApxmGraph`, `AgentHandle`, `Team` |
| `apxm.contract` | APXM/vLLM operational names — env vars, route paths, `RepoLayout`, `build_layout()` |
| `apxm.data_config` | `.apxm/` data-bucket resolution against `.apxm/config.toml` |
| `apxm.paths` | Path helpers for examples and local scripts (`find_repo_root`, `repo_path`) |
| `apxm.constants` | Stable string constants re-exported from the typed enums in `_generated/` |
| `apxm._generated.agents` | Generated agent profiles (Claude, Codex) |
| `apxm._generated.models` | Generated model IDs |

## Running workflows

The Python package emits AIR. Executing it requires the APXM runtime/compiler
— install it separately and use the `dekk agents` CLI:

```bash
dekk agents execute path/to/workflow.py
```

See the main [APXM repo](https://github.com/apxm-project/agents) for the
runtime, the AIS dialect, the dekk CLI, and the operator workflow.

## Companion repos

- [apxm-project/eval](https://github.com/apxm-project/eval) —
  benchmark + evaluation harness, preregistrations, claim cards
- [apxm-project/vllm](https://github.com/apxm-project/vllm) — graph-aware
  vLLM fork used by APXM-vLLM

Provider-agnostic operating skills loaded by `apxm-server` are builtin under
`crates/server/skills/`; deployment-specific skills should be
installed through explicit skill roots.

## License

MIT. See [LICENSE](https://github.com/apxm-project/agents/blob/main/LICENSE).
