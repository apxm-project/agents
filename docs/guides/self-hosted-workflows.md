# Self-Hosted Workflows: Using APXM to Build APXM

APXM can orchestrate its own development -- "dogfooding" the agent workflow system to add features, fix bugs, and refactor code. This guide shows how to use APXM's built-in development workflows.

---

## Why Self-Host Development?

Traditional development requires reading docs, understanding codebase structure, planning across compiler and runtime, writing code, and iterating on tests. Self-hosted development replaces most of that with:

```bash
apxm execute add_op.apxm "CHECKPOINT" "Save execution state"
```

Wait ~3 minutes, then review the output from autonomous agents.

**Benefits:**
- **Consistency:** Agents follow established patterns, naming, and structure.
- **Coverage:** Changes touch all layers (AIS enum, MLIR, TableGen, runtime handler, tests).
- **Speed:** Parallel agents (architect + compiler_dev + runtime_dev) finish in minutes.
- **Auditability:** Session outputs preserve full reasoning and decisions.

---

## Available Self-Hosted Workflows

APXM includes development workflows in `examples/python/self-hosted/`:

| Workflow | Purpose | Agents | Duration |
|----------|---------|--------|----------|
| `add_op.apxm` | Add new AIS operation | 4 (architect, compiler_dev, runtime_dev, reviewer) | ~3 min |
| `remove_op.apxm` | Remove deprecated operation | 3 (analyzer, remover, tester) | ~2 min |
| `explore.apxm` | Research feature feasibility | 2 (researcher, critic) | ~5 min |
| `plan_feature.apxm` | Design multi-operation feature | 3 (architect, planner, reviewer) | ~4 min |
| `refactor.apxm` | Refactor module/subsystem | 3 (analyzer, refactorer, validator) | ~6 min |
| `autofix_workflow.apxm` | Fix failing tests | 2 (debugger, fixer) | ~3 min |
| `audit.apxm` | Code quality audit | 1 (auditor) | ~4 min |

All workflows emit session output to `~/.apxm/sessions/<id>/` with full trace, agent workspaces, and artifacts.

---

## Example 1: Add New AIS Operation

**Usage:**
```bash
apxm execute examples/python/self-hosted/add_op.apxm \
  --emit-session \
  -- "CHECKPOINT" "Save execution state for later resume"
```

**What happens:**

1. **Spawn architect** (Claude) -- reads `definitions.rs`, `AISOps.td`, `ArtifactEmitter.cpp`, and example handlers; produces an implementation plan (wire index, attributes, MLIR mnemonic, handler logic).
2. **Spawn compiler_dev and runtime_dev in parallel** -- compiler_dev modifies `definitions.rs`, `AISOps.td`, `ArtifactEmitter.cpp`; runtime_dev creates `handlers/checkpoint.rs`, updates `executor/mod.rs`, writes tests. Both run concurrently (2x speedup).
3. **Spawn reviewer** (Claude) -- checks wire index consistency, verifies attribute names match, runs build and tests, reports pass/fail with fix suggestions.
4. **Synthesize** -- final summary with operation name, wire index, status, and test results.

**Session output:**
```
~/.apxm/sessions/add-checkpoint-20260408-143022/
+-- manifest.json
+-- input.apxm
+-- trace.ndjson
+-- results.json
+-- metrics.json
+-- nodes/
    +-- 01_architect/
    |   +-- CLAUDE.md
    |   +-- output.json
    |   +-- trace.ndjson
    +-- 02_compiler_dev/
    |   +-- CLAUDE.md
    |   +-- output.json
    +-- 03_runtime_dev/
    |   +-- CLAUDE.md
    |   +-- output.json
    +-- 04_reviewer/
        +-- CLAUDE.md
        +-- output.json
```

---

## Example 2: Explore Feature Feasibility

**Usage:**
```bash
apxm execute examples/python/self-hosted/explore.apxm \
  --emit-session \
  -- "Should APXM support streaming LLM responses?"
```

**What happens:**

1. **Researcher** (Claude) reads relevant codebase sections, reviews existing architecture, researches streaming APIs, and proposes an implementation approach.
2. **Critic** (Claude) reviews the proposal, identifies challenges, flags risks, and suggests alternatives.
3. **Synthesis** produces a final recommendation (feasible/not feasible), implementation complexity, dependencies, and estimated effort.

---

## Example 3: Automated Code Audit

**Usage:**
```bash
apxm execute examples/python/self-hosted/audit.apxm \
  --emit-session \
  -- "apxm-server"
```

**What happens:**

1. **Auditor** (Claude) reads all `.rs` files in the target crate and identifies: hardcoded strings, error handling issues (`.unwrap()`, `panic!`), performance anti-patterns, missing documentation, and test coverage gaps.
2. Findings are categorized by severity (CRITICAL/HIGH/MEDIUM/LOW) with file:line locations and suggested fixes.

---

## Example 4: Automated Refactoring

**Usage:**
```bash
apxm execute examples/python/self-hosted/refactor.apxm \
  --emit-session \
  -- "apxm-credentials/src/validate.rs" "Extract per-protocol validation functions"
```

**What happens:**

1. **Analyzer** (Claude) reads the target file, identifies current structure, proposes a refactoring plan.
2. **Refactorer** (Codex) implements the refactoring following the plan, preserving behavior.
3. **Validator** (Claude) runs `cargo test`, checks `cargo fmt` and `cargo clippy`, verifies no behavior changes.

---

## Authoring Your Own Workflows

### Basic Template

```python
from apxm import compile, GraphRecorder
from apxm._generated.agents import claude, codex
import os


@compile()
def my_dev_workflow(g: GraphRecorder, task_description: str):
    """Custom development workflow."""
    g.param("task_description", "str")

    cwd = os.environ.get("APXM_HOME", os.getcwd())

    planner = g.spawn("planner", profile=claude, cwd=cwd)
    implementer = g.spawn("implementer", profile=codex, cwd=cwd)
    tester = g.spawn("tester", profile=claude, cwd=cwd)

    planner.ask("Create implementation plan: {task_description}")
    plan = planner.get_last_node()
    g.print("=== PLAN ===\n{plan}")

    implementer.ask("Implement this plan: {plan}")
    impl = implementer.get_last_node()
    g.print("=== IMPLEMENTATION ===\n{impl}")

    tester.ask("Test this implementation: {impl}")
    test_results = tester.get_last_node()
    g.print("=== TESTS ===\n{test_results}")

    summary = g.think("Synthesize: Plan={plan}, Impl={impl}, Tests={test_results}")
    g.done(summary)


if __name__ == "__main__":
    print(my_dev_workflow._graph.to_air())
```

### Emit and Run

```bash
# Generate .apxm file
PYTHONPATH=crates/apxm-frontend/python python3 my_workflow.py > my_workflow.apxm

# Execute with session trace
apxm execute my_workflow.apxm --emit-session -- "Add logging to executor"

# Review session
apxm replay ~/.apxm/sessions/<id>
```

---

## Built-in Skills for Agents

Self-hosted workflows can use APXM-specific skills from `.agents/skills/`:

| Skill | Purpose | Command |
|-------|---------|---------|
| `/ops` | List all AIS operations | `apxm ops list` |
| `/compile` | Compile graph | `apxm compile graph.apxm` |
| `/execute` | Run workflow | `apxm execute graph.apxm` |
| `/build` | Build APXM | `apxm build` |
| `/test` | Run tests | `apxm test` |
| `/validate` | Validate graph | `apxm validate graph.apxm` |
| `/analyze` | Analyze graph | `apxm analyze graph.apxm` |

APXM injects skill definitions into each agent's `CLAUDE.md` context file within the node workspace.

---

## Session Management

### View Past Sessions

```bash
ls ~/.apxm/sessions/
```

### Replay Session Timeline

```bash
apxm replay ~/.apxm/sessions/add-checkpoint-20260408-143022
```

Output shows a timeline of spawns, completions, agents used, total LLM calls, token counts, and cost.

### Extract Node Outputs

```bash
cat ~/.apxm/sessions/<id>/nodes/01_architect/output.json | jq -r '.result'
```

---

## Best Practices

1. **Use `--emit-session`** for all self-hosted workflows (reproducibility and audit trail).
2. **Parameterize workflows** instead of hardcoding task descriptions.
3. **Review agent outputs** before committing -- agents make mistakes.
4. **Test generated code** by running `cargo test` manually.
5. **Track costs** -- self-hosted workflows use LLM credits ($0.30-$0.50 per run with gpt-4).
6. **Use multi-model routing** -- assign cheap models for non-critical agents to reduce cost by ~75%.

---

## Full Development Loop

```bash
# 1. Explore feasibility
apxm execute explore.apxm --emit-session -- "Add CHECKPOINT operation"

# 2. Plan implementation
apxm execute plan_feature.apxm --emit-session -- "CHECKPOINT operation"

# 3. Implement operation
apxm execute add_op.apxm --emit-session -- "CHECKPOINT" "Save execution state"

# 4. Fix issues manually based on reviewer feedback

# 5. Audit for quality
apxm execute audit.apxm --emit-session -- "apxm-runtime"

# 6. Commit
git add -A && git commit -m "feat(ais): add CHECKPOINT operation"
```

---

## Limitations

1. **Agent hallucination** -- agents may propose incorrect implementations.
2. **Context limits** -- large codebases may exceed agent context windows.
3. **No execution guarantees** -- generated code may not compile or pass tests.
4. **Cost** -- multiple agents with gpt-4 adds up ($0.30-$0.50 per workflow).
5. **Requires review** -- never blindly commit agent output.

**Mitigations:** use reviewer agents to validate output, run tests within workflows, emit session traces for audit trails, and iterate on prompts to improve quality.

---

## See Also

- [Multi-Agent Workflows](multi-agent.md) -- Spawning and coordinating agents
- [Multi-Model Routing](multi-model.md) -- Cost optimization via model assignment
- [Optimization Overview](../optimization/overview.md) -- Reducing workflow latency and cost
- [Session Output](../implementation/runtime/sessions.md) -- Session directory layout
- [Graph Format Reference](../reference/graph-format.md) -- `.apxm` file specification
- Example source code: `examples/python/self-hosted/`
