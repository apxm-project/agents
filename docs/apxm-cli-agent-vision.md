# apxm-cli conversational agent — vision: the agent *is* an APXM program

Status: vision draft, multi-agent verified · 2026-06-03
Principle (the brief): the chat agent must not be hardcoded Rust in `chat.rs`. It
should **be an APXM program** — a turn body + composable middleware + dynamic
prompts that do context injection, skill discovery, tool execution, compaction —
with the CLI as a thin host. "The simplicity of APXM: you can literally write a
conversational agent with a loop and simple instructions."

Verified by 5 investigators in isolated worktrees (apxm IR, apxm-os precedent,
authoring ergonomics, middleware, and a working prototype). file:line throughout.

---

## 1. The sharp finding

Today's `apxm chat` is the **worst of both worlds**:

- the conversation **loop is in Rust** (host-resident REPL, `chat.rs:7-9,167` — self-describes as "the host-resident conversational loop; the runtime stays single-shot, one DAG = one turn"), **and**
- the **turn body is a bare single `ais.ask`** (`apxm-ais/src/chat.rs:95`), with *all* the real agent logic — context injection, skill awareness, compaction — **also hardcoded in Rust host code** (`chat.rs:208-284`).

So nothing meaningful is in APXM. The vision is to **move the turn body and its
cross-cutting concerns into APXM** (program + middleware + dynamic prompts). The
outer loop staying in the host is acceptable — apxm-os does exactly that — but
the *cognition* should be the program, and it isn't yet.

apxm-os precedent, precisely: "the agent is APXM IR" (`vision.md:104`) means only
the `on_event`/`answer` flow bodies are IR — and those are **6-line single-`ais.ask`
graphs** (`apxm-libs/.../discord-project-curate/skill.air:1-6`). The loop
(`os-supervisor/src/lib.rs:607,682`), routing, dedup, belief-scope, and **context
assembly** (`os-dispatch/src/lib.rs:315`) are all Rust *before* the graph. So even
the flagship "agent as IR" keeps loop + middleware in Rust. Our vision can put
*more* into APXM than apxm-os does — but we should be honest about where the
boundary is.

---

## 2. What APXM can express today (verified)

| Concern | In APXM today? | Evidence |
|---|---|---|
| **Turn body as a program** (QMEM→ASK→UMEM→FENCE, tools, skill calls) | ✅ YES — validated | prototype `conversational_agent.py` compiles to valid AIR; `examples/python/conversational/chat_agent.py` is the high-water mark |
| **Authoring it simply** | ✅ YES — ~5–15 lines | Python frontend `@compile` + `g.ask(...)` + `Agent(instructions=, tools=)`; template auto-wiring (`{var}` → data edge); `@tool` 1-liner (`apxm-frontend/python/apxm/{proxy,agent}.py`) |
| **Static system prompt** | ✅ YES | `system_prompt=` attr, read at `llm/mod.rs:114` |
| **Tools in the ASK** | ✅ YES (native tool-calling loop, max 10 iters) | `tool_dispatch.rs:381`; `tool_groups=["web"]` |
| **Middleware around every node incl. ASK** | ✅ PRIMITIVE EXISTS (unused for this) | `OperationMiddleware` with `Next` continuation, pre/post/short-circuit, per-op `applies_to`, child-inherited (`executor/middleware.rs:49`) |
| **Tool-level interceptors** | ✅ | `CapabilityInterceptor` Allow/Deny/EditArgs (`capability/interceptor.rs`) |
| **Lifecycle hooks** | ✅ (observer) | `ExecutionHook`, subprocess hooks |
| **Subflow / cross-artifact composition** | ✅ | `FLOW_CALL`, `CALL_SKILL`, `ContextStack` prompt assembly |
| **The conversation LOOP in the graph** | ❌ NO | `LOOP_START/END` are **fire-once markers** — scheduler is strict DAG, no back-edges (`loop_start.rs:7`, `dataflow.rs:42`; `sugar.py:356` documents fire-once) |
| **Read user input from in-graph** | ❌ NO | no `RECV`/`TURN`/`READ_INPUT` op exists in the 44-op set |
| **Suspend/resume across turns** | ⚠️ only HITL-over-HTTP | `PAUSE`/`RESUME` poll apxm-server checkpoints; re-runs whole graph (`pause.rs:38`) |
| **System prompt from a runtime value** | ❌ NO | `resolve_system_prompt` is a static attr lookup (`llm/mod.rs:114`) — can't wire QMEM/AGENTS.md result into it |
| **Skill discovery by description** | ❌ NO | `CALL_SKILL` needs `skill_id` at compile time; no `FIND_SKILL(description)`; embedder exists but indexes only memory facts |

---

## 3. Two models — and the decision

The investigation cleanly splits the vision into two achievable tiers.

### Model A — Thin turn-graph + composable middleware (achievable now, mostly wiring)

Keep "one DAG = one turn"; the host loop stays tiny (read input, thread
`session_id`, stream). But:

- the **turn body becomes a real, editable APXM program** (default ships rich; user-overridable — `apxm chat --air my_agent.air` *already exists*, `chat.rs:141`);
- the three cross-cutting concerns move out of Rust host code into **`OperationMiddleware` layers + a registered tool**, all on existing primitives:
  - **Context injection** → an `OperationMiddleware` filtered to `op==Ask` that loads the AGENTS.md→CLAUDE.md→MEMORY.md hierarchy + AAM beliefs and prepends to the prompt (or fills the system prompt). Reuses `ContextStack`/`ContextAssembler`. Closes GAP-C without a new op.
  - **Skill discovery by description** → a `search_skills` **tool** (INV_TOOL) backed by the existing `fastembed` embedder over skill `name`+`description`; the model calls it like any tool (progressive disclosure / "tool search" pattern). Add `when_to_use`+`tags` to `SkillManifest`.
  - **Compaction** → a post-ASK middleware (or in-graph `CALL_SKILL` to a registered `summarize` skill) instead of the Rust post-hook in `chat.rs:259`.

This delivers the *whole* "reads AGENTS.md + discovers skills by description +
tools + compaction, all as APXM program/middleware" experience. New work is
small/medium and additive: concrete middlewares + a `MiddlewareConfig` knob
(today only `Timeout`/`LoopGuard`, `config/mod.rs:87`) or programmatic
registration; the `search_skills` tool + manifest fields; making the default
turn-graph rich.

### Context management — yes, it *is* middleware (mostly)

"Context management" is not one thing, and conflating it all as middleware is as
wrong as leaving it in the host. Decompose it and each piece lands on the right
layer. The dividing line:

> **Context *management* is middleware. Task *memory* is the program.
> The host must not own the transcript.**

| Concern | What it is | Right layer | Today |
|---|---|---|---|
| **Context injection** | put ambient context *into* the prompt — AGENTS.md, persona, beliefs, the conversation window | **middleware** (pre-ASK) | hardcoded host; none for chat |
| **History / transcript** | accumulate `(user, assistant)` turns across the session | **middleware → session memory** (pre-ASK recall window, post-ASK append) | host renders the string + threads it as a param (`chat.rs:199`) — *misplaced* |
| **Compaction** | summarise old turns when over budget | **middleware** (around-ASK) | host post-hook (`chat.rs:259`) — *misplaced* |
| **Budget / window fit** | keep within the token window; demote/prune low-value frames | **middleware + runtime `ContextStack`** | `TokenBudgetMiddleware` exists (`token_budget.rs:36`); `ContextStack` token-budgets prompt assembly (`context_stack/mod.rs:38`) |
| **Task memory** | facts the agent *deliberately* stores/recalls | **program** (`qmem`/`umem` in the body) | program (session-scoped, `context.rs:457`) |

So the answer to "is context management middleware?" is **yes for the lifecycle
concerns** — injection, history, compaction, budget/window are cross-cutting, run
on *every* turn, and should wrap the ASK automatically so the agent author never
plumbs them. The single piece that stays in the program is **deliberate task
memory**, because choosing what to remember/recall *is* the agent's logic, not
plumbing.

Two consequences that revise the earlier framing:

1. **The transcript is not a host string — it is session memory.** A
   `ConversationContextMiddleware` recalls the recent window pre-ASK and appends
   the turn post-ASK. The host then only passes the *latest message* +
   `session_id`, not a rendered transcript. This shrinks the host further and is
   what makes the loop-in-host vs loop-in-graph choice (Model A/B) orthogonal to
   context handling — either way, context lives in middleware + memory.
2. **`ContextStack` is the substrate, middleware is the policy.** Window fit and
   pruning already have a demand-paged, token-budgeted assembler (`ContextStack`);
   the missing part is a middleware that *drives* it per turn (which frames to
   include, when to compact), rather than the host deciding.

This makes the Model-A middleware stack a small, named set: `ContextInjection`,
`ConversationContext` (history+window), `Compaction`, `TokenBudget` (exists),
`LoopGuard`/`Timeout` (exist) — plus the `search_skills` tool. The agent program
stays a few lines; everything cross-cutting is a composable layer.

### Model B — The whole agent (loop included) in one program (needs runtime work)

Put the loop *itself* in the graph so the agent is one self-contained artifact.
Requires real runtime additions:

1. **`RECV`/`TURN` op** — block until the host delivers a user message as a token *(the keystone; nothing else matters without it)*.
2. **True loop semantics** — back-edges in `ExecutionDag` / a re-enter dispatcher mode (today the scheduler is topological, fire-once).
3. **Suspend/resume across turns** — execution snapshot + replay so the graph pauses for input without re-running.
4. **`COUNT_TOKENS` op** — so compaction can branch in-graph (`BRANCH_ON_VALUE`).
5. **Dataflow `system_prompt`** — wire a value into the system-prompt slot.

This is the literal "write the entire conversational agent, loop and all, as one
APXM program" ideal. It is real engineering (scheduler + runtime), not wiring.

### Recommendation

**Do Model A now.** It captures ~90% of the vision — an editable agent program +
middleware-composed context/skills/compaction — with additive changes and no
scheduler surgery, and it matches the proven apxm-os boundary (host owns the
loop, graph owns the cognition). **Sequence Model B's primitives as a runtime
roadmap**, starting with `RECV` + loop back-edges, *if and when* "the entire loop
in one artifact" becomes a real requirement (e.g. portable agents that run
identically under the CLI, apxm-os, and studio with no host loop).

The honest nuance to decide on: **is the host owning the loop a wart or a
boundary?** apxm-os treats it as a boundary (host = I/O/transport, graph =
cognition) and ships. If you accept that boundary, Model A *is* the vision. If you
want zero host logic (the agent fully portable as one artifact), you need Model B.

---

## 4. The target turn-program (Model A), concretely

```python
# default agent.py — the editable turn body (host loops over it)
from apxm import compile, GraphRecorder, tool

@tool
def search_skills(query: str) -> str:
    """Find skills relevant to the query, by description (embedder-backed)."""
    ...  # backed by /v1/skills + fastembed; returns top-K name+description+id

@compile()
def agent_turn(g: GraphRecorder):
    g.param("conversation", "str")          # host threads the transcript
    history = g.qmem("session summary", tier="stm")
    reply = g.ask(
        prompt="{conversation}\n\nMemory: {history}",
        # system_prompt injected by ContextInjectionMiddleware (AGENTS.md+memory+beliefs)
        tools=["search_skills", "web"],     # discover skills/tools by description
    )
    g.umem("session summary", reply, tier="stm")
    g.done(reply)
# middleware (registered at host startup, not in the graph):
#   ContextInjectionMiddleware(applies_to=Ask)  -> system prompt from AGENTS.md/memory
#   CompactionMiddleware(applies_to=Ask, post)  -> summarize when over budget
```

- **Editable / open to any implementation:** swap the program (`--air`/`--py`); the host doesn't change. This is the "you can literally write the agent" property, realized.
- **Middleware is composable:** add/remove context, skill-search, compaction, doom-loop as layers without touching the body.
- The prototype that validated this shape is at `conversational_agent.py` (this worktree).

---

## 5. Gap list (ranked, mapped to model)

| Gap | Blocks | Fix | Model |
|---|---|---|---|
| Turn body is a bare single-ASK; logic is Rust host code | the whole vision | ship a rich default turn-program + move concerns to middleware | A (now) |
| No `ContextInjection` middleware; system prompt static (GAP-C) | "reads AGENTS.md" | `OperationMiddleware` on ASK injecting AGENTS.md/memory/beliefs | A (now) |
| No description-based skill discovery (GAP-B) | "discovers skills by description" | `search_skills` tool over embedder; add `when_to_use`/`tags` to `SkillManifest` | A (now) |
| Compaction is Rust host-hook | "middleware in APXM" | post-ASK middleware or `CALL_SKILL` summarize | A (now) |
| `MiddlewareConfig` only Timeout/LoopGuard | configurable middleware | add variants / programmatic registration | A (now) |
| No `RECV`/`TURN` op (GAP-A) | loop-in-graph | new op | B |
| Loop is fire-once; no back-edges (GAP-D) | loop-in-graph | scheduler back-edge support | B |
| No suspend/resume across turns | loop-in-graph | execution snapshot + replay | B |
| No `COUNT_TOKENS` op (GAP-E) | in-graph compaction | new op | B |
| Canvas has no loop node | studio-authored agents | add loop node + lowering | B (studio) |

CLI/host UX (separate from the IR question, all host-side): session resume,
readline/history, permission modes, doom-loop detection, `/help` from a registry,
command grouping, kill dead `gui`. These stay in the thin host and are
independently worth doing.

---

## 6. Provenance
5 parallel investigators in isolated worktrees (apxm IR & loop semantics; apxm-os
agent-as-IR precedent; authoring ergonomics; middleware/interceptors; working
prototype), each citing file:line. Prototype `conversational_agent.py` validated
to well-formed AIR. Worktrees: `apxm/.claude/worktrees/agent-vision-{ir,mw,proto}`,
`apxm-os/.../agent-vision`, `apxm-studio/.../agent-vision`.

---

## Skill access — libraries + hierarchy (global-shared + scoped-imported)

The agent must **not** see the whole catalogue. Following Claude Code / Codex,
there are two orthogonal axes: **discovery scope** (which skills are visible) and
**capability scope** (what a skill may do — already exists via `allowed_tools` /
`side_effect_policy`). This section is discovery scope.

### Model
- **Visible set:** `visible(agent) = shared-tier skills ∪ resolve(agent.imports)`.
  Discovery (`search_skills`) **and** `CALL_SKILL` resolve only within it.
- **Tiers** (hierarchy, precedence by specificity — Claude Code P1/P7):
  - **global / shared** — `shared = true` skills (and the builtin/global root);
    ambient to every agent.
  - **project** — skills in the project root; shared within a project.
  - **scoped / imported** — reached only via an explicit `imports` entry.
- **Libraries:** packs are the libraries; ids are namespaced `lib::skill`
  (closes the flat-collision gap). Import a whole lib or a single `lib::skill`.
- **Import in APXM code:** a `skill.toml` / agent program declares
  `imports = ["apxm-app-github", "ops::deploy"]`; `shared = true` opts a skill
  into the global tier.
- **Difference from Claude Code:** non-global skills are **opt-in via import**
  (Claude Code is opt-out everywhere). The *idea* is the same — bundle = library,
  namespaced, precedence/shadowing.
- **Progressive disclosure** (Claude Code P3/P4): only `name`+`description`+
  `when_to_use` of the *visible* set load; the model picks by description; the
  body loads on call; `search_skills` narrows large visible sets (lexical now,
  embedder later).

### Mapped to apxm primitives (gaps closed)
- `allowed_skills` (apxm-os, client-side only) → generalized to `imports` /
  visible-set, to be enforced **server-side** on `/v1/execute` + `CALL_SKILL`.
- `skill_roots` (flat today) → tier labels global / project / local; the builtin
  root is the global seed.
- packs → `lib::skill` namespacing; visible-set propagated **no-widen** through
  `CALL_SKILL` chains, reusing the `side_effect_policy` admission machinery (a
  child can't see what the parent didn't import).

### Built so far (verified)
- `SkillManifest` gains `when_to_use`, `tags`, `imports`, `shared`
  (`apxm-skill`, serde-default, back-compat).
- `apxm_skill::discovery`: `SkillCard` + `VisibleSet` (shared ∪ imports) +
  `rank()` — a scope-aware lexical ranker, unit-tested: scoped skills cannot
  leak into discovery, shared tier is ambient, lib/namespaced/bare imports work,
  relevance ordering holds, irrelevant queries return empty (not everything).

### Remaining (sequenced)
1. `search_skills` capability wrapping `discovery::rank` over the server
   catalogue, scoped to the caller's visible set.
2. Tier the skill roots (global / project / local) + `lib::skill` ids.
3. Server-side visible-set enforcement on `/v1/execute` + `CALL_SKILL`
   (move the allow-list off the os-dispatch client) with no-widen propagation.
4. Frontend `imports` syntax: a program declares its imports → the visible set
   travels on the execution.

---

## Implementation status — branch `feat/conversational-agent-program`

BUILT + VERIFIED (tests green; warm `cargo` in the main checkout):
- **Manifest discovery fields** — `SkillManifest` gains `when_to_use`, `tags`,
  `imports`, `shared` (serde-default, back-compat). `apxm-skill`, 14 tests.
- **Scope-aware discovery ranker** — `apxm_skill::discovery` (`SkillCard`,
  `VisibleSet` = shared ∪ imports, `rank`). 6 unit tests prove scoped skills
  cannot leak, shared tier is ambient, lib/namespaced/bare imports work, ranking
  orders by relevance, irrelevant queries return empty.
- **`search_skills` capability** — `apxm-server` registers a scope-aware,
  description-ranked discovery capability over the live catalogue (group
  `skills`, read-only). Compiles + wired at startup.
- **Context injection (reads AGENTS.md)** — `apxm-cli` `context_assembly` walks
  the AGENTS.md/CLAUDE.md hierarchy (global → git-root → cwd, ancestor-first,
  32 KiB cap, stops at git root). 4 tests. `chat_air` gains an escaped
  `system_prompt`; `apxm chat` assembles context and injects it. `apxm-ais`,
  64 tests incl. escaping.
- **Discovery exposed to the agent** — `chat_air` gains a `skills` tool group;
  `apxm chat` enables it so the agent can call `search_skills`.

So **context injection + skill discovery (scoped) + tool execution** are real and
verified for `apxm chat`.

REMAINING (sequenced, specs above):
- **ConversationContext middleware** — history as session memory (recall window
  pre-ASK, append post-ASK) so the host passes only the latest message.
- **Compaction middleware** — move the host-side compaction post-hook into a
  composable layer.
- **Server-side visible-set enforcement** — pass `imports` on `/v1/execute` +
  `CALL_SKILL`, filter the library view, no-widen propagation (move the allow-list
  off the os-dispatch client).
- **Default rich agent program** — ship the `conversational_agent` program as a
  precompiled default `--air`; tier the skill roots (global/project/local) +
  `lib::skill` namespacing.
- **Cross-repo**: `apxm-studio`'s `ChatAirOptions` literal must add the new
  `system_prompt`/`skills` fields when it picks up this `apxm-ais`.
