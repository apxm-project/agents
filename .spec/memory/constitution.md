# APXM Agent Engineering Constitution

Non-negotiable principles for agent-facing work in this repo. Every spec phase
(`spec-specify`, `spec-plan`, `spec-analyze`) is gated against these. A conflict
is resolved by changing the spec/plan/tasks, never by softening a principle.

## Core Principles

### 1. Transport parity
Every delivered agent capability MUST behave identically whether the agent runs
on the interactive CLI host or the HTTP server path (`/v1/execute/stream`). No
delivered capability MUST depend on local-CLI-only execution
(`requires_local_cli` or equivalent). *Rationale: the conversational agent runs
over the server path; a CLI-only feature is not delivered for it.*

### 2. The program owns the cognition; the host is a pipe
All conversational behavior — loop, turn body, context policy, hooks, sub-agents
— MUST be expressible in the agent program. A host MUST NOT be required to add
conversational behavior; its role is limited to delivering input and rendering
output. *Rationale: this is the feature's reason for existing (0001).*

### 3. AIR-portability
An agent's control logic (loop, lifecycle hooks, compaction policy, sub-agent
definitions) MUST travel inside the compiled artifact, not in host config or a
sidecar that the server discards. A deployed `.apxmobj` MUST carry its own
control logic. *Rationale: portability across CLI, server, apxm-os, studio.*

### 4. One Python-handler mechanism
Python-authored tools AND hooks MUST share a single invocation path
(`python_handler_id` + the tool bridge + the artifact sidecar). A second
parallel Python-invocation mechanism MUST NOT be introduced. *Rationale: the
bridge already runs on the server path; reuse is what makes hooks "just work."*

### 5. Control, not just observation
Where the spec calls for control, a pre-execution hook MUST be able to allow,
deny, or modify the guarded action (and inject context for an ask). A purely
observational callback is insufficient for a control requirement.

### 6. Named entry, no positional contract
Host→program input (the user message / turn) MUST bind by a reserved parameter
name, not by argument position. *Rationale: positional binding mis-wires silently
for custom programs.*

### 7. No dead surface
An authored capability MUST be wired end-to-end or not shipped. Decorative or
no-op public APIs MUST be removed rather than left to mislead, and docstrings
MUST match actual behavior.

### 8. Reuse the proven substrate
New runtime behavior MUST build on existing primitives — no-poll park/wake, live
`splice_dag`, the Python tool bridge, the multi-DAG artifact format — rather than
a parallel mechanism, unless the needed primitive is genuinely absent (then add
it once, generally). *Rationale: these primitives are tested and load-bearing.*

### 9. Waits MUST NOT pin compute
A parked or waiting node MUST yield its worker and release its admission/compute
permit; busy-polling that holds a slot for the duration of a wait is prohibited.
*Rationale: unbounded conversations would otherwise exhaust the worker/LLM pool.*

### 10. Green invariant suites
A change MUST keep the runtime, server, and compiler test suites green; adding an
AIR op MUST update the op-count / tablegen parity guard in the same change. These
suites encode park/wake correctness, op invariants, and server security and MUST
NOT be weakened to pass. *Rationale: the suites are the safety net for exactly
the subsystems this work touches.*

## Governance

- **Amendment:** edit this file in place via `spec-constitution`; never fork a
  parallel copy. After amending, reconcile `spec-specify` / `spec-plan` /
  `spec-analyze` with the change.
- **Versioning:** MAJOR = a principle removed or redefined incompatibly; MINOR =
  a new or materially expanded principle; PATCH = wording/clarification.
- **Compliance review:** `spec-plan`'s Constitution Check and `spec-analyze`'s
  Pass D MUST cite these principles by number; an unjustified violation is
  CRITICAL.

**Version**: 1.0.0 | **Ratified**: 2026-06-15 | **Last Amended**: 2026-06-15
