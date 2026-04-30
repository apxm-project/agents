# APXM – Agent Programming eXecution Model

APXM is a full toolchain for building autonomous agent workflows:

- **AIR** as the canonical human-readable graph source
- **AIS MLIR dialect** as the compiler representation
- **`.apxmobj` artifacts** as deterministic compiled workflows
- **Runtime** with scheduling, memory, tools, backend routing, and graph metrics
- **Dekk-first CLI** for install, compile, execute, and backend setup

---

## Quick Start

**New installation:**
Install Dekk first if it is not already available on your machine.

```bash
git clone https://github.com/randreshg/apxm
cd apxm
dekk apxm install --no-interactive
dekk apxm doctor
```

**Optional repo-local vLLM fork:**
```bash
# Stock vLLM does not consume APXM's vLLM extension hints.
dekk apxm vllm install
dekk apxm vllm doctor
dekk apxm vllm start <MODEL_REF> --served-model-name <SERVED_MODEL_ID> --wait
dekk apxm vllm probe
dekk apxm vllm enable <SERVED_MODEL_ID>
```

**What the installer does:**
- ✓ Detects platform and package manager (via dekk)
- ✓ Creates a repo-local conda environment with MLIR/LLVM 22
- ✓ Installs Rust nightly if needed
- ✓ Builds the APXM binary
- ✓ Runs APXM commands without manual `PATH` or `LD_LIBRARY_PATH` edits when invoked through `dekk apxm ...`

## Prerequisites

- **Python** 3.10+ (for the CLI)
- **mamba** or **conda** ([miniforge](https://github.com/conda-forge/miniforge) recommended)
- **Git**

Optional (installer can set these up automatically):
- Rust nightly
- CMake >= 3.20

Run `dekk apxm doctor` to check your environment automatically.

---

## CLI Commands

```bash
dekk apxm doctor                           # Check environment
dekk apxm install --no-interactive        # Install/update APXM
dekk apxm compile <file.air> -o out.apxmobj
dekk apxm execute <file.air>
dekk apxm execute <file.air> --trace debug
dekk apxm run <file.apxmobj>
dekk apxm validate <file.air>
dekk apxm analyze <file.air>
dekk apxm vllm install
dekk apxm vllm start <MODEL_REF> --served-model-name <SERVED_MODEL_ID> --wait
dekk apxm vllm enable <SERVED_MODEL_ID>
```

Run `dekk apxm --help` for complete command reference.

---

## Runtime Tracing

APXM includes a tracing system for debugging and performance analysis:

```bash
dekk apxm execute graph.air                # Silent execution
dekk apxm execute graph.air --trace info   # High-level execution flow
dekk apxm execute graph.air --trace debug  # Detailed worker/operation info
dekk apxm execute graph.air --trace trace  # Full verbosity (tokens, LLM calls)
```

Trace targets: `apxm::scheduler`, `apxm::ops`, `apxm::llm`, `apxm::tokens`, `apxm::dag`

---

## LLM Operations

APXM provides three core LLM operations with different reasoning characteristics:

| Operation | Purpose | Example |
|-----------|---------|---------|
| `ask` | Simple Q&A with LLM | `ais.ask "What is APXM?"` |
| `think` | Extended thinking with token budget | `ais.think "Analyze {problem}" {budget = 1000}` |
| `reason` | Structured reasoning with belief updates | `ais.reason "Update beliefs from {evidence}"` |

```mlir
module {
  func.func @llm_ops(%topic: !ais.token) -> !ais.token attributes {ais.entry = true} {
    %background = ais.ask "Explain the domain background of {topic}" [%topic : !ais.token]
      {input_names = ["topic"]} : !ais.token
    %analysis = ais.think "Analyze the implications of {background}" [%background : !ais.token]
      {input_names = ["background"], budget = 2000 : i64} : !ais.token
    %answer = ais.reason "Summarize the next action from {analysis}" [%analysis : !ais.token]
      {input_names = ["analysis"]} : !ais.token
    ais.return %answer : !ais.token
  }
}
```

Context operands (comma-separated) are appended at runtime as:

```
<template>

Context 1: <value>
Context 2: <value>
```

The `+` operator merges tokens; use the comma form for context operands.

---

## IR Debugging

```bash
# Write per-pass MLIR snapshots to the given directory.
APXM_PRINT_IR_DIR=/tmp/apxm-ir dekk apxm compile file.air -o output.apxmobj

# Optional: print a one-line trace of IR printing config.
APXM_PRINT_IR_TRACE=1 APXM_PRINT_IR_DIR=/tmp/apxm-ir dekk apxm compile file.air -o output.apxmobj
```

---

## Configuration

Register backends through the CLI so generated frontend bindings and runtime
configuration stay aligned:

```bash
dekk apxm backend add <name> --type <cloud|onprem|local> --protocol <protocol> --endpoint <URL>
dekk apxm backend add-model <name> <SERVED_MODEL_ID> --alias <ROLE>
```

---

## Project Layout

```text
crates/
  core/
    apxm-core     # Shared downstream graph contract, types, constants, errors
    apxm-ais      # Authoring/codegen source for AIS ops, attrs, and pass generation
  compiler/
    apxm-compiler # MLIR compiler
    apxm-frontend # Python graph authoring frontend
  orchestration/
    apxm-driver   # Compiler+runtime orchestration
    apxm-artifact # Artifact format
    apxm-acp      # ACP protocol glue
  runtime/
    apxm-backends # LLM and storage backends
    apxm-credentials # Backend credentials and registry helpers
    apxm-runtime  # Execution engine
  tools/
    apxm-cli      # CLI tool
    apxm-server   # HTTP/MCP server
    apxm-gui      # Browser UI
examples/
  python/
    getting-started, parallelism, optimization, ...   # Feature-focused samples
    benchmarks/                                       # Shared harness + stress/
    demos/gemma4/                                     # Three skill-library proof points (ReviewSynthesis, context pruning, vLLM hints)
docs/             # Documentation (start at docs/README.md)
```

---

## Environment Diagnostics

The `dekk apxm doctor` command uses [dekk](https://github.com/randreshg/dekk) for comprehensive environment detection:

```bash
dekk apxm doctor
```

**What it checks:**
- **Platform** -- OS, architecture, Linux distro, WSL, containers
- **Dependencies** -- Rust (nightly), Cargo, CMake, Ninja, Git, LLVM
- **Conda environment** -- `apxm` env activation, Python version, MLIR/LLVM 22.x
- **Build status** -- whether the compiler binary has been built
- **Backend registry** -- registered backends, protocols, and endpoint wiring
- **CI environment** -- GitHub Actions, GitLab CI, Jenkins, and other providers (auto-detected)

Each check provides actionable fix suggestions when issues are found.

---

## Documentation

### Guides
- [Documentation Index](docs/README.md) — Entry point for the current docs set
- CLI Reference — Run `dekk apxm --help` for commands, options, and graph tooling
- [vLLM Backend](docs/backends/vllm.md) — Dekk-first optional backend setup and metrics
- [External vLLM Fork](docs/external-vllm-fork.md) — Architecture and fork integration notes

### PXM
- [Overview](docs/pxm/readme.md) — High-level overview of APXM as a program execution model
- [Foundations](docs/pxm/foundations.md) — How A-PXM draws on decades of PXM research
- [AAM](docs/pxm/aam.md) — Agent Abstract Machine state model
- [AIS](docs/pxm/ais.md) — Agent Instruction Set contract and typed operations
- [Compute](docs/pxm/compute.md) — Compute across 6 foundational PXMs
- [Memory](docs/pxm/memory.md) — Memory separation across PXMs
- [Scheduling](docs/pxm/scheduling.md) — Scheduling and execution across PXMs
- [Vision](docs/pxm/vision.md) — Long-range system direction

### AIS Operations
- Run `dekk apxm ops list` or `dekk apxm ops show <OP>` for the live AIS surface

### Design Notes
- [Compiler Pipeline](docs/compiler/pipeline.md) — Current pass pipeline and optimization stages
- [Guardrails and Handoffs](docs/design/guardrails_handoffs.md) — Design notes for safe inter-agent boundaries
- [Sessions](docs/design/sessions.md) — Durable session and checkpoint design
