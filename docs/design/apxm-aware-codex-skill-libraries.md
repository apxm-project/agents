# APXM-Aware Codex Skill Libraries

This document captures the integration direction for making Codex and other
coding agents APXM-aware. It connects the APXM "LLVM for agents" vision with a
practical skill-library architecture: skills should be discoverable,
convertible to AIR, exposable through `apxm-server`, executable through APXM,
and measurable against ordinary markdown skill usage.

The immediate goal is a plan, not a claim that the whole stack is implemented.
APXM already has important pieces: `.agents/skills` discovery for node
workspaces, generated `AGENTS.md` context for spawned Codex nodes, ACP agent
profiles, session artifacts, a CLI, and REST/MCP server surfaces. The missing
piece is a shared skill-library control plane and an evidence-backed path from
human-readable skills to APXM graphs.

## Why This Fits The Vision

The APXM vision is that agent systems need shared infrastructure: a clean IR,
modular optimization passes, portable artifacts, and open runtime contracts.
Skills are the right pressure test for that vision.

Today, most agent skills are prose packages. A `SKILL.md` file can be useful to a
host agent, but the host still interprets it ad hoc. APXM should make a skill
more like a software object:

- The source package remains readable and installable by normal agents.
- The APXM form lowers the skill into typed AIR.
- The compiler validates and optimizes the skill graph.
- The runtime executes it with explicit capabilities, state, sessions, metrics,
  and replay.
- The server exposes the library so Codex, GUI, MCP, and other clients see the
  same inventory.

That is the LLVM analogy applied to agent skills. Frontends can author or import
skills in different forms, but APXM gives them a shared IR, optimizer, artifact
format, and runtime.

## Current Integration Surfaces

APXM already has enough surface area to make this concrete.

| Surface | Current role | Relevance |
| --- | --- | --- |
| `.agents/skills` | `SkillResolver` scans repo-local skill directories containing `SKILL.md`. | Existing discovery convention for source skills. |
| Generated `AGENTS.md` | `ContextAssembler` renders Codex node context and references available skills. | Codex can be made APXM-aware per node without rewriting Codex. |
| ACP profiles | `apxm-acp` has profile templates including `codex`, permissions, env, and capability servers. | APXM can spawn Codex as a subprocess and provision capabilities. |
| `apxm-server` | HTTP gateway for execution, capabilities, agents, tasks, checkpoints, MCP, and A2A. | Correct home for a shared skill-library API. |
| `apxm-mcp-server` | Stdio MCP server exposes APXM compiler/runtime tools. | External agents can call APXM tools without shelling out. |
| Session artifacts | Execute/run paths emit session directories, metrics, traces, and node outputs. | Required evidence layer for evaluation and debugging. |
| Quality/eval tools | `quality_eval`, `ablation`, `trace_diff`, and `benchmark_e2e.py`. | Existing measurement machinery should be reused. |

The important architectural decision: skill libraries should be served by
`apxm-server`, not by the GUI. The GUI can display and manage skills, but the
server should own discovery, metadata, compilation status, capability checks,
and eventually dynamic loading. Codex and MCP clients should consume that same
server surface.

## Skill Package Model

APXM should distinguish source skills from APXM skills.

### Source Skill

A source skill is compatible with existing coding-agent ecosystems:

```text
skill-name/
  SKILL.md
  scripts/
  references/
  assets/
```

`SKILL.md` is the human contract: what the skill does, when to use it, required
steps, resources, and verification behavior. Scripts and references are support
files. Source skills are still useful as plain Codex skills.

### APXM Skill

An APXM skill is compiled or compilable:

```text
skill-name/
  skill.air
  skill.toml
  skill.apxmobj        # optional, for precompiled/static use
  resources/
  tests/
  conversion-report.json
```

`skill.toml` is the ABI manifest. A minimal manifest should include:

- `name`, `version`, `description`
- source package path and source hash
- AIR hash and optional artifact hash
- compiler version, optimization level, and target
- expected inputs and outputs
- required capabilities and allowed tools
- side-effect policy and approval policy
- loading mode: `static`, `dynamic`, or `hybrid`
- compatibility with APXM runtime/compiler versions

The manifest lets the server answer "what is this skill?" without loading the
whole graph.

## Server-Owned Library API

`apxm-server` should expose skill libraries as a read-first control plane. The
first version should not execute arbitrary dynamic skills. It should make
inventory, metadata, and validation status visible.

Recommended initial REST surface:

| Endpoint | Purpose |
| --- | --- |
| `GET /v1/skills` | List discovered source and APXM skills. |
| `GET /v1/skills/{name}` | Return manifest, source metadata, hashes, capability requirements, and status. |
| `POST /v1/skills/{name}/validate` | Validate `SKILL.md`, `skill.toml`, and `skill.air` if present. |
| `POST /v1/skills/{name}/compile` | Compile `skill.air` to `.apxmobj`, with explicit opt level and target. |
| `GET /v1/skills/{name}/artifact` | Return artifact metadata or download handle when available. |

Recommended MCP tools after the REST model exists:

| MCP tool | Purpose |
| --- | --- |
| `apxm_skills_list` | Discover available APXM/source skills. |
| `apxm_skill_get` | Fetch one skill's manifest and status. |
| `apxm_skill_validate` | Validate a source or APXM skill. |
| `apxm_skill_compile` | Compile a validated APXM skill. |
| `apxm_skill_explain` | Summarize graph shape, capabilities, risks, and execution contract. |

The server should use the same library implementation that the GUI and CLI use.
Avoid building separate skill scanners in each tool.

## Loading Modes

### Static Loading

Static loading compiles at install, CI, or release time:

```text
source skill -> skill.air -> skill.apxmobj -> registry entry
```

Use this for demos, CI, benchmark runs, and production deployments where the
artifact must be reproducible.

### Dynamic Loading

Dynamic loading resolves and prepares a skill at request time:

```text
request -> discover skill -> validate/convert -> compile/cache -> execute
```

This is the right long-term model for marketplaces and user-installed skill
libraries, but it must fail closed on missing capabilities, invalid AIR,
unsafe scripts, disallowed tools, or ambiguous conversion.

### Hybrid Loading

Hybrid loading serves metadata statically and compiles on first use. This is
likely the default for developer workflows: discovery is cheap, validation is
visible, and warm-cache runs behave like static loading.

## Skill Conversion Pipeline

APXM should have a `skill-to-air` frontend. It can be assisted by an agent, but
the load-bearing output must be structured and auditable.

Recommended pipeline:

1. Parse `SKILL.md` frontmatter and markdown with structured parsers.
2. Normalize into a `SkillSpec`: name, triggers, anti-triggers, prerequisites,
   ordered steps, decision points, resources, scripts, expected outputs, and
   verification steps.
3. Map the spec to AIS:
   instructions become reasoning nodes, scripts become registered
   capabilities, references become explicit resource reads, checks become
   verification/tool nodes, branches become typed control flow, and joins
   become explicit synchronization.
4. Emit `skill.air` and `skill.toml`.
5. Run validation, O0 compile, O2 compile, decompile, and round-trip checks.
6. Write `conversion-report.json` with unmapped sections, inferred decisions,
   assumptions, missing capabilities, and risk flags.

The converter must not silently turn vague prose into production AIR. Any
LLM-inferred graph structure is provisional until validation and tests approve
it.

## Codex Integration Model

Codex does not need to be rewritten to become APXM-aware. The first integration
layer should be additive:

1. Repo-root instructions teach Codex APXM conventions: prefer `dekk apxm ...`,
   ignore `external/vllm` unless asked, use sessions and metrics for debugging,
   and prefer compile-only checks when no backend is configured.
2. Repo-local skills teach APXM workflows: build, test, debug, benchmark,
   compiler pass work, server/MCP work, and quality evaluation.
3. APXM-generated node workspaces provide task-specific `AGENTS.md` and copied
   skills for spawned Codex nodes.
4. APXM MCP tools let Codex validate AIR, inspect the AIS contract, compile
   graphs, and eventually query the skill library.
5. APXM runtime executes compiled skill graphs when the task needs orchestration,
   evidence, replay, or optimization.

This keeps Codex as the shell and APXM as the execution substrate. That matches
the vision's "wrap, do not rewrite" path.

## Initial APXM Skill Pack

The first repo-local APXM-aware skill pack should focus on workflows that
currently require local project knowledge:

| Skill | Purpose |
| --- | --- |
| `apxm-orient` | Read the right docs and crate boundaries before changing code. |
| `apxm-author-python` | Author Python frontend workflows and inspect emitted AIR. |
| `apxm-compiler-pass` | Develop or debug AIS/MLIR compiler passes. |
| `apxm-runtime-backends` | Work on runtime scheduling, backend routing, and vLLM hints. |
| `apxm-server-mcp` | Modify REST/MCP/A2A server surfaces coherently. |
| `apxm-quality-eval` | Run quality fixtures, budgets, judges, and claim checks. |
| `apxm-demos-benchmarks` | Work on Gemma demos, benchmark harnesses, and reports. |
| `apxm-design-docs` | Update conceptual docs without overclaiming implementation state. |

The first batch should be `apxm-orient`, `apxm-server-mcp`,
`apxm-quality-eval`, `apxm-compiler-pass`, and `apxm-demos-benchmarks`.

## Evaluation Matrix

The evaluation should compare ordinary skill use against APXM representations
and APXM execution. Use the same prompt, model/backend, workspace seed, tool
policy, budget, and timeout across arms.

| Arm | Name | What runs | Question |
| --- | --- | --- | --- |
| A0 | No skill | Baseline coding agent receives only the task. | Does any skill help? |
| A1 | Markdown skill | Host agent follows `SKILL.md`. | How good is the existing skill ecosystem? |
| A2 | AIR-as-skill | Host agent receives readable `skill.air`. | Is AIR useful as a portable skill representation? |
| A3 | APXM O0 | APXM executes compiled `skill.air` without optimization. | Does executable APXM preserve correctness? |
| A4 | APXM O2 | APXM executes optimized `skill.air`. | Do compiler/runtime optimizations improve cost, speed, or auditability? |

Interpret deltas carefully:

- `A1 - A0`: skill value.
- `A2 - A1`: readability/portability of AIR as a skill artifact.
- `A3 - A1`: executable parity.
- `A4 - A3`: compiler/runtime value.
- `A4 - A0`: end-to-end APXM-aware skill value.

Reuse the existing Gemma skill evaluation plan and benchmark harness rather
than creating a parallel metrics stack.

## Metrics And Evidence

Correctness and workflow:

- `task_success`
- tests passed/failed and test runtime
- expected files/artifacts produced
- regressions introduced
- clarification expected/asked
- constraint violations
- unsupported claims

Skill behavior:

- `skill_should_activate`
- `skill_did_activate`
- trigger precision/recall
- step coverage
- missed verification steps
- step order violations
- over-application on negative-trigger tasks

Runtime and optimization:

- wall time, graph duration, compile time, load time
- LLM call count and token usage when provider-reported
- nodes executed, failed, skipped, retried
- observed critical path and critical milestone finish
- queue wait totals and p95
- fired passes, eliminated ops, eliminated calls, tokens saved
- backend cache/pin metrics when applicable

Auditability:

- `manifest.json`, `results.json`, `metrics.json`, `trace.ndjson`
- input AIR, artifact path, compiler diagnostics
- per-node prompt/output/metrics evidence
- replay and decompile success
- source skill hash, AIR hash, artifact hash, fixture hash
- claim evidence coverage

Token and cost claims for spawned Codex or Claude ACP subprocesses must not be
inferred from wall time. Report token/cost only when the ACP adapter or provider
reports usage into APXM token accounting.

## Go/No-Go Gates

Minimum go gates before claiming that APXM-aware skills improve coding-agent
work:

- A3 task success is at least A1 for executable-parity fixtures.
- A4 task success is at least A3; no O2 correctness regression.
- Deterministic fixtures pass at least 4 out of 5 repeated runs, or
  nondeterministic fixtures have documented flake analysis.
- Quality fixtures pass with budget caps.
- Speed claims have repeated rows, meaningful absolute delta, meaningful
  relative delta, and confidence that speedup is greater than 1.0.
- Token/call claims show unchanged success and actual reported token/call
  reduction.
- Runtime scheduling claims use critical-path or milestone metrics under queue
  pressure, not unrelated whole-graph wall time.
- Dynamic loading works cold and warm, with cache hit/miss and load overhead
  measured.
- Every headline claim traces to `results.csv`, session metrics, compiler
  diagnostics, or trace artifacts.

No-go conditions:

- O2 lowers task success relative to O0.
- Host-agent markdown skills beat APXM execution on correctness for fixtures
  meant to test executable parity.
- Cost/token numbers are inferred rather than provider-reported.
- Negative-trigger fixtures show systematic over-activation.
- `trace_diff` shows semantic divergence where O0/O2 should be equivalent.
- Reports contain claims that cannot be traced to artifacts.

## Implementation Phases

### Phase 0: Document And Align

- Land this design note.
- Link it from the docs index.
- Keep server and dynamic loading work out of this phase.

### Phase 1: Read-Only Skill Library

- Add a shared skill-library scanner/model, preferably outside GUI-specific
  code.
- Discover source skills from `.agents/skills` and configured roots.
- Read `SKILL.md`, `skill.toml`, `skill.air`, and artifact metadata when
  present.
- Expose `GET /v1/skills` and `GET /v1/skills/{name}` from `apxm-server`.
- Update GUI to consume the server-owned endpoint instead of owning discovery.

### Phase 2: Validation And Compilation

- Add validation for manifests, AIR, hashes, and capability requirements.
- Add `POST /v1/skills/{name}/validate`.
- Add explicit `POST /v1/skills/{name}/compile` with opt level and target.
- Cache artifacts by content hash and compiler/runtime version.
- Add MCP tools for list/get/validate/compile/explain.

### Phase 3: Conversion

- Implement `skill-to-air` as a report-first converter.
- Emit skeleton AIR only when required structure is identifiable.
- Require conversion reports for any inferred graph structure.
- Add fixtures for source skills, generated AIR, and round-trip validation.

### Phase 4: Execution And Dynamic Loading

- Execute statically compiled APXM skills first.
- Add hybrid loading after compile/cache behavior is stable.
- Add dynamic loading only with explicit capability policy and fail-closed
  safety checks.
- Record loader metrics in sessions.

### Phase 5: Evaluation And Claims

- Create `examples/python/skill-evals/` around the A0-A4 matrix.
- Wrap `benchmark_e2e.py` instead of duplicating runtime measurement.
- Add claim linting so reports cannot cite unsupported numbers.
- Promote claims into demos/decks only after artifact-backed runs exist.

## Non-Goals For The First Cut

- Do not rewrite Codex.
- Do not make dynamic execution the first milestone.
- Do not treat generated AIR as correct because it compiles.
- Do not duplicate large docs into every skill package.
- Do not make the GUI the source of truth for skill discovery.
- Do not infer ACP subprocess costs or tokens from elapsed time.
- Do not scan or import `external/vllm` as an ordinary APXM skill source.

## Open Questions

- What is the stable APXM skill ABI beyond `inputs`, `outputs`,
  `capabilities`, `resources`, `side_effects`, and `approval_policy`?
- Should skill-library state live in `apxm-driver`, `apxm-runtime`, a new
  orchestration crate, or `apxm-server` first and then be factored out?
- How should APXM represent intentionally judgment-heavy steps: PLAN nodes,
  autonomous zones, or host-agent callbacks behind typed boundaries?
- Are scripts inside source skills runtime capabilities, conversion-time
  helpers, or both?
- What safety policy applies to dynamically loaded skills with shell scripts?
- How should remote/marketplace skill trust be represented in manifests?

## Immediate Next Steps

1. Add a root APXM-aware `AGENTS.md` or repo-local skill pack so Codex starts
   with the project conventions.
2. Add the read-only server skill-library model and endpoints.
3. Reconnect the GUI skill view to the server-owned API.
4. Create one hand-authored APXM skill package with `SKILL.md`, `skill.toml`,
   `skill.air`, and tests.
5. Build the first A0-A4 fixture around that skill and reuse the existing
   benchmark/session tooling.

