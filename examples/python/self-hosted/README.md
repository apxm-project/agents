# Self-Hosted APXM Workflows

This directory contains APXM workflows that use APXM to build APXM itself. They
coordinate APXM ACP agent profiles and optional self-hosted vLLM routes.

## Requirements

- Run through Dekk: `dekk agents execute ...`.
- Agent workflows require APXM ACP profile imports and authenticated local CLIs.
  Check them with `dekk agents agent list` and `dekk agents agent test <name>`.
- The checked-in `claude` profile uses
  `npx -y @agentclientprotocol/claude-agent-acp@^0.24.2`; the checked-in
  `codex` profile uses `npx -y @zed-industries/codex-acp@^0.16.0`.
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

### 6. `autofix_workflow.py` — Autofix as APXM Workflow

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
dekk agents execute examples/python/self-hosted/add_op.py \
  "MyNewOp" "A new operation that does X"
```

### Compile canonical AIR

```bash
RUN_DIR="$(mktemp -d)"
dekk agents agent build <source-package>
dekk agents compile-service-canonical <source-package> > "${RUN_DIR}/add_op.air.json"
```

### Capture a session

```bash
RUN_DIR="$(mktemp -d)"
dekk agents execute \
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
dekk agents execute \
  --emit-session "${RUN_DIR}/session" \
  examples/python/self-hosted/add_op.py \
  "CHECKPOINT" "Save execution state for later resume"

jq '.nodes[] | select(.name == "print_review")' \
  "${RUN_DIR}/session"/*/results.json
```

## How It Works

These workflows use the Python `@compile` decorator. The decorator:
1. Records workflow construction operations (spawn, ask, think, etc.)
2. Builds an in-memory workflow representation
3. Hands the workflow to the APXM compiler

The workflow is then lowered to AIS operations, optimized, emitted as an
`.apxmobj` artifact, and executed by the APXM runtime scheduler.

During execution:
- Agents are spawned via ACP (Agent Communication Protocol)
- Operations execute in parallel where possible
- Session traces capture all events live
- Results are written to the session directory

The result is a self-hosted toolchain where the compiler and runtime
coordinate coding agents that modify the compiler and runtime themselves.
