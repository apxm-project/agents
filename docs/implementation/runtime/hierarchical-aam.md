---
title: "AAM Implementation Diagrams: From Flat HashMaps to Hierarchical File Tree"
description: "Visual mapping of how the Agent Abstract Machine is currently implemented vs how a file-tree-backed hierarchical implementation would work."
date: "2026-03-16"
---

# AAM Implementation Diagrams

## Diagram 1: PXM Theory — The Three Layers

The Program Execution Model defines three distinct layers. The key insight is
that the runtime IMPLEMENTS the abstract machine — it is not separate from it.

```
 ┌─────────────────────────────────────────────────────────────────────────┐
 │                    PROGRAM EXECUTION MODEL (PXM)                       │
 │                                                                        │
 │  Defines the THEORY of how agents compute                              │
 │  - What is an agent? → AAM = (B, G, C)                                │
 │  - What operations exist? → AIS (typed instructions)                   │
 │  - How do transitions work? → δ(AAM, Instr) → AAM'                   │
 │  - How is execution scheduled? → Dataflow (token-based)               │
 ├────────────────────────────────────────────────────────────────────────┤
 │                                                                        │
 │              ┌──────────────────────────────────────────┐              │
 │              │      ABSTRACT MACHINE (AAM)              │              │
 │              │                                          │              │
 │              │  Defines WHAT an agent IS:                │              │
 │              │    B = Beliefs (what it knows)            │              │
 │              │    G = Goals (what it wants)              │              │
 │              │    C = Capabilities (what it can do)      │              │
 │              │                                          │              │
 │              │  This is a SPECIFICATION.                 │              │
 │              │  It says nothing about HOW (B,G,C)        │              │
 │              │  are stored or organized.                 │              │
 │              └────────────────┬─────────────────────────┘              │
 │                               │                                        │
 │                               │ "implemented by"                       │
 │                               ▼                                        │
 │              ┌──────────────────────────────────────────┐              │
 │              │        RUNTIME                           │              │
 │              │                                          │              │
 │              │  IMPLEMENTS the Abstract Machine:         │              │
 │              │    B → some concrete storage              │              │
 │              │    G → some concrete structure            │              │
 │              │    C → some concrete registry             │              │
 │              │                                          │              │
 │              │  Also implements:                         │              │
 │              │    - AIS operation executors              │              │
 │              │    - Dataflow scheduler                   │              │
 │              │    - Memory hierarchy (STM/LTM/Episodic)  │              │
 │              └──────────────────────────────────────────┘              │
 │                                                                        │
 └────────────────────────────────────────────────────────────────────────┘

 Analogy:

   Von Neumann PXM:                    Agent PXM (APXM):
   ┌─────────────────┐                 ┌──────────────────────┐
   │ Abstract Machine │                 │ Abstract Machine     │
   │ = (PC, Regs, Mem)│                 │ = (B, G, C)          │
   ├─────────────────┤                 ├──────────────────────┤
   │ Runtime =        │                 │ Runtime =            │
   │  CPU silicon     │ ◄── implements  │  ??? ◄── THIS IS    │
   │  + RAM chips     │                 │          THE QUESTION│
   │  + bus           │                 │                      │
   └─────────────────┘                 └──────────────────────┘
```

---

## Diagram 2: Current Implementation — Flat AAM

This is how the APXM runtime currently implements the AAM. All state is flat,
in-memory, and agent-global.

```
 ┌──────────────────── Runtime (runtime.rs) ──────────────────────────────┐
 │                                                                        │
 │  aam: Aam ──────────► Arc<RwLock<AamState>>                           │
 │                              │                                         │
 │                              ▼                                         │
 │              ┌──────────────────────────────────┐                     │
 │              │          AamState                 │                     │
 │              │                                   │                     │
 │              │  beliefs:      HashMap<String,     │                     │
 │              │                  Value>            │ ◄── FLAT. One       │
 │              │                                   │     global map.     │
 │              │  goals:        PriorityQueue       │     Every node      │
 │              │                <GoalId, u32>       │     sees it.        │
 │              │  goal_tree:    GoalTree            │ ◄── parent-child    │
 │              │                                   │     via GoalTree    │
 │              │  goal_details: HashMap<GoalId,     │                     │
 │              │                  Goal>             │                     │
 │              │                                   │                     │
 │              │  capabilities: HashMap<String,     │ ◄── FLAT. One       │
 │              │                CapabilityRecord>   │     global registry.│
 │              │                                   │                     │
 │              │  transitions:  Vec<TransitionRec>  │                     │
 │              │  call_stack:   Vec<CallFrame>      │ ◄── used by          │
 │              │                                   │     enter/exit_op   │
 │              │  exception_handlers: HashMap       │                     │
 │              └──────────────────────────────────┘                     │
 │                              │                                         │
 │          ┌───────────────────┼────────────────────┐                   │
 │          │                   │                    │                    │
 │          ▼                   ▼                    ▼                    │
 │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐            │
 │  │ExecutionCtx  │  │ExecutionCtx  │  │ExecutionCtx      │            │
 │  │ Flow: main   │  │ Flow: review │  │ Flow: report     │            │
 │  │              │  │              │  │                   │            │
 │  │ aam: clone() │  │ aam: clone() │  │ aam: clone()     │            │
 │  │  ↑           │  │  ↑           │  │  ↑                │            │
 │  │  └── same ───┴──┴──┴── Arc ────┴──┴──┘                │            │
 │  │      underlying AamState                               │            │
 │  │                                                        │            │
 │  │  ALL flows share the SAME flat (B, G, C)              │            │
 │  │  No scoping. No isolation. No hierarchy.               │            │
 │  └────────────────────────────────────────────────────────┘            │
 └────────────────────────────────────────────────────────────────────────┘

 Problem: When Node A in Flow "main" writes UMEM("result", value),
          Node B in Flow "review" can see it immediately.
          There is no task-level isolation.
```

---

---

> **Everything below this point is future vision / speculative design.**
> Diagrams 1-2 above reflect the current implementation. Diagrams 3+ describe
> proposed extensions that have not been built yet.

---

## Diagram 3: The File Tree IS the AAM

The file-tree approach is not an authoring metaphor. It is a concrete
implementation of the AAM where the filesystem backs (B, G, C).

```
 The File Tree:                          Maps to AAM:

 my-agent/                               ┌─────────────────────────────┐
 │                                        │ ROOT AAM = (B₀, G₀, C₀)   │
 │                                        │                             │
 ├── data/                                │ B₀ = {                      │
 │   ├── agent_name: "ResearchBot"        │   agent_name: "ResearchBot" │
 │   └── domain: "ML papers"             │   domain: "ML papers"       │
 │                                        │ }                           │
 ├── goals.toml                           │                             │
 │   goal = "Research and report on ML"   │ G₀ = Goal {                 │
 │   priority = 1                         │   desc: "Research ML"       │
 │                                        │   children: [G₁, G₂, G₃]   │
 ├── tools/                               │ }                           │
 │   ├── search_web.toml                  │                             │
 │   └── read_paper.toml                  │ C₀ = {search_web, read}    │
 │                                        └──────────┬──────────────────┘
 │                                                    │
 ├── research/            ◄── SUB-TASK FOLDER         │ inherits from
 │   │                                                ▼
 │   ├── data/                            ┌─────────────────────────────┐
 │   │   └── search_queries: [...]        │ CHILD AAM₁ = (B₁, G₁, C₁) │
 │   │                                    │                             │
 │   ├── prompts/                         │ B₁ = B₀ + {                 │
 │   │   └── instructions.md              │   search_queries: [...]     │
 │   │   "Search for recent ML papers     │ }                           │
 │   │    on transformers. For each,       │                             │
 │   │    extract title, authors,          │ G₁ = Goal {                 │
 │   │    abstract, key findings."         │   desc: "Gather papers"     │
 │   │                                    │   parent: G₀                │
 │   └── tools/                           │ }                           │
 │       └── arxiv_api.toml               │                             │
 │                                        │ C₁ = C₀ + {arxiv_api}      │
 │                                        │    (inherits + extends)     │
 │                                        └──────────┬──────────────────┘
 │                                                    │
 ├── analysis/            ◄── SUB-TASK FOLDER         │
 │   │                                                ▼
 │   ├── data/                            ┌─────────────────────────────┐
 │   │   └── focus_areas: [...]           │ CHILD AAM₂ = (B₂, G₂, C₂) │
 │   │                                    │                             │
 │   ├── prompts/                         │ B₂ = B₀ + {                 │
 │   │   └── instructions.md              │   focus_areas: [...]        │
 │   │   "Analyze gathered papers.         │ }                           │
 │   │    Compare methodologies.           │                             │
 │   │    Identify trends."                │ G₂ = Goal {                 │
 │   │                                    │   desc: "Analyze papers"    │
 │   └── tools/                           │   parent: G₀                │
 │       (empty — inherits parent only)   │ }                           │
 │                                        │                             │
 │                                        │ C₂ = C₀                     │
 │                                        │    (inherits only, no local)│
 │                                        └──────────┬──────────────────┘
 │                                                    │
 └── report/              ◄── SUB-TASK FOLDER         │
     │                                                ▼
     ├── data/                            ┌─────────────────────────────┐
     │   └── output_format: "markdown"    │ CHILD AAM₃ = (B₃, G₃, C₃) │
     │                                    │                             │
     ├── prompts/                         │ B₃ = B₀ + {                 │
     │   └── instructions.md              │   output_format: "markdown" │
     │   "Synthesize analysis into        │ }                           │
     │    a structured report."            │                             │
     │                                    │ G₃ = Goal {                 │
     └── tools/                           │   desc: "Write report"      │
         └── write_file.toml              │   parent: G₀                │
                                          │ }                           │
                                          │                             │
                                          │ C₃ = C₀ + {write_file}     │
                                          │    (inherits + extends)     │
                                          └─────────────────────────────┘
```

---

## Diagram 4: Scoping Rules — How Child AAMs Inherit

```
                     SCOPE INHERITANCE RULES
                     ══════════════════════

  ┌─────────────── Parent AAM (directory) ────────────────┐
  │                                                        │
  │  B_parent = { agent: "Bot", domain: "ML" }            │
  │  G_parent = Goal("Research ML", priority=1)            │
  │  C_parent = { search_web, read_paper }                 │
  │                                                        │
  └──────────────────────┬─────────────────────────────────┘
                         │
          ┌──────────────┼─────────────────────────────────┐
          │              │                  │              │
          ▼              ▼                  ▼              ▼
  ┌───────────┐  ┌───────────┐      ┌───────────┐  ┌───────────┐
  │ INHERIT   │  │ ISOLATE   │      │ SNAPSHOT  │  │ FILTER    │
  │           │  │           │      │           │  │           │
  │ B = B_par │  │ B = {}    │      │ B = copy  │  │ B = B_par │
  │  + B_local│  │  + B_local│      │  of B_par │  │  ∩ {keys} │
  │           │  │           │      │  (diverge)│  │  + B_local│
  │ C = C_par │  │ C = {}    │      │           │  │           │
  │  + C_local│  │  + C_local│      │ C = copy  │  │ C = C_par │
  │           │  │           │      │  of C_par │  │  ∩ {names}│
  │ G = child │  │ G = child │      │           │  │           │
  │  of G_par │  │  of G_par │      │ G = child │  │ G = child │
  └───────────┘  └───────────┘      └───────────┘  └───────────┘

  INHERIT:  Full access to parent (shares Arc — bidirectional writes).
            Use case: sub-task that needs full context.

  ISOLATE:  Clean slate. No parent state visible.
            Use case: independent sub-agent, sandbox.

  SNAPSHOT: Copies parent state then diverges. Writes are local.
            Use case: speculative execution, rollback-safe branches.

  FILTER:   Selective access. Only specified keys/caps visible.
            Use case: security boundary, need-to-know.

  All modes:
  - Child's goal is always a CHILD of parent's goal
  - Child's transitions (writes) are LOCAL until explicitly promoted
  - Parent can observe child's state (for monitoring/debugging)
```

---

## Diagram 5: Current vs Proposed — Side by Side

```
 ══════════ CURRENT IMPLEMENTATION ═══════════    ════════ PROPOSED IMPLEMENTATION ═══════════

   Runtime                                         Runtime
     │                                               │
     ▼                                               ▼
   Aam (single instance)                           AamTree (hierarchical)
     │                                               │
     ▼                                               ▼
 ┌────────────────────┐                        ┌──────────────────────────┐
 │     AamState       │                        │      AamScope (root)     │
 │                    │                        │                          │
 │  beliefs: HashMap  │ ← FLAT                 │  path: "my-agent/"       │
 │  goals: PriQueue   │ ← FLAT                 │  beliefs: data/*.toml    │ ← from files
 │  caps: HashMap     │ ← FLAT                 │  goal: goals.toml        │ ← from file
 │                    │                        │  caps: tools/*.toml      │ ← from files
 │  ALL nodes share   │                        │                          │
 │  this one state.   │                        │  children: [             │
 │                    │                        │    AamScope("research/") │
 │  No scoping.       │                        │    AamScope("analysis/") │
 │  No hierarchy.     │                        │    AamScope("report/")   │
 │  STM dropped on    │                        │  ]                       │
 │  exit; LTM/Episodic│                        │                          │
 │  persist.          │                        │                          │
 └────────────────────┘                        │  Each child has its OWN  │
                                               │  scoped (B, G, C) that   │
                                               │  INHERITS from parent.   │
                                               │                          │
                                               │  Files persist across    │
                                               │  sessions. Beliefs       │
                                               │  survive restarts.       │
                                               └──────────────────────────┘

   Transition:                                  Transition:

   δ(AAM, UMEM("key", val))                    δ(AAM_scope, UMEM("key", val))
     → AAM.beliefs["key"] = val                  → write to scope's data/key.toml
     → visible to ALL nodes                      → visible only within this scope
     → lost when process exits                   → persisted to filesystem
                                                  → promoted to parent only if explicit
```

---

## Diagram 6: The Full Stack — How File Tree + Dataflow Compose

```
 ┌─────────────────────────────────────────────────────────────────────────┐
 │                                                                        │
 │                      DEVELOPER / AI AGENT                              │
 │                                                                        │
 │  Creates/modifies the file tree. This IS the AAM.                      │
 │                                                                        │
 └─────────────────────────────────┬──────────────────────────────────────┘
                                   │
                                   ▼
 ┌─────────────────────────────────────────────────────────────────────────┐
 │                                                                        │
 │  FILE TREE (= Hierarchical AAM Implementation)                         │
 │                                                                        │
 │  my-agent/                                                             │
 │  ├── data/           ← B (Beliefs)                                     │
 │  ├── goals.toml      ← G (Goals)                                      │
 │  ├── tools/          ← C (Capabilities)                                │
 │  ├── research/       ← Child AAM scope (sub-goal)                      │
 │  │   ├── data/       ← B_child (scoped beliefs)                       │
 │  │   ├── prompts/    ← Transition instructions                        │
 │  │   └── tools/      ← C_child (scoped capabilities)                  │
 │  └── analysis/       ← Child AAM scope (sub-goal)                      │
 │      ├── data/                                                         │
 │      ├── prompts/                                                      │
 │      └── tools/                                                        │
 │                                                                        │
 │  The filesystem IS the state. Persistent. Navigable. Editable.         │
 │                                                                        │
 └───────────────┬──────────────────────────────┬─────────────────────────┘
                 │                              │
        "reads structure"              "reads/writes state"
                 │                              │
                 ▼                              │
 ┌──────────────────────────────┐               │
 │                              │               │
 │  COMPILER                    │               │
 │                              │               │
 │  1. Reads file tree          │               │
 │  2. Discovers goal hierarchy │               │
 │     (directory structure)    │               │
 │  3. Parses prompts, tools    │               │
 │  4. Builds ApxmGraph per     │               │
 │     scope (DAG of AIS ops)   │               │
 │  5. Optimizes:               │               │
 │     - FuseAskOps             │               │
 │     - CSE, DCE               │               │
 │     - CondenseOps            │               │
 │  6. Emits execution plan     │               │
 │                              │               │
 │  The compiler extracts       │               │
 │  PARALLELISM from the        │               │
 │  file tree's structure.      │               │
 │                              │               │
 │  Files that don't depend     │               │
 │  on each other → parallel.   │               │
 │                              │               │
 └──────────────┬───────────────┘               │
                │                               │
        "execution plan"                        │
                │                               │
                ▼                               │
 ┌──────────────────────────────┐               │
 │                              │               │
 │  DATAFLOW SCHEDULER          │               │
 │                              │               │
 │  Executes AIS operations     │◄──────────────┘
 │  via token-based scheduling. │
 │                              │  δ(AAM_scope, Instr) → AAM_scope'
 │  Each operation:             │
 │  1. Reads from its AAM scope │  ← reads data/ in scope
 │  2. Executes (LLM/tool/mem)  │
 │  3. Writes to its AAM scope  │  ← writes data/ in scope
 │  4. Emits tokens downstream  │
 │                              │
 │  Parallelism is AUTOMATIC:   │
 │  research/ and analysis/     │
 │  are independent scopes →    │
 │  their DAGs run in parallel. │
 │                              │
 └──────────────────────────────┘

 KEY INSIGHT:
   The file tree provides: Hierarchy, Persistence, Navigability, Modularity
   The dataflow scheduler provides: Parallelism, Optimization, Verification
   Together they implement the complete PXM.
```

---

## Diagram 7: Transition Function With Scoped AAM

Shows how δ(AAM, Instr) → AAM' works with hierarchical scoping.

```
 BEFORE transition:

   Root AAM:  B₀ = {domain: "ML"}
              G₀ = Goal("Research ML") [Active]
              C₀ = {search_web}
                │
                ├── research/ AAM:  B₁ = B₀ + {queries: [...]}
                │                   G₁ = Goal("Gather", parent=G₀)
                │                   C₁ = C₀ + {arxiv_api}
                │
                └── analysis/ AAM:  B₂ = B₀ + {focus: [...]}
                                    G₂ = Goal("Analyze", parent=G₀)
                                    C₂ = C₀


 EXECUTION: Node in research/ scope executes:
            ASK("Search for papers on {queries}")

   Step 1: Read from B₁
           → queries = B₁["queries"]  (scoped read)

   Step 2: Execute ASK via LLM
           → result = "Found 5 papers: ..."

   Step 3: Write to B₁
           δ(AAM₁, ASK) → AAM₁'
           B₁' = B₁ + {search_results: "Found 5 papers..."}

   Step 4: Token emitted downstream (dataflow)


 AFTER transition:

   Root AAM:  B₀ = {domain: "ML"}        ◄── UNCHANGED (isolated)
              G₀ = Goal("Research ML") [Active]
              C₀ = {search_web}
                │
                ├── research/ AAM:  B₁' = B₀ + {queries: [...],
                │                              search_results: "Found 5..."}
                │                   G₁ = Goal("Gather", parent=G₀) ◄── still active
                │                   C₁ = C₀ + {arxiv_api}
                │                                    │
                │   ┌────────────────────────────────┘
                │   │  On G₁ completion → PROMOTE results to B₀
                │   │  Then mark G₁ = Completed
                │   │  Then check: all children of G₀ done? → G₀ = Completed
                │
                └── analysis/ AAM:  B₂ = B₀ + {focus: [...]}
                                                     │
                      CAN'T see B₁' changes ─────────┘
                      (isolated until promoted)
```

---

## Diagram 8: Goal Tree = Directory Tree

```
 DIRECTORY STRUCTURE:                    GOAL TREE:

 my-agent/                              G₀: "Research and report on ML"
 │                                       │   status: Active
 │                                       │   completion: AllChildren
 │                                       │
 ├── research/                           ├── G₁: "Gather papers"
 │   │                                   │   status: Active
 │   │                                   │   completion: AllChildren
 │   │                                   │
 │   ├── academic/                       │   ├── G₁ₐ: "Search academic DBs"
 │   │   └── prompts/                    │   │   status: Pending
 │   │       └── instructions.md         │   │   completion: Manual
 │   │                                   │   │
 │   └── patents/                        │   └── G₁ᵦ: "Search patent DBs"
 │       └── prompts/                    │       status: Pending
 │           └── instructions.md         │       completion: Manual
 │                                       │
 ├── analysis/                           ├── G₂: "Analyze findings"
 │   └── prompts/                        │   status: Pending (blocked on G₁)
 │       └── instructions.md             │   completion: Manual
 │                                       │
 └── report/                             └── G₃: "Write report"
     └── prompts/                            status: Pending (blocked on G₂)
         └── instructions.md                 completion: Manual

 DIRECTORY NESTING = GOAL DECOMPOSITION
 DIRECTORY NAME = GOAL DESCRIPTION
 DIRECTORY CONTENTS = AAM SCOPE (B, G, C) FOR THAT GOAL

 Completion propagation:
   G₁ₐ completes → G₁ᵦ completes → G₁ completes (AllChildren)
   → G₂ unblocks → G₂ completes
   → G₃ unblocks → G₃ completes
   → G₀ completes (AllChildren)
```

---

## Diagram 9: Capability Condensation/Expansion via the File Tree

The file tree naturally supports the "condense a workflow into a tool" pattern.

```
 BEFORE: research/ is a multi-step workflow (3 nodes in its DAG)

   my-agent/
   └── research/                    ← directory = sub-workflow
       ├── prompts/
       │   └── instructions.md      ← multi-step instructions
       ├── tools/
       │   ├── arxiv_api.toml
       │   └── semantic_scholar.toml
       └── data/
           └── queries.json

   Compiles to: ASK(classify) → INV(arxiv) ─┐
                                              ├→ WAIT_ALL → ASK(summarize)
                                INV(scholar) ─┘

   Execution time: ~8 seconds (3 LLM calls + 2 tool calls)


 AFTER: Model update makes search+summarize a native capability.
        "Condense" the workflow into a single tool.

   my-agent/
   └── research/                    ← SAME directory
       ├── prompts/                 ← REMOVED (model handles it)
       ├── tools/
       │   └── native_research.toml ← REPLACED with one tool
       │       capability = "model_native_research"
       │       type = "model_native"
       └── data/
           └── queries.json         ← SAME data

   Compiles to: INV(native_research)    ← single node

   Execution time: ~2 seconds (one model call)


 THE FILE TREE MAKES THIS A FILE OPERATION:
   1. Delete old prompts and tool files
   2. Add new tool definition
   3. Re-run: apxm execute my-agent/

 No code changes. No recompilation of other parts.
 The compiler reads the updated file tree and generates a simpler DAG.

 THE REVERSE (expansion) also works:
   If native_research is too expensive, replace the tool file
   with a prompts/ folder and multiple tool definitions.
   The compiler generates a multi-node DAG automatically.
```

---

## Diagram 10: The .apxm/ Directory — Already Halfway There

> **Note:** This diagram refers to the `.apxm/` structure from an external
> project (apxm frontend), not the APXM runtime itself. It is included for
> comparison purposes only.

```
 CURRENT .apxm/ STRUCTURE:          WHAT IT MAPS TO IN AAM:

 .apxm/
 ├── config.toml                         Agent configuration (model, provider)
 │   provider = "anthropic"              └── NOT part of AAM (meta-config)
 │   model = "claude-sonnet-4-20250514"
 │
 ├── .env                                API keys
 │   ANTHROPIC_API_KEY=sk-...            └── NOT part of AAM (credentials)
 │
 ├── prompts/                            ┌── Transition instructions
 │   ├── system.md                       │   System-level prompt
 │   └── ask.md                          │   Per-operation prompts
 │                                       │   PARTIALLY maps to AAM transitions
 │                                       └── (how δ is parameterized)
 │
 ├── skills/                             ┌── C (Capabilities) !
 │   ├── code-review.md                  │   Each skill IS a capability
 │   └── summarize.md                    │   with YAML frontmatter schema
 │                                       └── ALREADY maps to AAM.C
 │
 ├── hooks/                              Lifecycle callbacks
 │   └── on-task-complete.sh             └── NOT part of AAM (runtime hooks)
 │
 └── commands/                           Slash-commands
     └── deploy.md                       └── PARTIALLY maps to AAM.C
                                             (commands are invocable)


 WHAT'S MISSING TO MAKE IT A FULL AAM:

 .apxm/
 ├── config.toml                         (existing — meta-config)
 ├── .env                                (existing — credentials)
 │
 ├── beliefs/               ◄── NEW     B: Persistent beliefs
 │   ├── user_prefs.toml                 Long-lived knowledge
 │   └── learned_facts.toml              (currently in LTM SQLite,
 │                                        not in file tree)
 │
 ├── goals/                 ◄── NEW     G: Goal tree
 │   └── current.toml                    Active goals with hierarchy
 │                                        (currently flat PriorityQueue)
 │
 ├── tools/                 ◄── NEW     C: Tool definitions
 │   ├── bash.toml                       (currently hardcoded in
 │   ├── search_web.toml                  register_standard_tools())
 │   └── mcp/
 │       └── filesystem.toml             MCP server configs
 │
 ├── prompts/                            (existing — transition instructions)
 ├── skills/                             (existing — maps to C)
 ├── hooks/                              (existing — lifecycle)
 └── commands/                           (existing — maps to C)

 ├── workflows/             ◄── NEW     Hierarchical workflow definitions
 │   └── research/                       Each sub-directory = sub-AAM scope
 │       ├── data/                       B_local for this workflow
 │       ├── prompts/                    Instructions for this scope
 │       └── tools/                      C_local for this scope
```

---

## Summary: What Each Diagram Shows

| # | Diagram | Key Point |
|---|---------|-----------|
| 1 | PXM Theory Layers | Runtime IMPLEMENTS the abstract machine — not separate from it |
| 2 | Current Flat AAM | All state is flat HashMaps shared globally — no scoping |
| 3 | File Tree = AAM | Each directory IS an AAM instance with its own (B, G, C) |
| 4 | Scoping Rules | Inherit / Isolate / Filter policies for child AAMs |
| 5 | Current vs Proposed | Side-by-side: flat HashMaps vs hierarchical file-backed scopes |
| 6 | Full Stack | File tree (state) + compiler (optimization) + scheduler (execution) |
| 7 | Scoped Transitions | δ(AAM_scope, Instr) → AAM_scope' with isolation |
| 8 | Goal Tree = Dir Tree | Directory nesting IS goal decomposition |
| 9 | Condensation | Replace a directory of files with a single tool definition |
| 10 | .apxm/ Gap | Current structure is halfway there — needs beliefs/, goals/, tools/ |
| 11 | Runtime Control Plane | Defines who materializes/manages scope folders and state projection |
| 12 | Company Scope Overlay | Shows shared context + local isolation across company/team/agent/workflow/run |
| 13 | Cache Plane + Epochs | Deterministic invalidation and single-flight dedupe for scale |
| 14 | Migration Phases | Clear staged implementation path with objective gates |

---

## Investigation Anchors (Current Runtime Constraints)

The new diagrams below are grounded in current runtime behavior:

- Runtime currently carries one shared `Aam` handle and shared memory handles
  across execution contexts (global state shape).
- Child flow execution (`FLOW_CALL`) inherits parent context and writes temporary
  flow arguments into shared STM keys.
- Artifact caching exists today, but runtime-scoped retrieval/tool/LLM/subgraph
  caches are not yet first-class.
- Tool registration has two paths (`tools.json` CLI and runtime config loading)
  that are not unified into one capability discovery pipeline.
- Session-lane serialization exists, which is a useful foundation for future
  namespace/scope-level concurrency control.

These constraints are why the control plane, scope overlays, and cache model are
introduced in Diagrams 11-14.

---

## Diagram 11: The Missing Piece — Runtime Control Plane

This answers the practical question directly:

> Who creates folders? Who updates them? Who keeps runtime and file tree in sync?

```
 ┌───────────────────────────── APXM RUNTIME ─────────────────────────────┐
 │                                                                         │
 │  Existing today:                                                        │
 │    - Compiler/optimizer                                                 │
 │    - Dataflow scheduler                                                 │
 │    - AIS handlers                                                       │
 │    - Memory tiers (STM/LTM/Episodic)                                   │
 │                                                                         │
 │  Missing control-plane services:                                        │
 │                                                                         │
 │   ┌─────────────────────────────────────────────────────────────────┐   │
 │   │ WorkspaceManager                                                │   │
 │   │  - ensure_scope_chain(company/team/agent/workflow/run)         │   │
 │   │  - create/open/archive scope folders                            │   │
 │   └──────────────────────┬──────────────────────────────────────────┘   │
 │                          │                                              │
 │   ┌──────────────────────▼──────────────────────────────────────────┐   │
 │   │ Materializer                                                    │   │
 │   │  - write scope.toml (scope_id, parent, policy, revision)       │   │
 │   │  - materialize beliefs/, goals/, capabilities/ skeletons        │   │
 │   └──────────────────────┬──────────────────────────────────────────┘   │
 │                          │                                              │
 │   ┌──────────────────────▼──────────────────────────────────────────┐   │
 │   │ StateProjector + PolicyEngine                                  │   │
 │   │  - project AAM <-> file tree                                   │   │
 │   │  - enforce inherit/isolate/filter promotion rules              │   │
 │   │  - bump namespace epochs on writes                             │   │
 │   └──────────────────────┬──────────────────────────────────────────┘   │
 │                          │                                              │
 │                          ▼                                              │
 │              .apxm/workspaces/... (LIVE AAM BACKING STORE)             │
 │                                                                         │
 └─────────────────────────────────────────────────────────────────────────┘

 Key point:
   Agents do not directly "mkdir" arbitrary state.
   They request mutations through runtime operations, and the control plane
   applies them under policy and version control.
```

---

## Diagram 12: Company-Scale Scope Overlay (Shared + Isolated)

For multi-agent organizations, we need reusable shared context and strict
local isolation at the same time.

```
                    READ PATH (overlay lookup)

              Run Scope  ───────────────────────────────┐
                  │                                      │
                  ▼                                      │
            Workflow Scope                               │
                  │                                      │
                  ▼                                      │
              Agent Scope                               fallback
                  │                                      │
                  ▼                                      │
               Team Scope                                │
                  │                                      │
                  ▼                                      │
             Company Scope  ◄────────────────────────────┘


                    WRITE PATH (default = local only)

              Run Scope writes stay in Run Scope
              Workflow Scope writes stay in Workflow Scope
              Agent Scope writes stay in Agent Scope
              ...

              Promotion upward is EXPLICIT:
                promote(scope=workflow, key="new_fact", to=team)
```

### Semantics Matrix

| AAM Component | Read Behavior | Default Write Behavior | Promotion |
|---|---|---|---|
| Beliefs (B) | Overlay fallback (run → ... → company) | Local scope only | Explicit, policy-gated |
| Goals (G) | Parent + local visibility | Local tree updates | Completion/status propagation |
| Capabilities (C) | Inherited then filtered | Registry mutation in local scope | Optional publish upward |

This enables:
- team-wide cached knowledge at company/team scopes
- run-local scratch data without polluting global state
- deterministic sharing through explicit promotion

---

## Diagram 13: Cache Plane + Namespace Epoch Invalidation

Caching is where company-scale cost/latency wins happen.

```
 ┌──────────────────────────── CACHE LAYERS ──────────────────────────────┐
 │                                                                        │
 │  [L0] In-flight single-flight map                                      │
 │       key -> shared future (dedupe concurrent identical work)          │
 │                                                                        │
 │  [L1] Run/workflow scoped caches                                       │
 │       - retrieval cache (QMEM)                                         │
 │       - tool cache (INV)                                               │
 │       - LLM response cache (ASK/THINK/REASON)                          │
 │                                                                        │
 │  [L2] Team/company shared caches                                       │
 │       - reusable facts, stable tool outputs, compiled subgraph outputs │
 │                                                                        │
 │  [L3] Artifact cache (compile output)                                  │
 │       graph_hash -> .apxmobj                                           │
 │                                                                        │
 └────────────────────────────────────────────────────────────────────────┘

 Invalidation primitive:

   namespace_epoch(scope_id, namespace) -> u64

   UMEM write:
     bump(namespace_epoch(scope_id, namespace))

   Cache key includes epoch:
     key = hash(op_type, inputs, scope_id, namespace, epoch, model/tool version)

 Result:
   No global flushes required.
   Stale entries auto-miss when epoch changes.
```

### Recommended Cache Keys

| Cache | Key Includes | Invalidates When |
|---|---|---|
| Retrieval (`QMEM`) | scope + query + tier + limit + epoch | `UMEM` in namespace |
| Tool (`INV`) | cap name + args hash + cap version + scope + epoch | tool/config/policy change |
| LLM | model + prompt hash + sys prompt hash + tools hash + temperature + epoch | prompt/model/tools/data change |
| Subgraph | sub-DAG fingerprint + input hash + epoch | dependency write or plan revision |

---

## Diagram 14: Clear Migration Plan (Phases + Gates)

```
 PHASE 0 ─ Contracts + Observability
   deliver:
     - ScopeId / ScopePath / ScopePolicy types
     - namespace epoch contract
     - trace fields: scope_id, revision, cache_hit, cache_key
   gate:
     - all runs emit scope/cache telemetry

 PHASE 1 ─ Scoped State (no file-tree refactor yet)
   deliver:
     - namespaced keys in STM/LTM
     - scope-aware ExecutionContext
     - epoch-aware QMEM/UMEM
   gate:
     - zero cross-scope key collision regressions

 PHASE 2 ─ Runtime Control Plane + Materialization
   deliver:
     - WorkspaceManager + Materializer + StateProjector
     - deterministic .apxm/workspaces layout with revisions
   gate:
     - restart/resume from file-backed scopes is deterministic

 PHASE 3 ─ Multi-level Caching + Single-flight
   deliver:
     - retrieval/tool/LLM/subgraph caches
     - in-flight dedupe coordinator
   gate:
     - measurable duplicate-call reduction in multi-agent workloads

 PHASE 4 ─ Goal-Driven Scheduling + Capability/Workflow Unification
   deliver:
     - hierarchical goal tree
     - goal priority projection to scheduler priority
     - capability resolves to native tool OR sub-workflow
   gate:
     - higher-priority goals complete earlier under contention

 PHASE 5 ─ Adaptive Condense/Expand
   deliver:
     - condensation registry (workflow pattern -> capability)
     - reversible expansion fallback
   gate:
     - swap workflow<->tool without caller DAG changes
```

---

## Storyline Check: The Direction Is Now Explicit

1. The file tree is the **state implementation** of AAM.
2. The dataflow DAG is the **execution plan** extracted from that state.
3. The runtime control plane is the **missing connector** that keeps both in sync.
4. Scoped overlays + epochs unlock safe company-scale caching and reuse.
5. The migration plan preserves APXM strengths while adding hierarchical state.
