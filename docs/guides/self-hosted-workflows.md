# Self-Hosted Workflows: Using APXM to Build APXM

APXM can orchestrate its own development — "dogfooding" the agent workflow system to add features, fix bugs, and refactor code. This guide shows how to use APXM's built-in development workflows.

---

## Why Self-Host Development?

Traditional development:
1. Read documentation
2. Understand codebase structure
3. Plan implementation across compiler + runtime
4. Write code manually
5. Test, debug, iterate

**Self-hosted development:**
1. Run `dekk apxm execute add_op.air "CHECKPOINT" "Save execution state"`
2. Wait ~3 minutes
3. Review PR from autonomous agents

**Benefits:**
- **Consistency:** Same patterns, naming, structure
- **Coverage:** Touches all layers (AIS enum, MLIR, TableGen, runtime handler, tests)
- **Speed:** Parallel agents (architect + compiler_dev + runtime_dev) finish in minutes
- **Documentation:** Session outputs preserve full reasoning and decisions

---

## Available Self-Hosted Workflows

APXM includes 6 self-hosted workflows in `examples/python/self-hosted/`:

| Workflow | Purpose | Agents | Duration |
|----------|---------|--------|----------|
| `add_op.air` | Add new AIS operation | 4 (architect, compiler_dev, runtime_dev, reviewer) | ~3 min |
| `remove_op.air` | Remove deprecated operation | 3 (analyzer, remover, tester) | ~2 min |
| `explore.air` | Research feasibility of feature | 2 (researcher, critic) | ~5 min |
| `plan_feature.air` | Design multi-operation feature | 3 (architect, planner, reviewer) | ~4 min |
| `refactor.air` | Refactor module/subsystem | 3 (analyzer, refactorer, validator) | ~6 min |
| `autofix_workflow.air` | Fix failing tests | 2 (debugger, fixer) | ~3 min |
| `audit.air` | Code quality audit | 1 (auditor) | ~4 min |

All workflows emit session output to `~/.apxm/sessions/<id>/` with full trace, agent workspaces, and artifacts.

---

## Example 1: Add New AIS Operation

### Workflow: `add_op.air`

**Purpose:** Implement a new AIS operation end-to-end (compiler + runtime).

**Usage:**
```bash
dekk apxm execute examples/python/self-hosted/add_op.air \
  --emit-session \
  -- "CHECKPOINT" "Save execution state for later resume"
```

**What Happens:**

1. **Spawn architect** (Claude)
   - Reads `definitions.rs`, `AISOps.td`, `ArtifactEmitter.cpp`, example handler
   - Creates implementation plan (wire index, attributes, MLIR mnemonic, handler logic)
   - Output: Structured plan (~400 words)

2. **Spawn compiler_dev** (Claude) and **runtime_dev** (Codex) in parallel
   - **compiler_dev:** Modifies `definitions.rs`, `AISOps.td`, `ArtifactEmitter.cpp`
   - **runtime_dev:** Creates `handlers/checkpoint.rs`, updates `executor/mod.rs`, writes tests
   - Both agents work concurrently (2x speedup)

3. **Spawn reviewer** (Claude)
   - Checks wire index consistency
   - Verifies attribute names match
   - Runs `dekk apxm build` and `cargo test -p apxm-runtime`
   - Reports pass/fail + fix suggestions

4. **Synthesize results**
   - Final summary: operation name, wire index, status, test results
   - Artifacts: Full implementation ready to commit

**Session Output:**
```
~/.apxm/sessions/add-checkpoint-20260408-143022/
├── manifest.json
├── input.apxm
├── trace.ndjson
├── results.json
├── metrics.json
└── nodes/
    ├── 01_architect/
    │   ├── CLAUDE.md        # Agent context
    │   ├── output.json      # Implementation plan
    │   └── trace.ndjson
    ├── 02_compiler_dev/
    │   ├── CLAUDE.md
    │   └── output.json      # Compiler changes
    ├── 03_runtime_dev/
    │   ├── CLAUDE.md
    │   └── output.json      # Runtime handler
    └── 04_reviewer/
        ├── CLAUDE.md
        └── output.json      # Review + test results
```

**Expected Duration:** ~3 minutes (parallel execution)

**Cost:** ~$0.40 (4 agents × ~$0.10 each, using gpt-4)

---

## Example 2: Explore Feature Feasibility

### Workflow: `explore.air`

**Purpose:** Research whether a proposed feature is feasible and how to implement it.

**Usage:**
```bash
dekk apxm execute examples/python/self-hosted/explore.air \
  --emit-session \
  -- "Should APXM support streaming LLM responses?"
```

**What Happens:**

1. **Spawn researcher** (Claude)
   - Reads relevant codebase sections
   - Reviews existing architecture (backends, runtime, protocol)
   - Researches streaming APIs (OpenAI, Anthropic)
   - Proposes implementation approach

2. **Spawn critic** (Claude)
   - Reviews researcher's proposal
   - Identifies challenges (protocol changes, artifact format, runtime scheduler)
   - Flags risks and unknowns
   - Suggests alternative approaches

3. **Synthesize**
   - Final recommendation: feasible/not feasible
   - Implementation complexity (low/medium/high)
   - Dependencies and blockers
   - Estimated effort

**Session Output:**
```
~/.apxm/sessions/explore-streaming-20260408-150312/
└── nodes/
    ├── 01_researcher/
    │   └── output.json      # Feasibility analysis
    └── 02_critic/
        └── output.json      # Critical review + alternatives
```

**Use Cases:**
- Evaluate new feature ideas
- Assess technical debt refactoring
- Compare implementation approaches
- Research upstream dependencies

---

## Example 3: Automated Code Audit

### Workflow: `audit.air`

**Purpose:** Identify code quality issues, hardcoded strings, anti-patterns.

**Usage:**
```bash
dekk apxm execute examples/python/self-hosted/audit.air \
  --emit-session \
  -- "apxm-server"  # Crate to audit
```

**What Happens:**

1. **Spawn auditor** (Claude)
   - Reads all `.rs` files in target crate
   - Identifies:
     - Hardcoded strings (should use constants)
     - Error handling issues (.unwrap(), panic!)
     - Performance anti-patterns (mutex contention, unnecessary clones)
     - Missing documentation
     - Test coverage gaps
   - Categorizes by severity (CRITICAL/HIGH/MEDIUM/LOW)

2. **Generate report**
   - Findings grouped by category
   - File:line locations
   - Suggested fixes
   - Prioritized action items

**Session Output:**
```
~/.apxm/sessions/audit-server-20260408-152130/
└── nodes/
    └── 01_auditor/
        ├── output.json      # Full audit report
        └── findings.md      # Human-readable summary
```

**Example Findings:**
```markdown
# Audit: apxm-server (2026-04-08)

## CRITICAL
- **DashMap TOCTOU race** (main.rs:1508-1524)
  - Guard dropped before serialization
  - Fix: Scope guard lifetime

## HIGH
- **Hardcoded protocol version** (main.rs:915)
  - "2025-11-05" hardcoded (vs constants::protocols::MCP_VERSION)
  - Fix: Use centralized constant

## MEDIUM
- **Timeout hardcoded** (main.rs:1129)
  - 30_000 literal (should be DEFAULT_HTTP_CAPABILITY_TIMEOUT_MS)
```

**Use Cases:**
- Pre-release quality checks
- Onboarding review (understand codebase issues)
- Technical debt tracking
- Security vulnerability scanning

---

## Example 4: Automated Refactoring

### Workflow: `refactor.air`

**Purpose:** Refactor a module or subsystem with agent assistance.

**Usage:**
```bash
dekk apxm execute examples/python/self-hosted/refactor.air \
  --emit-session \
  -- "apxm-credentials/src/validate.rs" "Extract per-protocol validation functions"
```

**What Happens:**

1. **Spawn analyzer** (Claude)
   - Reads target file
   - Identifies current structure
   - Proposes refactoring plan (extract functions, split modules, etc.)

2. **Spawn refactorer** (Codex)
   - Implements refactoring following plan
   - Preserves behavior (same tests pass)
   - Improves code structure

3. **Spawn validator** (Claude)
   - Runs `cargo test`
   - Checks code style (`cargo fmt`, `cargo clippy`)
   - Verifies no behavior changes (same test output)

**Session Output:**
```
~/.apxm/sessions/refactor-validate-20260408-154500/
└── nodes/
    ├── 01_analyzer/
    │   └── output.json      # Refactoring plan
    ├── 02_refactorer/
    │   └── output.json      # Refactored code
    └── 03_validator/
        └── output.json      # Test results + validation
```

---

## Authoring Your Own Self-Hosted Workflows

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

    # Spawn agents
    planner = g.spawn("planner", profile=claude, cwd=cwd)
    implementer = g.spawn("implementer", profile=codex, cwd=cwd)
    tester = g.spawn("tester", profile=claude, cwd=cwd)

    # Step 1: Plan
    planner.ask("Create implementation plan: {task_description}")
    plan = planner.get_last_node()

    g.print("=== PLAN ===\n{plan}")

    # Step 2: Implement
    implementer.ask("Implement this plan: {plan}")
    impl = implementer.get_last_node()

    g.print("=== IMPLEMENTATION ===\n{impl}")

    # Step 3: Test
    tester.ask("Test this implementation: {impl}")
    test_results = tester.get_last_node()

    g.print("=== TESTS ===\n{test_results}")

    # Final summary
    summary = g.think("Synthesize: Plan={plan}, Impl={impl}, Tests={test_results}")

    g.done(summary)


if __name__ == "__main__":
    print(my_dev_workflow._graph.to_air())
```

### Emit and Run

```bash
# Generate .air file
PYTHONPATH=crates/apxm-frontend/python python3 my_workflow.py > my_workflow.air

# Execute with session trace
dekk apxm execute my_workflow.air --emit-session -- "Add logging to executor"

# Review session
dekk apxm replay ~/.apxm/sessions/<id>
```

---

## Built-in Skills for Agents

Self-hosted workflows can use APXM-specific skills from `.agents/skills/`:

| Skill | Purpose | Usage in Agent Workspace |
|-------|---------|--------------------------|
| `/ops` | List all AIS operations | `dekk apxm ops list` |
| `/compile` | Compile graph | `dekk apxm compile graph.air` |
| `/execute` | Run workflow | `dekk apxm execute graph.air` |
| `/build` | Build APXM | `dekk apxm build` |
| `/test` | Run tests | `dekk apxm test` |
| `/validate` | Validate graph | `dekk apxm validate graph.air` |
| `/analyze` | Analyze graph | `dekk apxm analyze graph.air` |

**Example agent prompt:**
```
You are a compiler developer for APXM. Use the following skills:
- /ops list: View all operations
- /build: Build the project
- /test: Run tests

Your task: Add a new operation CHECKPOINT to the AIS.
```

Agents can invoke these skills via their workspace, and APXM injects the skill definitions into the agent's `CLAUDE.md` context file.

---

## Session Management

### View Past Sessions

```bash
ls ~/.apxm/sessions/
```

**Output:**
```
add-checkpoint-20260408-143022/
explore-streaming-20260408-150312/
audit-server-20260408-152130/
refactor-validate-20260408-154500/
```

### Replay Session Timeline

```bash
dekk apxm replay ~/.apxm/sessions/add-checkpoint-20260408-143022
```

**Output:**
```
Session: add-checkpoint-20260408-143022
Status: completed
Duration: 3m 24s

Timeline:
────────────────────────────────────────
[00:00] START   — execution started
[00:12] SPAWN   — architect (node 1)
[00:45] SPAWN   — compiler_dev (node 2)
[00:46] SPAWN   — runtime_dev (node 3)
[02:13] SPAWN   — reviewer (node 4)
[03:08] PRINT   — final summary
[03:24] DONE    — execution complete

Agents Used:
- Claude (architect, compiler_dev, reviewer): 3 instances
- Codex (runtime_dev): 1 instance

Metrics:
- Total LLM calls: 18
- Total tokens: 142,483
- Cost: $0.38
```

### Extract Node Outputs

```bash
cat ~/.apxm/sessions/<id>/nodes/01_architect/output.json | jq -r '.result'
```

---

## Best Practices

1. **Use `--emit-session`** for all self-hosted workflows (reproducibility)
2. **Parameterize workflows** instead of hardcoding task descriptions
3. **Review agent outputs** before committing (agents make mistakes)
4. **Test generated code** — run `cargo test` manually
5. **Track costs** — self-hosted workflows use LLM credits ($0.30-$0.50 per run)
6. **Version control sessions** — commit session manifests for critical changes
7. **Iterate on workflows** — refine prompts based on output quality

---

## Example: Full Development Loop

```bash
# 1. Explore feasibility
dekk apxm execute explore.air --emit-session -- "Add CHECKPOINT operation"
# Review output → feasible

# 2. Plan implementation
dekk apxm execute plan_feature.air --emit-session -- "CHECKPOINT operation"
# Review plan → looks good

# 3. Implement operation
dekk apxm execute add_op.air --emit-session -- "CHECKPOINT" "Save execution state"
# Review code → minor fixes needed

# 4. Fix issues manually
# (edit files based on reviewer feedback)

# 5. Audit for quality
dekk apxm execute audit.air --emit-session -- "apxm-runtime"
# Review findings → all clear

# 6. Commit
git add -A
git commit -m "feat(ais): add CHECKPOINT operation"
```

---

## Cost and Performance

**Typical workflow costs (using gpt-4):**
- `add_op.air`: $0.40 (4 agents, ~3 min)
- `explore.air`: $0.25 (2 agents, ~5 min)
- `audit.air`: $0.15 (1 agent, ~4 min)
- `refactor.air`: $0.35 (3 agents, ~6 min)

**Cost optimization:**
- Use `gpt-4o-mini` for non-critical agents (~75% cost reduction)
- Use local models (ollama) for privacy/cost ($0)
- Multi-model routing: fast model for formatting, gpt-4 for reasoning

**Example with cost optimization:**
```python
# Use cheap model for triage, expensive for implementation
planner = g.spawn("planner", profile=claude, model="gpt-4o-mini")
implementer = g.spawn("implementer", profile=codex, model="gpt-4")
```

---

## Limitations

1. **Agent hallucination** — agents may propose incorrect implementations
2. **Context limits** — large codebases may exceed agent context windows
3. **No execution guarantees** — generated code may not compile/pass tests
4. **Cost** — multiple agents × gpt-4 adds up ($0.30-$0.50 per workflow)
5. **Requires review** — never blindly commit agent output

**Mitigations:**
- Use reviewer agents to validate output
- Run tests in workflow (automated validation)
- Emit session traces (full audit trail)
- Iterate on prompts to improve quality

---

## Next Steps

- [Multi-Agent Collaboration](multi-agent.md) — Spawning and coordinating agents
- [Optimization](optimization.md) — Reducing workflow latency and cost
- [Examples](../../examples/python/self-hosted/) — Full self-hosted workflow source code
