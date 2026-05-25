# Vision — A library system for agent skills

> Rather than positioning APXM strictly as a program execution model, which felt too abstract
> for some, we now present it as a practical solution to the "fragmented skills" problem we see
> in large-scale agent deployments. Every team encodes the same processes as different "skills,"
> leading to constant rewriting. Software scaled when code became libraries; agents will scale
> only when **skills become libraries: compiled, versioned, linkable, and governed**.
>
> APXM provides the typed isolation, reproducible sessions, and compiler diagnostics necessary
> for a production-grade library of skills. This allows agents to load optimized versions of
> skills statically or dynamically into their contexts, backed by node-level metrics and
> checkpointing.

## The fragmented-skills problem

In every team running agents in production, the same scene plays out: the platform team writes
a "review skill," the QA team writes a *different* "review skill," the on-call team writes
*yet another* "review skill," and so on. Each one is a slightly different prompt-and-tool
recipe, copy-pasted between repos, drifting out of sync, optimized in isolation if at all.

There is no shared compiled object. No versioning. No deduping. No single optimizer. No
shared governance over what each skill is allowed to read, call, or write. Every agent host
ends up with its own private dialect of skills — and rewrites them again the moment it hands
work to another agent.

This is the same problem the software industry hit before libraries became a first-class
artifact. Code stopped being a per-project pile of patterns the day `qsort` could be linked
in by name. Skills will follow the same path the day they have a software object you can ship.

## The library thesis

Software scaled when code became libraries: compiled, versioned, linkable, governed objects
with stable APIs and a tooling ecosystem around them. Agents will scale only when skills
become libraries with the same properties:

- **Compiled** — a skill has a build step that produces a deterministic artifact, not just
  a markdown prompt or a JSON config.
- **Versioned** — a skill has a name, a semantic version, and a changelog. Two callers can
  pin different versions during a migration.
- **Linkable** — a skill exposes a typed interface. Other skills (and other agents) call
  into it without copying its internals.
- **Governed** — a skill carries its own capability manifest, side-effect policy, and
  approval gates. The runtime enforces them; the caller cannot weaken them.

This is the unit APXM is designed to make real.

## What APXM provides

APXM is the infrastructure layer that turns this into a concrete production system, not a
slogan. Five commitments back the library thesis:

1. **Typed isolation.** Every skill declares its capabilities, side-effect policy, and
   approval gates in a manifest. The runtime enforces them in a sandbox: a tool the manifest
   does not authorize cannot be called, full stop. Recent hardening work
   ([`af71601`](https://github.com/apxm-project/apxm/commit/af71601),
   [`e0eb949`](https://github.com/apxm-project/apxm/commit/e0eb949),
   [`db198d4`](https://github.com/apxm-project/apxm/commit/db198d4)) tightens the boundary
   between agent-triggered execution and declared policy.
2. **Reproducible sessions.** Every execution emits a session directory with metrics,
   traces, node outputs, and pass diagnostics that you can replay. Persistent skill
   execution snapshots make this auditable and resumable, not just observable.
3. **Compiler diagnostics.** APXM is built on a typed IR (AIR/AIS) lowered through an
   MLIR-based pass pipeline. The optimizer eliminates redundant LLM calls, prunes dead
   context, stamps backend hints, and surfaces the *why* of each transformation. New
   passes compound across every existing skill — that is the entire point of having an IR.
4. **Node-level metrics.** Every node in a compiled skill carries its own duration, token,
   cache, and parallelism counters. The runtime reports them with the session, so claims
   about a skill ("this version cuts LLM calls by N%") can be reproduced from artifacts.
5. **Server-owned library.** `apxm-server` exposes a single skill inventory through a
   REST/MCP surface. Codex, Claude Code, the GUI, and `apxm-cli` all consume the same
   library through one API, so the optimization made for one agent is the optimization
   inherited by every other agent.

Together, these are what make a "library of skills" different from a folder of `.md` files.

## Three concrete proof points

The cleanest place to see this end-to-end today is
[`examples/python/demos/gemma4/`](examples/python/demos/gemma4/), three runnable workflows
that target a real vLLM-served Gemma model.

### Proof 1 — Same compiled skill, multiple agents

[`workflows/01_review_synthesis_skill.py`](examples/python/demos/gemma4/workflows/01_review_synthesis_skill.py)
defines a `ReviewSynthesis` skill: a 6-way fanout of aspect reviewers (security, performance,
reliability, compliance, UX, cost) sharing a pinned KV-cache prefix on vLLM, plus two ACP
sub-agents (Claude as architect, Codex as implementation reviewer) running in parallel,
with Gemma synthesizing the final answer.

The point: the skill is **packaged once**. Different host agents (Claude Code, Codex, the
GUI) all consume the same compiled `.apxmobj` through the server's skill API. They do not
re-implement the fanout, the prefix-caching choice, or the synthesis contract.

### Proof 2 — Compiler optimization is part of the skill, not the caller

[`workflows/02_checkout_context_pruning.py`](examples/python/demos/gemma4/workflows/02_checkout_context_pruning.py)
extracts five separate context branches, but the final decision prompt only references one of
them. The other four data edges are stale.

- At **O0**, every branch runs (5 LLM calls upstream + 1 decision = 6 calls).
- At **O2**, `DeadOperandElimination` and the canonical cleanup pass drop the unused
  upstream LLM nodes, and the skill executes 2 LLM calls instead of 6.

The point: the optimization is **compiled into the skill**. Every caller of this skill
inherits the call/token reduction without changing their code. New compiler passes added
to APXM compound across every existing skill.

### Proof 3 — Typed runtime hints survive lowering, reach the real backend

[`workflows/03_vllm_backend_hints.py`](examples/python/demos/gemma4/workflows/03_vllm_backend_hints.py)
deliberately creates vLLM queue pressure: eight long background audit prompts run first,
then a short user-visible critical chain becomes ready while the backend is busy. The
compiler stamps the critical chain with APXM priority hints. The runtime lowers those hints
to vLLM `apxm_hints` so vLLM's priority scheduler can preempt the background queue.

The point: a skill carries its **backend-aware execution intent** as typed metadata, and
that intent reaches the actual deployment target. One packaging, multiple deployments.

## Where this leads

The skill-library framing is the front door. The engine room behind it is a formal Program
Execution Model for agentic AI ("A-PXM"). The same way the LLVM IR makes a thousand
language frontends and a hundred backends interoperate around one optimization pipeline,
A-PXM is built so a thousand agent skills and many backends can interoperate around one
typed graph IR.

For the long-range theoretical direction, see
[docs/pxm/vision.md](docs/pxm/vision.md) ("LLVM for Agents") and the rest of the
[`docs/pxm/`](docs/pxm/) tree:

- [Overview](docs/pxm/readme.md)
- [Foundations](docs/pxm/foundations.md)
- [Agent Abstract Machine (AAM)](docs/pxm/aam.md)
- [Agent Instruction Set (AIS)](docs/pxm/ais.md)
- [Compute](docs/pxm/compute.md), [Memory](docs/pxm/memory.md),
  [Scheduling](docs/pxm/scheduling.md)

And for the concrete integration roadmap that turns "skill library" from a thesis into a
delivered surface, see
[docs/design/apxm-aware-codex-skill-libraries.md](docs/design/apxm-aware-codex-skill-libraries.md)
and
[docs/design/apxm-skill-runtime-task-backlog.md](docs/design/apxm-skill-runtime-task-backlog.md).

The library thesis is the front door. The PXM substrate is what makes the front door possible.
