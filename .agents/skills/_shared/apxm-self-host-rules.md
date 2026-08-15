# Shared rule — APXM self-hosting (APXM builds itself)

Load this file before writing an Agent Program that drives changes to APXM
itself. The toolchain that coordinates agents modifying the toolchain must
follow the project's own dev rules.

**Nothing self-hosted ships in this tree today.** There is no `.apxmw` workflow
format, no `BRANCH_ON_VALUE` node, no goal bundle, no `autofix_workflow.py`, no
`agent_council`, and no `dekk agents execute`. The only execution entry point is
`dekk agents execute-canonical`, which runs one admitted canonical AIR module.
Treat the rules below as constraints on any future self-hosted program, not as a
description of something you can run now.

## The three contracts

1. **No raw build tooling in agent prompts.** Never instruct an agent to
   run `cargo`, `docker`, `srun`, or `sbatch` (banned by
   `_shared/apxm-development-rules.md`). Direct agents through
   `dekk agents <command>`, the same path a human uses.
2. **No hardcoded source paths in prompts.** Paths drift and rot. Resolve
   them at run time, or read them from one shared facts file — never bake
   `crates/.../foo.rs` into a prompt string.
3. **No hardcoded provider profiles.** Do not name `claude`, `codex`, or
   any vendor in a program. Declare roles (`architect`, `dev`, `reviewer`) and
   bind providers at the edge.

Control flow belongs in the program, not in an agent instruction: use the
structural loop and yield/resume semantics the canonical frontend and AIR
contracts expose, rather than delegating "iterate until tests pass" to an
agent's internal loop.

Any git mutation such a program performs still obeys
`_shared/apxm-agent-operating-rules.md`.
