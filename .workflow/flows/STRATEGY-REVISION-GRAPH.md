# Strategy Revision Graph

**10-Agent Parallel Document Revision Pipeline with Ultrathink Sub-Agents**

Two acpx flows work together: an **orchestrator** coordinates 5 dependency-aware phases, spawning **sub-flow** instances that each revise one document in an isolated git worktree. Every acp node deploys a **team of 5 ultrathink agents** — mental sub-agents each focused on one dimension of the task — to maximize thoroughness.

---

## Document Dependency Graph

```
01-VISION ────────────┐
                      │
02-OPPORTUNITIES ─────┤──── 06-GAP-ANALYSIS
                      │
04-ARCHITECTURE ──────┤──── 07-ACPX-ABSORPTION ───┐
                      │                            ├── 03-PLAN
05-PATENT ────────────┘──── 08-OPTIMIZATION ───────┘       │
                                                           v
Phase 1 (4 parallel)    Phase 2 (3 parallel)    Phase 3   README
                                                          Phase 4
```

03-PLAN explicitly references 07-ACPX-ABSORPTION and 08-OPTIMIZATION-TARGETS, so it **must** run after Phase 2. README depends on all 8 docs.

---

## Orchestrator Flow

**File:** `strategy-revision-orchestrator.flow.ts`
**Nodes:** 19 | **Edges:** 20 | **Retry loops:** 1

### Phases

| Phase | Docs | Agents | Profile | Parallelism |
|-------|------|--------|---------|-------------|
| 0 — Analysis | all 9 (read-only) | Agent 1 | Claude Code | 1 (coordinator) |
| 1 — Independent | 01, 02, 04, 05 | Agents 2-5 | Codex | 4 parallel |
| 2 — First-dependent | 06, 07, 08 | Agents 6-8 | Claude Code | 3 parallel |
| 3 — Plan | 03 | Agent 9 | Claude Code | 1 sequential |
| 4 — Final | README | Agent 10 | Claude Code | 1 sequential |

### Graph

```
┌─────────────┐    ┌───────────┐    ┌─────────────┐    ┌────────────┐
│ load_config │───>│ scan_docs │───>│ analyze_all │───>│ plan_tasks │
│  (compute)  │    │  (shell)  │    │ (acp/claude)│    │  (compute) │
│             │    │  ls *.md  │    │  Agent 1    │    │ build plan │
└─────────────┘    └───────────┘    │ 15min       │    └─────┬──────┘
                                    └─────────────┘          │
                                                             v
                                                      ┌──────────────┐
                                                      │ approve_plan │
                                                      │ (checkpoint) │
                                                      │ HUMAN REVIEW │
                                                      └──────┬───────┘
═══════════════════════════════════════════════════════════════╪═══════════
 PHASE 1: 4x PARALLEL CODEX                                  v
 ┌──────────────────────────────────┐  ┌──────────────┐ ┌─────────────┐
 │ doc-01: 01-VISION.md     (Ag.2) │  │ spawn_phase1 │>│ phase1_gate │
 │ doc-02: 02-OPPORTUN.md  (Ag.3) │  │   (shell)    │ │  (compute)  │
 │ doc-04: 04-ARCHITECT.md (Ag.4) │  │  bash & wait │ │  status chk │
 │ doc-05: 05-PATENT.md    (Ag.5) │  │  30min       │ └──────┬──────┘
 └──────────────────────────────────┘  └──────────────┘       │
═══════════════════════════════════════════════════════════════╪═══════════
 PHASE 2: 3x PARALLEL CLAUDE                                 v
 ┌──────────────────────────────────┐  ┌──────────────┐ ┌─────────────┐
 │ doc-06: 06-GAP-ANAL.md  (Ag.6) │  │ spawn_phase2 │>│ phase2_gate │
 │ doc-07: 07-ACPX-ABS.md  (Ag.7) │  │   (shell)    │ │  (compute)  │
 │ doc-08: 08-OPTIM.md     (Ag.8) │  │  bash & wait │ │  status chk │
 └──────────────────────────────────┘  │  30min       │ └──────┬──────┘
                                       └──────────────┘        │
═══════════════════════════════════════════════════════════════╪═══════════
 PHASE 3: 1x CLAUDE (depends on 07+08)                       v
 ┌──────────────────────────────────┐  ┌──────────────┐ ┌─────────────┐
 │ doc-03: 03-PLAN.md       (Ag.9) │  │ spawn_phase3 │>│ phase3_gate │
 └──────────────────────────────────┘  │   (shell)    │ │  (compute)  │
                                       │  15min       │ └──────┬──────┘
                                       └──────────────┘        │
═══════════════════════════════════════════════════════════════╪═══════════
 PHASE 4: README + FINAL VALIDATION                           v
 ┌──────────────────────────────────┐  ┌──────────────┐ ┌──────────────────┐
 │ doc-readme: README.md   (Ag.10) │  │ spawn_readme │>│final_validation  │
 └──────────────────────────────────┘  │   (shell)    │ │  (acp/claude)    │
                                       │  15min       │ │  Agent 1, 15min  │
                                       └──────────────┘ └────────┬─────────┘
                                                                 v
                                                          ┌─────────────┐
                                                 ┌────────│ judge_final │
                                                 │        │  (compute)  │
                                                 │        └──────┬──────┘
                                                 │  retry        │ done
                                                 │  (conf < 0.8  v
                                                 │  or issues>2)
                                                 │        ┌─────────────────┐
                                                 └───────>│ generate_report │
                                                  LOOP    │   (compute)     │
                                                  back    │   DONE          │
                                                          └─────────────────┘
```

### Edge List

| # | From | To | Type |
|---|------|----|------|
| 1 | load_config | scan_docs | linear |
| 2 | scan_docs | analyze_all | linear |
| 3 | analyze_all | plan_tasks | linear |
| 4 | plan_tasks | approve_plan | linear |
| 5 | approve_plan | spawn_phase1 | linear |
| 6 | spawn_phase1 | phase1_gate | linear |
| 7 | phase1_gate | spawn_phase2 | linear |
| 8 | spawn_phase2 | phase2_gate | linear |
| 9 | phase2_gate | spawn_phase3 | linear |
| 10 | spawn_phase3 | phase3_gate | linear |
| 11 | phase3_gate | spawn_readme | linear |
| 12 | spawn_readme | final_validation | linear |
| 13 | final_validation | judge_final | linear |
| 14 | judge_final | generate_report | switch: `done` |
| 15 | judge_final | final_validation | switch: `retry` (LOOP) |

### Node Inventory

| Node | Type | Agent | Timeout | Purpose |
|------|------|-------|---------|---------|
| load_config | compute | — | — | Parse input, set defaults |
| scan_docs | shell | — | — | Inventory docs, extract cross-refs |
| analyze_all | acp (claude) | Agent 1 | 15min | Deep cross-doc analysis, build glossary |
| plan_tasks | compute | — | — | Build phase plan from analysis |
| approve_plan | checkpoint | — | — | Human reviews plan before execution |
| spawn_phase1 | shell | — | 30min | Launch 4 parallel sub-flows (Codex) |
| phase1_gate | compute | — | — | Check Phase 1 completion status |
| spawn_phase2 | shell | — | 30min | Launch 3 parallel sub-flows (Claude) |
| phase2_gate | compute | — | — | Check Phase 2 completion status |
| spawn_phase3 | shell | — | 15min | Launch 1 sub-flow for 03-PLAN |
| phase3_gate | compute | — | — | Check Phase 3 completion status |
| spawn_readme | shell | — | 15min | Launch 1 sub-flow for README |
| final_validation | acp (claude) | Agent 1 | 15min | Cross-doc consistency check, fix issues |
| judge_final | compute | — | — | Route: done (conf >= 0.8) or retry |
| generate_report | compute | — | — | Compile final status report |

---

## Sub-Flow Pipeline (per document)

**File:** `strategy-revision-doc.flow.ts`
**Nodes:** 10 | **Edges:** 9 | **Retry loops:** 2

Each document revision runs this pipeline in its own process and git worktree.

### Graph

```
┌────────────┐     ┌─────────────────┐     ┌────────────┐     ┌──────────────┐
│ load_input │────>│ create_worktree │────>│ revise_doc │────>│ validate_doc │
│ (compute)  │     │    (shell)      │     │   (acp)    │     │ (acp/claude) │
│ parse JSON │     │ git worktree add│  ┌─>│ 10min      │     │ 5min         │
└────────────┘     └─────────────────┘  │  └────────────┘     └──────┬───────┘
                                        │                            │
                                        │                            v
                       LOOP 1           │                     ┌─────────────┐
                       retry (max 2)    │                     │ judge_valid │
                       validation fail  └─────────────────────│  (compute)  │
                                            route: "retry"    └──────┬──────┘
                                                                     │
                                                        route: "pass"│
                                                                     v
┌──────────────────┐     ┌──────────────────┐              ┌──────────────┐
│ regression_check │<────│  simplify_doc    │<─────────────│              │
│    (shell)       │     │  (acp/claude)    │              └──────────────┘
│ grep broken refs │     │  /simplify skill │
└────────┬─────────┘     │  5min            │
         │               └─────────────────┬┘
         v                                 ^
┌─────────────────┐                        │
│ judge_regression│                        │  LOOP 2
│   (compute)     │────────────────────────┘  fix (max 1)
└────────┬────────┘  route: "fix"             broken links → back to revise_doc
         │
         │ route: "pass"
         v
┌──────────────┐     ┌───────────────┐
│ collect_diff │────>│ merge_changes │
│   (shell)    │     │    (shell)    │
│ git diff stat│     │ commit+merge  │
└──────────────┘     │ rm worktree   │
                     └───────────────┘
```

### Edge List

| # | From | To | Type |
|---|------|----|------|
| 1 | load_input | create_worktree | linear |
| 2 | create_worktree | revise_doc | linear |
| 3 | revise_doc | validate_doc | linear |
| 4 | validate_doc | judge_valid | linear |
| 5 | judge_valid | simplify_doc | switch: `pass` |
| 6 | judge_valid | revise_doc | switch: `retry` (LOOP 1) |
| 7 | simplify_doc | regression_check | linear |
| 8 | regression_check | judge_regression | linear |
| 9 | judge_regression | collect_diff | switch: `pass` |
| 10 | judge_regression | revise_doc | switch: `fix` (LOOP 2) |
| 11 | collect_diff | merge_changes | linear |

### Node Inventory

| Node | Type | Agent | Timeout | Purpose |
|------|------|-------|---------|---------|
| load_input | compute | — | — | Parse input JSON, init retry counter |
| create_worktree | shell | — | — | `git worktree add` with PID-scoped path |
| revise_doc | acp (profile) | varies | 10min | Edit doc based on analysis + retry context |
| validate_doc | acp (claude) | Claude Code | 5min | Cross-doc consistency validation |
| judge_valid | compute | — | — | Route: pass or retry (max 2 retries) |
| simplify_doc | acp (claude) | Claude Code | 5min | Run `/simplify` skill on revised doc |
| regression_check | shell | — | — | Grep for broken links in all docs |
| judge_regression | compute | — | — | Route: pass or fix broken links |
| collect_diff | shell | — | — | `git diff --cached --stat` |
| merge_changes | shell | — | — | Commit, merge branch, remove worktree |

### Retry Behavior

**Loop 1 — Validation retry (max 2):**
If `validate_doc` finds issues, `judge_valid` routes back to `revise_doc` with the validation issues injected into the prompt. The agent sees the specific failures and fixes only those. After 2 retries, execution proceeds regardless — the orchestrator's final validation catches remaining issues.

**Loop 2 — Regression fix (max 1):**
If `regression_check` finds broken links (e.g., a heading was renamed), `judge_regression` routes back to `revise_doc` to fix them. This loops through the full revise-validate-simplify pipeline again.

---

## Agent Assignments

| Agent | Document | Revise Profile | Validate | Simplify | Phase |
|-------|----------|----------------|----------|----------|-------|
| 1 | Coordinator (no doc) | Claude Code | — | — | 0, 4 |
| 2 | 01-VISION.md | Codex | Claude Code | Claude Code `/simplify` | 1 |
| 3 | 02-OPPORTUNITIES.md | Codex | Claude Code | Claude Code `/simplify` | 1 |
| 4 | 04-ARCHITECTURE-INTEGRATION.md | Codex | Claude Code | Claude Code `/simplify` | 1 |
| 5 | 05-PATENT-ALIGNMENT.md | Codex | Claude Code | Claude Code `/simplify` | 1 |
| 6 | 06-GAP-ANALYSIS.md | Claude Code | Claude Code | Claude Code `/simplify` | 2 |
| 7 | 07-ACPX-ABSORPTION.md | Claude Code | Claude Code | Claude Code `/simplify` | 2 |
| 8 | 08-OPTIMIZATION-TARGETS.md | Claude Code | Claude Code | Claude Code `/simplify` | 2 |
| 9 | 03-PLAN.md | Claude Code | Claude Code | Claude Code `/simplify` | 3 |
| 10 | README.md | Claude Code | Claude Code | Claude Code `/simplify` | 4 |

**Profile rationale:**
- **Codex** for Phase 1 revision: independent docs need fast, file-focused edits with no cross-doc reasoning
- **Claude Code** for Phase 2-4 revision: dependent docs need cross-document reasoning to maintain consistency
- **Claude Code** for all validation: always requires cross-doc reasoning
- **Claude Code** for all simplify: runs the `/simplify` skill which reviews for reuse, quality, and efficiency

---

## Ultrathink Agent Teams

Every acp node deploys a **team of 5 ultrathink agents** — mental sub-agents that each focus on one dimension of the task. This ensures comprehensive coverage by forcing the model to reason about each concern independently before merging findings.

### Per-Node Team Composition

**`analyze_all` (Orchestrator — Phase 0)**

| Sub-Agent | Role | Focus |
|-----------|------|-------|
| Agent A | Numerical Auditor | Track every number across 9 docs, flag disagreements |
| Agent B | Cross-Reference Mapper | Map every inter-doc reference, verify accuracy |
| Agent C | Terminology Cataloger | Build canonical glossary, identify definitive source |
| Agent D | Timeline Validator | Verify week/phase assignments across docs |
| Agent E | Architecture Consistency | Component descriptions match across docs |

**`revise_doc` (Sub-flow — per document)**

| Sub-Agent | Role | Focus |
|-----------|------|-------|
| Agent A | Inconsistency Hunter | Find numerical/factual mismatches vs canonical values |
| Agent B | Cross-Reference Validator | Verify every inter-doc ref is accurate and resolves |
| Agent C | Terminology Enforcer | Ensure every term matches the canonical glossary |
| Agent D | Clarity Editor | Simplify complex sentences without losing precision |
| Agent E | Structure Guardian | Preserve headings, document flow, original intent |

**`validate_doc` (Sub-flow — per document)**

| Sub-Agent | Role | Focus |
|-----------|------|-------|
| Agent A | Internal Consistency | Every claim within the doc agrees with every other |
| Agent B | Cross-Reference Accuracy | Every ref to another doc is correct and current |
| Agent C | Terminology Compliance | Every term matches canonical glossary |
| Agent D | Numerical Accuracy | Every number matches canonical value across docs |
| Agent E | Completeness | No dangling references, missing sections, orphaned links |

**`simplify_doc` (Sub-flow — per document, uses `/simplify` skill)**

| Sub-Agent | Role | Focus |
|-----------|------|-------|
| Agent A | Redundancy Detector | Find repeated information, duplicated claims |
| Agent B | Clarity Optimizer | Identify overly complex sentences to simplify |
| Agent C | Filler Remover | Cut unnecessary words, hedging language, padding |
| Agent D | Format Verifier | Verify tables and code examples are accurate |
| Agent E | Preservation Guard | Ensure nothing critical lost — structure, refs, numbers |

**`final_validation` (Orchestrator — Phase 4)**

| Sub-Agent | Role | Focus |
|-----------|------|-------|
| Agent A | Numbers Sweep | One canonical number per metric everywhere |
| Agent B | Link Checker | Every [link](file.md) reference resolves |
| Agent C | Glossary Enforcer | Terms used consistently per canonical glossary |
| Agent D | Narrative Coherence | Story flows logically across all 9 docs |
| Agent E | Stale Content Hunter | No pre-absorption references remain in 01, 02, 04 |

### Why 5 Agents Per Node

The 5-agent pattern maps to orthogonal quality dimensions:

```
         ┌─── A: Numbers/Facts ───┐
         │                        │
         ├─── B: References ──────┤
         │                        │
Input ───┼─── C: Terminology ─────┼──→ Merged Output
         │                        │
         ├─── D: Clarity/Flow ────┤
         │                        │
         └─── E: Structure/Guard ─┘
```

Each agent operates independently on the same input, producing focused findings. The model merges all 5 perspectives before taking action. This prevents tunnel vision — no single concern dominates at the expense of others.

---

## Aggregate Metrics

| Metric | Value |
|--------|-------|
| Orchestrator nodes | 19 |
| Sub-flow nodes (per doc) | 10 |
| Total nodes (19 + 9 x 10) | **109** |
| Orchestrator edges | 15 |
| Sub-flow edges (per doc) | 11 |
| Total edges (15 + 9 x 11) | **114** |
| Retry loops (2/doc x 9 + 1 final) | **19** |
| Max parallel processes | 4 (Phase 1) |
| Total agent processes | 10 |
| Documents revised | 9 |

---

## Parallelism Model

acpx executes nodes **sequentially** within a single flow. Parallelism is achieved at the **process level**: each `spawn_phase*` shell node launches multiple `acpx flow run` commands as background processes (`&`) and waits for all to complete (`wait`).

```bash
# What spawn_phase1 actually runs:
( acpx flow run ./strategy-revision-doc.flow.ts -s doc-01 --input-json '...' --approve-all ) &
( acpx flow run ./strategy-revision-doc.flow.ts -s doc-02 --input-json '...' --approve-all ) &
( acpx flow run ./strategy-revision-doc.flow.ts -s doc-04 --input-json '...' --approve-all ) &
( acpx flow run ./strategy-revision-doc.flow.ts -s doc-05 --input-json '...' --approve-all ) &
wait
```

### APXM Migration Path

When APXM replaces acpx:
- Sub-flows become **DELEGATE** sub-DAGs in the dataflow scheduler
- Shell-level `&` + `wait` becomes **native parallel execution**
- Profile selection becomes **ModelRouter** policies
- Retry loops become **conditional back-edges** with counter state in 3-tier memory
- Phase gates become **barrier synchronization** primitives

---

## Execution Timeline

```
Time ──────────────────────────────────────────────────────────────────>

Phase 0   │ scan ─> analyze ─> plan ─> approve │
          │         Agent 1 (Claude)            │
          │                                     │
Phase 1   │                                     │ Ag.2: 01-VISION ────────── │
          │                                     │ Ag.3: 02-OPPORTUNITIES ─── │  (4 parallel)
          │                                     │ Ag.4: 04-ARCHITECTURE ──── │
          │                                     │ Ag.5: 05-PATENT ────────── │
          │                                     │                            │
Phase 2   │                                     │                            │ Ag.6: 06-GAP ──── │
          │                                     │                            │ Ag.7: 07-ACPX ─── │ (3 parallel)
          │                                     │                            │ Ag.8: 08-OPTIM ── │
          │                                     │                            │                   │
Phase 3   │                                     │                            │                   │ Ag.9: 03-PLAN ── │
          │                                     │                            │                   │                  │
Phase 4   │                                     │                            │                   │                  │ Ag.10: README │ Ag.1: FINAL ──> REPORT
```

---

## Running

```bash
# Full orchestrator (recommended)
cd flows/
npm run strategy

# Single document (testing)
npm run strategy:doc -- -s doc-01 \
  --input-json '{"file":"01-VISION.md","docsPath":"$HOME/projects/agents/docs/strategy","projectRoot":"$HOME/projects/agents","agentId":2,"phase":"independent","profile":"codex","analysisJson":"{}"}'

# Monolith fallback (all sequential, no sub-flows)
npm run strategy:monolith
```
