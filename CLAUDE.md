# APXM -- Agent Programming eXecution Model

Compiler + dataflow runtime for agent workflows. Graphs of AIS (Agent Instruction Set) operations compile through MLIR to optimized artifacts, then execute on a parallel scheduler.

## CLI Commands

Global flag: `--json` emits machine-readable JSON for most commands.

### Discovery
```bash
apxm ops list                          # all 39 AIS ops, grouped by category
apxm ops list --category reasoning     # filter by category
apxm ops show ASK                      # detailed info + example JSON for one op
apxm template list                     # available starter graph patterns
apxm template show fan-out --json      # emit template as ready-to-use graph JSON
```

### Project Setup
```bash
apxm init                              # scaffold agents/, flows/, nodes/, prompts/, tools/ dirs + apxm.toml
```

### Graph Authoring
```bash
apxm validate graph.apxm              # check graph against AIS contract
apxm validate graph.apxm --json       # machine-readable validation errors
apxm validate graph.apxm --no-check-resources  # skip Tier 2 environment checks
apxm analyze graph.apxm               # parallelism, critical path, speedup estimate
apxm analyze graph.apxm --json        # full analysis as JSON
apxm explain graph.apxm               # human-readable summary of what a graph does
```

### Composition
```bash
apxm task merge a.apxm b.apxm --name combined   # merge graph fragments into one workflow
apxm task merge a.apxm b.apxm --name combined -o out.apxm
```

### Compilation & Execution
```bash
apxm compile graph.apxm               # compile graph to .apxmobj artifact
apxm compile graph.apxm -o out.apxmobj -O2          # with output path + opt level
apxm compile graph.apxm --emit-diagnostics diag.json # compilation statistics
apxm execute graph.apxm               # compile + run in one step
apxm execute graph.apxm -O0           # skip optimizations (e.g., FuseReasoning)
apxm execute graph.apxm --emit-metrics metrics.json  # runtime statistics
apxm execute graph.apxm --emit-session              # live tracing to session dir
apxm run out.apxmobj                   # run a pre-compiled artifact
apxm run out.apxmobj --emit-metrics metrics.json
apxm decompile out.apxmobj            # reverse-map artifact back to graph JSON
apxm replay ~/.apxm/sessions/<id>     # replay a session trace as timeline
```

### Environment
```bash
apxm doctor                            # diagnose MLIR/LLVM/conda dependencies
apxm doctor --json                     # machine-readable environment report
apxm activate --shell bash             # print shell exports for MLIR/LLVM env setup
apxm install                           # install/update conda env from environment.yaml
```

### Configuration
```bash
apxm backend add my-key --type cloud --protocol openai --api-key sk-...
apxm backend add local --type local --protocol ollama --endpoint http://localhost:11434
apxm backend list                      # show registered backends
apxm backend test                      # validate all backends
apxm backend test my-key               # validate one backend
apxm backend remove my-key             # delete a backend
apxm backend add-model my-key gpt-4o   # add model to a backend
apxm backend start <name>              # start local backend container
apxm backend stop <name>               # stop local backend container
apxm agent templates                   # list available built-in agent templates
apxm agent add claude                  # register agent from template (tests spawn)
apxm agent add claude --no-test        # register without spawn test
apxm agent add my-agent --command "my-agent --acp"  # register custom agent
apxm agent list                        # show registered agents
apxm agent test claude                 # test spawning a registered agent
apxm agent remove claude               # remove a registered agent
apxm tool add my-tool --description "..."    # register external tool for INV nodes
apxm tool list                         # list registered tools
apxm tool remove my-tool               # remove a tool registration
```

## Graph File Format

Graph files use the `.apxm` extension (JSON content). All graphs share this shape (see `apxm ops show <OP>` for per-op attributes):

```json
{"name": "...", "nodes": [{"id": 1, "name": "...", "op": "ASK", "attributes": {...}}], "edges": [{"from": 1, "to": 2, "dependency": "Data"}], "parameters": [], "metadata": {}}
```

Valid dependency types: `Data`, `Control`, `Effect`. Valid parameter types: `str`, `int`, `float`, `bool`, `json`.

## Session Output

When `--emit-session` is passed, APXM creates a session directory with:
- `manifest.json` — execution metadata (status updates live during execution)
- `input.apxm` — copy of the input graph
- `trace.ndjson` — NDJSON event stream (written live during execution)
- `live.json` — current progress snapshot (atomically updated)
- `results.json` — all node outputs
- `metrics.json` — execution metrics
- `node_statuses.json` — per-node status

## Build

Requires conda env with MLIR 21+. `cargo build -p apxm-cli --features driver` for full CLI.
