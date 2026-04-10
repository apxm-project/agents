# Self-Hosted APXM Workflows

This directory contains APXM workflows that use APXM to build APXM itself. These workflows demonstrate the power of the APXM programming model by orchestrating real coding agents (Claude Code, Codex) to modify the APXM codebase.

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

### Compile a workflow

```bash
# Generate .air file
PYTHONPATH=crates/compiler/apxm-frontend/python python3 examples/python/self-hosted/add_op.py > /tmp/add_op.air

# Compile to artifact
dekk apxm compile /tmp/add_op.air -o /tmp/add_op.apxmobj
```

### Execute a workflow

```bash
# Execute with parameters
dekk apxm execute /tmp/add_op.air "MyNewOp" "A new operation that does X"

# With session output for debugging
dekk apxm execute /tmp/add_op.air --emit-session "MyNewOp" "A new operation that does X"
```

### Inspect session output

```bash
# List sessions
ls ~/.apxm/sessions/

# View live progress
cat ~/.apxm/sessions/<id>/live.json

# View per-node status
cat ~/.apxm/sessions/<id>/node_statuses.json

# View all node outputs
cat ~/.apxm/sessions/<id>/results.json

# Replay timeline
dekk apxm replay ~/.apxm/sessions/<id>
```

## Example: Adding a new operation

```bash
# Compile the add_op workflow
PYTHONPATH=crates/compiler/apxm-frontend/python python3 examples/python/self-hosted/add_op.py > /tmp/add_op.air

# Execute it with parameters
dekk apxm execute /tmp/add_op.air --emit-session "CHECKPOINT" "Save execution state for later resume"

# Monitor progress
watch -n 1 cat ~/.apxm/sessions/*/live.json

# After completion, view the review
cat ~/.apxm/sessions/*/results.json | jq '.nodes[] | select(.name == "print_review")'
```

## How It Works

These workflows use the Python `@compile` decorator from `apxm.graph`. The decorator:
1. Records graph construction operations (spawn, ask, think, etc.)
2. Builds an in-memory graph representation
3. Emits `.air` text format (APXM Intermediate Representation)

The `.air` file is then:
1. Compiled by the APXM compiler (Rust + MLIR backend)
2. Lowered to AIS operations
3. Optimized (e.g., FuseReasoning pass)
4. Emitted as a binary artifact (`.apxmobj`)
5. Executed by the APXM runtime scheduler

During execution:
- Agents are spawned via ACP (Agent Communication Protocol)
- Operations execute in parallel where possible
- Session traces capture all events live
- Results are written to the session directory

This is "APXM building APXM" — a self-hosted toolchain where the compiler and runtime orchestrate coding agents that modify the compiler and runtime themselves.
