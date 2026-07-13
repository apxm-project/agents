# apxm — Python frontend for APXM

The Python authoring frontend for
[APXM](https://github.com/apxm-project/agents), the typed abstract machine,
compiler, and runtime for agent programs.

This package records APXM workflows as the compiler-owned `FrontendGraph` DTO.
`apxm emit-air` validates that DTO and renders canonical AIR; the compiler then
lowers AIR through MLIR into a `.apxmobj` artifact. TypeScript and Rust follow
the same contract.

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

The Python package emits AIR. Executing it requires the installed APXM
runtime/compiler CLI:

```bash
apxm execute path/to/workflow.py
```

Set `APXM_BIN` when the CLI is not named `apxm` on `PATH`. See the main
[APXM repo](https://github.com/apxm-project/agents) for the runtime, AIS
dialect, and operator workflow.

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
