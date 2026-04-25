# Self-Hosted APXM Workflows

This directory contains APXM workflows that use APXM to build APXM itself. They
orchestrate registered coding agents and optional self-hosted vLLM routes.

## Requirements

- Run through Dekk: `dekk apxm execute ...`.
- Agent workflows require generated ACP profiles and authenticated local CLIs.
  Check them with `dekk apxm agent list` and `dekk apxm agent test <name>`.
- The checked-in `claude` profile uses
  `npx -y @agentclientprotocol/claude-agent-acp@^0.24.2`; the checked-in
  `codex` profile uses `npx @zed-industries/codex-acp@^0.10.0`.
- vLLM workflows are optional and require a running repo-local
  `external/vllm` fork plus a registered served model alias. See
  `docs/backends/vllm.md`.

## Workflows

### 1. `add_op.py` — Add New Operation

Orchestrates the full 8-step procedure to add a new AIS operation:
- Spawns architect (Claude) to analyze the create-op workflow
- Spawns compiler_dev (Claude) for compiler-side implementation
- Spawns runtime_dev (Codex) for runtime-side implementation
- Spawns reviewer (Claude) to verify the implementation

**Parameters:** `op_name` (str), `op_description` (str)

### 2. `remove_op.py` — Remove Operation

Safely removes an operation from the full stack:
- Analyzes impact across the codebase
- Removes from compiler (enum, MLIR, wire index)
- Removes from runtime (handler, dispatcher, tests)
- Verifies nothing broke

**Parameters:** `op_name` (str)

### 3. `explore.py` — AI Council / Explore Solution

5-way parallel exploration with adversarial synthesis:
- Architect (systems design perspective)
- Adversary (what breaks, what's over-engineered)
- Implementer (concrete Rust implementation)
- Researcher (literature/industry perspective)
- User advocate (user needs)

**Parameters:** `question` (str)

### 4. `plan_feature.py` — Generate Implementation Plan

Takes a feature request and produces a detailed implementation plan:
- Architect analyzes project structure
- Gap analysis (what's missing vs what exists)
- Risk analysis (what could go wrong)
- Crate ordering (bottom-up dependency order)
- Structured plan with file paths and test strategy

**Parameters:** `feature` (str)

### 5. `refactor.py` — Refactor Module

Refactors a module/crate:
- Analyzer identifies refactoring opportunities
- Implementer does the refactoring
- Test runner verifies nothing broke

**Parameters:** `target` (str), `goal` (str)

### 6. `autofix_workflow.py` — Autofix as APXM Graph

The autofix loop as a native APXM workflow:
- Runs autofix validation, classifies failures
- Spawns parallel fixers per failure cluster
- Verifies all fixes
- Reports results

**Parameters:** `scope` (str, default "examples/python")

## Usage

All workflows follow the same pattern:

### Execute a workflow

```bash
dekk apxm execute examples/python/self-hosted/add_op.py \
  "MyNewOp" "A new operation that does X"
```

### Compile an artifact

```bash
RUN_DIR="$(mktemp -d)"
dekk apxm compile \
  examples/python/self-hosted/add_op.py \
  -o "${RUN_DIR}/add_op.apxmobj"
dekk apxm run "${RUN_DIR}/add_op.apxmobj" \
  "MyNewOp" "A new operation that does X"
```

### Capture a session

```bash
RUN_DIR="$(mktemp -d)"
dekk apxm execute \
  --emit-session "${RUN_DIR}/session" \
  examples/python/self-hosted/add_op.py \
  "MyNewOp" "A new operation that does X"
```

### Inspect session output

```bash
find "${RUN_DIR}/session" -maxdepth 2 -type f | sort
cat "${RUN_DIR}/session"/*/results.json
```

## Example: Adding a new operation

```bash
RUN_DIR="$(mktemp -d)"
dekk apxm execute \
  --emit-session "${RUN_DIR}/session" \
  examples/python/self-hosted/add_op.py \
  "CHECKPOINT" "Save execution state for later resume"

jq '.nodes[] | select(.name == "print_review")' \
  "${RUN_DIR}/session"/*/results.json
```

## How It Works

These workflows use the Python `@compile` decorator. The decorator:
1. Records graph construction operations (spawn, ask, think, etc.)
2. Builds an in-memory graph representation
3. Hands the graph to the APXM compiler

The graph is then lowered to AIS operations, optimized, emitted as an
`.apxmobj` artifact, and executed by the APXM runtime scheduler.

During execution:
- Agents are spawned via ACP (Agent Communication Protocol)
- Operations execute in parallel where possible
- Session traces capture all events live
- Results are written to the session directory

The result is a self-hosted toolchain where the compiler and runtime
orchestrate coding agents that modify the compiler and runtime themselves.
