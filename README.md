# APXM – Agent Programming eXecution Model

APXM is a full toolchain for building autonomous agents:

- **ApxmGraph IR** as the canonical frontend format
- **AIS dialect** as the compiler/lowering representation for that graph contract
- **Compiler** that lowers graph → AIS MLIR → executable artifacts
- **Runtime** with scheduler, memory system, and LLM registry
- **CLI** for compile/run graph execution

---

## Quick Start

**New installation:**
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
dekk apxm vllm serve <HF_MODEL_ID>
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
dekk apxm backend add vllm-fork --type onprem --protocol vllm --endpoint http://127.0.0.1:8916/v1
dekk apxm backend add-model vllm-fork <HF_MODEL_ID>
dekk apxm vllm install
dekk apxm vllm serve <HF_MODEL_ID>
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
| `ask` | Simple Q&A with LLM | `ask("What is 2+2?") -> answer` |
| `think` | Extended thinking with token budget | `think("Analyze this problem", budget: 1000) -> analysis` |
| `reason` | Structured reasoning with belief updates | `reason("Solve step by step", context) -> solution` |

```json
{
  "name": "llm_ops",
  "nodes": [
    { "id": 1, "name": "ask", "op": "ASK", "attributes": { "template_str": "Explain the domain background of {topic}" } },
    { "id": 2, "name": "think", "op": "THINK", "attributes": { "template_str": "Analyze the implications", "budget": 2000 } },
    { "id": 3, "name": "reason", "op": "REASON", "attributes": { "template_str": "Execute step 1: {think}", "input_names": ["think"] } }
  ],
  "edges": [
    { "from": 1, "to": 2, "dependency": "Data" },
    { "from": 2, "to": 3, "dependency": "Data" }
  ],
  "parameters": [{ "name": "topic", "type_name": "str" }],
  "metadata": { "is_entry": true }
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
APXM_PRINT_IR_DIR=/tmp/apxm-ir apxm compiler compile file.air -o output.apxmobj

# Optional: print a one-line trace of IR printing config.
APXM_PRINT_IR_TRACE=1 APXM_PRINT_IR_DIR=/tmp/apxm-ir apxm compiler compile file.air -o output.apxmobj
```

---

## Configuration

Create `~/.apxm/config.toml`:

```toml
[chat]
providers = ["ollama"]
default_model = "ollama"

[[llm_backends]]
name = "ollama"
provider = "ollama"
model = "gpt-oss:20b-cloud"
endpoint = "http://localhost:11434"
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
examples/         # Sample ApxmGraph programs
docs/             # Documentation
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
- [External vLLM Fork](docs/external-vllm-fork.md) — Repo-local fork setup and backend registration

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
