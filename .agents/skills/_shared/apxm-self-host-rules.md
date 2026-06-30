# Shared rule — APXM self-hosting (APXM builds itself)

Load this file before authoring or editing a self-hosted development
workflow (`examples/python/self-hosted/*`, goal bundles, or any graph
that spawns agents to modify APXM itself). The toolchain that coordinates
agents modifying the toolchain must follow the project's own dev rules.

Self-hosted dev tasks are authored as APXM graphs / goal bundles, never
ad-hoc Python. The rule prose is the source of truth; the workflow is the
machine-checking arm — mirror the way `commit-message-rules.md`
pairs with `check_commit_message.py`.

## The four contracts

1. **No raw build tooling in agent prompts.** Never instruct an agent to
   run `cargo`, `docker`, `srun`, or `sbatch` (banned by
   `_shared/apxm-development-rules.md`). Workflows direct agents through
   `dekk agents <command>`, the same path a human uses.
2. **No hardcoded source paths in prompts.** Paths drift and rot. Resolve
   them at run time with an `ais.ask` "locate the enum/handler" node, or
   read them from one shared facts file — never bake
   `crates/.../foo.rs` into a prompt string.
3. **No hardcoded provider profiles.** Do not name `claude`, `codex`, or
   any vendor in a workflow. Declare roles (`architect`, `dev`,
   `reviewer`) and bind providers at the edge — `goal-orchestrator`
   requires that no provider-specific host is assumed.
4. **Control flow lives in the graph, not the agent.** Use
   `BRANCH_ON_VALUE` / fan-in nodes; do not delegate "iterate until
   tests pass" to an agent's internal loop. Note the scheduler fires a
   node once per run — a `g.loop` is single-pass, so design convergence
   as explicit graph structure, not an agent instruction.

## Authoring checklist

- Reuse the shipped skeletons: classify → fan-out → verify → report
  (`autofix_workflow.py`), planner → executor → reviewer → synthesize
  (`agent_council/workflow.apxmw`).
- Dispatch with `dekk agents execute <workflow.py>`; obey
  `_shared/apxm-agent-operating-rules.md` for any git mutation the
  workflow performs.
- Each new dev workflow should be referenced from the skill whose manual
  checklist it replaces, so the skill becomes "run the workflow, then
  verify."
