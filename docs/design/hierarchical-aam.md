---
title: "AAM Implementation Diagrams: From Flat HashMaps to Hierarchical File Tree"
description: "Visual mapping of how the Agent Abstract Machine is currently implemented vs how a file-tree-backed hierarchical implementation would work."
date: "2026-03-16"
---

# AAM Implementation Diagrams

> **See also:**
> - [AAM theory](../pxm/aam.md) -- formal specification of (B, G, C)
> - [Agent ontology](agent-ontology.md) -- conceptual foundation (skills, scoping, capabilities)
> - [Agentic OS](agentic-os.md) -- OS-level mapping of AAM subsystems
> - [Sessions](../implementation/runtime/sessions.md) -- per-node workspace layout (the physical AAM)

## Diagram 1: PXM Theory -- The Three Layers

The Program Execution Model defines three distinct layers. The runtime IMPLEMENTS the abstract machine -- it is not separate from it.

```
 +--------------------------------------------------------------------------+
 |                    PROGRAM EXECUTION MODEL (PXM)                         |
 |                                                                          |
 |  Defines the THEORY of how agents compute                                |
 |  - What is an agent? -> AAM = (B, G, C)                                 |
 |  - What operations exist? -> AIS (typed instructions)                    |
 |  - How do transitions work? -> d(AAM, Instr) -> AAM'                    |
 |  - How is execution scheduled? -> Dataflow (token-based)                 |
 +--------------------------------------------------------------------------+
 |                                                                          |
 |              +----------------------------------------------+            |
 |              |      ABSTRACT MACHINE (AAM)                  |            |
 |              |                                              |            |
 |              |  Defines WHAT an agent IS:                    |            |
 |              |    B = Beliefs (what it knows)                |            |
 |              |    G = Goals (what it wants)                  |            |
 |              |    C = Capabilities (what it can do)          |            |
 |              |                                              |            |
 |              |  This is a SPECIFICATION.                     |            |
 |              |  It says nothing about HOW (B,G,C)            |            |
 |              |  are stored or organized.                     |            |
 |              +--------------------+-------------------------+            |
 |                                   | "implemented by"                     |
 |                                   v                                      |
 |              +----------------------------------------------+            |
 |              |        RUNTIME                               |            |
 |              |                                              |            |
 |              |  IMPLEMENTS the Abstract Machine:             |            |
 |              |    B -> some concrete storage                 |            |
 |              |    G -> some concrete structure               |            |
 |              |    C -> some concrete registry                |            |
 |              |                                              |            |
 |              |  Also implements:                             |            |
 |              |    - AIS operation executors                  |            |
 |              |    - Dataflow scheduler                       |            |
 |              |    - Memory hierarchy (STM/LTM/Episodic)      |            |
 |              +----------------------------------------------+            |
 |                                                                          |
 +--------------------------------------------------------------------------+

 Analogy:

   Von Neumann PXM:                    Agent PXM (APXM):
   +-----------------+                 +----------------------+
   | Abstract Machine |                 | Abstract Machine     |
   | = (PC, Regs, Mem)|                 | = (B, G, C)          |
   +-----------------+                 +----------------------+
   | Runtime =        |                 | Runtime =            |
   |  CPU silicon     | <-- implements  |  APXM runtime        |
   |  + RAM chips     |                 |  (this is the        |
   |  + bus           |                 |   question below)    |
   +-----------------+                 +----------------------+
```

---

## Diagram 2: Current Implementation -- Flat AAM

This is how the APXM runtime currently implements the AAM. All state is flat, in-memory, and agent-global.

```
 +-------------------- Runtime (runtime.rs) ---------------------------+
 |                                                                      |
 |  aam: Aam ---------> Arc<RwLock<AamState>>                          |
 |                              |                                       |
 |                              v                                       |
 |              +----------------------------------+                    |
 |              |          AamState                 |                    |
 |              |                                   |                    |
 |              |  beliefs:      HashMap<String,     |                    |
 |              |                  Value>            | <-- FLAT. One      |
 |              |                                   |     global map.    |
 |              |  goals:        PriorityQueue       |     Every node     |
 |              |                <GoalId, u32>       |     sees it.       |
 |              |  goal_tree:    GoalTree            |                    |
 |              |                                   |                    |
 |              |  capabilities: HashMap<String,     | <-- FLAT. One      |
 |              |                CapabilityRecord>   |     global registry.|
 |              |                                   |                    |
 |              |  transitions:  Vec<TransitionRec>  |                    |
 |              |  call_stack:   Vec<CallFrame>      |                    |
 |              |  exception_handlers: HashMap       |                    |
 |              +----------------------------------+                    |
 |                                                                      |
 |  ALL flows share the SAME flat (B, G, C)                            |
 |  No scoping. No isolation. No hierarchy.                            |
 +----------------------------------------------------------------------+

 Problem: When Node A in Flow "main" writes UMEM("result", value),
          Node B in Flow "review" can see it immediately.
          There is no task-level isolation.
```

---

> **Status: Proposed** -- Diagrams 3+ describe future architecture.
> Diagrams 1-2 above reflect the current implementation. Diagrams 3 onward
> describe proposed extensions that have not been built yet.

---

## Diagram 3: The File Tree IS the AAM

The file-tree approach is not an authoring metaphor. It is a concrete
implementation of the AAM where the filesystem backs (B, G, C).

```
 The File Tree:                          Maps to AAM:

 my-agent/                               +-----------------------------+
 |                                        | ROOT AAM = (B0, G0, C0)    |
 |                                        |                             |
 +-- data/                                | B0 = {                      |
 |   +-- agent_name: "ResearchBot"        |   agent_name: "ResearchBot" |
 |   +-- domain: "ML papers"             |   domain: "ML papers"       |
 |                                        | }                           |
 +-- goals.toml                           |                             |
 |   goal = "Research and report on ML"   | G0 = Goal {                 |
 |   priority = 1                         |   desc: "Research ML"       |
 |                                        |   children: [G1, G2, G3]    |
 +-- tools/                               | }                           |
 |   +-- search_web.toml                  |                             |
 |   +-- read_paper.toml                  | C0 = {search_web, read}     |
 |                                        +----------+------------------+
 |                                                   |
 +-- research/            <-- SUB-TASK FOLDER        | inherits from
 |   |                                               v
 |   +-- data/                            +-----------------------------+
 |   |   +-- search_queries: [...]        | CHILD AAM1 = (B1, G1, C1)  |
 |   +-- prompts/                         | B1 = B0 + {                 |
 |   |   +-- instructions.md              |   search_queries: [...]     |
 |   +-- tools/                           | }                           |
 |       +-- arxiv_api.toml               | C1 = C0 + {arxiv_api}      |
 |                                        +-----------------------------+
```

---

## Diagram 4: Scoping Rules -- How Child AAMs Inherit

```
                     SCOPE INHERITANCE RULES

  +--------------- Parent AAM (directory) ------------------+
  |                                                          |
  |  B_parent = { agent: "Bot", domain: "ML" }              |
  |  G_parent = Goal("Research ML", priority=1)              |
  |  C_parent = { search_web, read_paper }                   |
  |                                                          |
  +----------------------+-----------------------------------+
                         |
          +--------------+------------------+
          |              |                  |
          v              v                  v
  +-----------+  +-----------+       +-----------+
  | INHERIT   |  | ISOLATE   |       | SNAPSHOT  |
  |           |  |           |       |           |
  | B = B_par |  | B = {}    |       | B = copy  |
  |  + B_local|  |  + B_local|       |  of B_par |
  |           |  |           |       |  (diverge)|
  | C = C_par |  | C = {}    |       |           |
  |  + C_local|  |  + C_local|       | C = copy  |
  +-----------+  +-----------+       +-----------+

  INHERIT:  Full access to parent (bidirectional writes).
  ISOLATE:  Clean slate. No parent state visible.
  SNAPSHOT: Copies parent state then diverges. Writes are local.
  FILTER:   Selective access. Only specified keys/caps visible.
```

---

## Diagram 5: Current vs Proposed -- Side by Side

```
 === CURRENT IMPLEMENTATION ===          === PROPOSED IMPLEMENTATION ===

   Runtime                                 Runtime
     |                                       |
     v                                       v
   Aam (single instance)                   AamTree (hierarchical)
     |                                       |
     v                                       v
 +--------------------+                +----------------------------+
 |     AamState       |                |      AamScope (root)       |
 |                    |                |                            |
 |  beliefs: HashMap  | <- FLAT        |  path: "my-agent/"         |
 |  goals: PriQueue   | <- FLAT        |  beliefs: data/*.toml      | <- from files
 |  caps: HashMap     | <- FLAT        |  goal: goals.toml          | <- from file
 |                    |                |  caps: tools/*.toml        | <- from files
 |  ALL nodes share   |                |                            |
 |  this one state.   |                |  children: [               |
 |                    |                |    AamScope("research/")   |
 |  No scoping.       |                |    AamScope("analysis/")   |
 |  No hierarchy.     |                |    AamScope("report/")     |
 +--------------------+                |  ]                         |
                                       |                            |
                                       |  Each child has its OWN    |
                                       |  scoped (B, G, C) that     |
                                       |  INHERITS from parent.     |
                                       +----------------------------+
```

---

## Diagram 6: The Full Stack -- How File Tree + Dataflow Compose

```
 +--------------------------------------------------------------------------+
 |                                                                          |
 |  FILE TREE (= Hierarchical AAM Implementation)                           |
 |                                                                          |
 |  my-agent/                                                               |
 |  +-- data/           <- B (Beliefs)                                      |
 |  +-- goals.toml      <- G (Goals)                                       |
 |  +-- tools/          <- C (Capabilities)                                 |
 |  +-- research/       <- Child AAM scope (sub-goal)                       |
 |  |   +-- data/       <- B_child (scoped beliefs)                        |
 |  |   +-- prompts/    <- Transition instructions                         |
 |  |   +-- tools/      <- C_child (scoped capabilities)                   |
 |  +-- analysis/       <- Child AAM scope (sub-goal)                       |
 |      +-- data/                                                           |
 |      +-- prompts/                                                        |
 |      +-- tools/                                                          |
 |                                                                          |
 |  The filesystem IS the state. Persistent. Navigable. Editable.           |
 |                                                                          |
 +---------------+--------------------------------+-------------------------+
                 |                                |
        "reads structure"                "reads/writes state"
                 |                                |
                 v                                |
 +------------------------------+                 |
 |  COMPILER                    |                 |
 |                              |                 |
 |  1. Reads file tree          |                 |
 |  2. Discovers goal hierarchy |                 |
 |  3. Parses prompts, tools    |                 |
 |  4. Builds ApxmGraph per     |                 |
 |     scope (DAG of AIS ops)   |                 |
 |  5. Optimizes (fusion, etc.) |                 |
 |  6. Emits execution plan     |                 |
 |                              |                 |
 |  See: optimization passes    |                 |
 +------------------------------+                 |
                 |                                |
        "execution plan"                          |
                 |                                |
                 v                                |
 +------------------------------+                 |
 |  DATAFLOW SCHEDULER          |<-----------------+
 |                              |
 |  Executes AIS operations     |  d(AAM_scope, Instr) -> AAM_scope'
 |  via token-based scheduling. |
 |                              |
 |  Parallelism is AUTOMATIC:   |
 |  research/ and analysis/     |
 |  are independent scopes ->   |
 |  their DAGs run in parallel. |
 +------------------------------+

 KEY INSIGHT:
   The file tree provides: Hierarchy, Persistence, Navigability, Modularity
   The dataflow scheduler provides: Parallelism, Optimization, Verification
   Together they implement the complete PXM.
```

See [optimization passes](../optimization/passes.md) for the compiler pass catalogue and [dataflow scheduler](../implementation/runtime/dataflow-scheduler.md) for the scheduling model.

---

## Diagram 7: Transition Function With Scoped AAM

Shows how d(AAM, Instr) -> AAM' works with hierarchical scoping.

```
 BEFORE transition:

   Root AAM:  B0 = {domain: "ML"}
              G0 = Goal("Research ML") [Active]
              C0 = {search_web}
                |
                +-- research/ AAM:  B1 = B0 + {queries: [...]}
                |                   G1 = Goal("Gather", parent=G0)
                |                   C1 = C0 + {arxiv_api}
                |
                +-- analysis/ AAM:  B2 = B0 + {focus: [...]}
                                    G2 = Goal("Analyze", parent=G0)
                                    C2 = C0


 EXECUTION: Node in research/ scope executes:
            ASK("Search for papers on {queries}")

   Step 1: Read from B1 -> queries = B1["queries"]  (scoped read)
   Step 2: Execute ASK via LLM -> result = "Found 5 papers: ..."
   Step 3: Write to B1: B1' = B1 + {search_results: "Found 5 papers..."}
   Step 4: Token emitted downstream (dataflow)


 AFTER transition:

   Root AAM:  B0 = {domain: "ML"}        <-- UNCHANGED (isolated)

                +-- research/ AAM:  B1' = B0 + {queries: [...],
                |                              search_results: "Found 5..."}
                |                   G1 = Goal("Gather", parent=G0) <-- still active
                |
                +-- analysis/ AAM:  B2 = B0 + {focus: [...]}
                                                    |
                      CAN'T see B1' changes --------+
                      (isolated until promoted)
```

---

## Diagram 8: Goal Tree = Directory Tree

```
 DIRECTORY STRUCTURE:                    GOAL TREE:

 my-agent/                              G0: "Research and report on ML"
 |                                       |   status: Active
 |                                       |   completion: AllChildren
 +-- research/                           +-- G1: "Gather papers"
 |   +-- academic/                       |   +-- G1a: "Search academic DBs"
 |   +-- patents/                        |   +-- G1b: "Search patent DBs"
 |                                       |
 +-- analysis/                           +-- G2: "Analyze findings"
 |                                       |   status: Pending (blocked on G1)
 +-- report/                             +-- G3: "Write report"
                                             status: Pending (blocked on G2)

 DIRECTORY NESTING = GOAL DECOMPOSITION
 DIRECTORY NAME = GOAL DESCRIPTION
 DIRECTORY CONTENTS = AAM SCOPE (B, G, C) FOR THAT GOAL
```

---

## Diagram 9: Capability Condensation/Expansion via the File Tree

The file tree naturally supports the "condense a workflow into a tool" pattern.

```
 BEFORE: research/ is a multi-step workflow (3 nodes in its DAG)

   my-agent/
   +-- research/                    <- directory = sub-workflow
       +-- prompts/instructions.md  <- multi-step instructions
       +-- tools/arxiv_api.toml
       +-- tools/semantic_scholar.toml
       +-- data/queries.json

   Compiles to: ASK(classify) -> INV(arxiv) -+
                                              +-> WAIT_ALL -> ASK(summarize)
                               INV(scholar) -+

   Execution time: ~8 seconds

 AFTER: Replace the workflow with a single tool.

   my-agent/
   +-- research/
       +-- tools/native_research.toml  <- REPLACED with one tool
       +-- data/queries.json           <- SAME data

   Compiles to: INV(native_research)   <- single node
   Execution time: ~2 seconds

 THE FILE TREE MAKES THIS A FILE OPERATION:
   1. Delete old prompts and tool files
   2. Add new tool definition
   3. Re-run: apxm execute my-agent/
```

---

## Diagram 10: The .apxm/ Directory -- Already Halfway There

> **Note:** This diagram refers to the `.apxm/` structure from an external
> project frontend, not the APXM runtime itself. Included for comparison.

```
 CURRENT .apxm/ STRUCTURE:          WHAT IT MAPS TO IN AAM:

 .apxm/
 +-- config.toml                    Agent configuration (NOT part of AAM)
 +-- .env                           API keys (NOT part of AAM)
 +-- prompts/                       Transition instructions (partial AAM)
 +-- skills/                        C (Capabilities)
 +-- hooks/                         Lifecycle callbacks (NOT part of AAM)
 +-- commands/                      Slash-commands (partial AAM.C)

 WHAT'S MISSING TO MAKE IT A FULL AAM:

 .apxm/
 +-- beliefs/               <-- NEW     B: Persistent beliefs
 +-- goals/                 <-- NEW     G: Goal tree
 +-- tools/                 <-- NEW     C: Tool definitions
 +-- workflows/             <-- NEW     Hierarchical workflow definitions
     +-- research/                      Each sub-directory = sub-AAM scope
```

---

## Diagram 11: The Missing Piece -- Runtime Control Plane

> **Status: Proposed** -- this describes planned infrastructure.

Answers: who creates folders? Who updates them? Who keeps runtime and file tree in sync?

```
 +--------------------------- APXM RUNTIME ----------------------------+
 |                                                                      |
 |  Existing today:                                                     |
 |    - Compiler/optimizer                                              |
 |    - Dataflow scheduler                                              |
 |    - AIS handlers                                                    |
 |    - Memory tiers (STM/LTM/Episodic)                                |
 |                                                                      |
 |  Missing control-plane services:                                     |
 |                                                                      |
 |   +--------------------------------------------------------------+  |
 |   | WorkspaceManager                                              |  |
 |   |  - ensure_scope_chain(company/team/agent/workflow/run)       |  |
 |   |  - create/open/archive scope folders                          |  |
 |   +--------------------------------------------------------------+  |
 |                          |                                           |
 |   +----------------------v--------------------------------------+   |
 |   | Materializer                                                 |   |
 |   |  - write scope.toml (scope_id, parent, policy, revision)    |   |
 |   |  - materialize beliefs/, goals/, capabilities/ skeletons     |   |
 |   +--------------------------------------------------------------+  |
 |                          |                                           |
 |   +----------------------v--------------------------------------+   |
 |   | StateProjector + PolicyEngine                               |   |
 |   |  - project AAM <-> file tree                                |   |
 |   |  - enforce inherit/isolate/filter promotion rules           |   |
 |   |  - bump namespace epochs on writes                          |   |
 |   +--------------------------------------------------------------+  |
 |                          |                                           |
 |                          v                                           |
 |              .apxm/workspaces/... (LIVE AAM BACKING STORE)          |
 |                                                                      |
 +----------------------------------------------------------------------+

 Key point:
   Agents do not directly "mkdir" arbitrary state.
   They request mutations through runtime operations, and the control plane
   applies them under policy and version control.
```

---

## Diagram 12: Company-Scale Scope Overlay (Shared + Isolated)

> **Status: Proposed** -- describes multi-tenant scoping for future use.

```
                    READ PATH (overlay lookup)

              Run Scope  --------+
                  |               |
                  v               |
            Workflow Scope        |
                  |               |
                  v               | fallback
              Agent Scope         |
                  |               |
                  v               |
               Team Scope         |
                  |               |
                  v               |
             Company Scope  <-----+


                    WRITE PATH (default = local only)

              Run Scope writes stay in Run Scope
              Promotion upward is EXPLICIT:
                promote(scope=workflow, key="new_fact", to=team)
```

---

## Diagram 13: Cache Plane + Namespace Epoch Invalidation

> **Status: Proposed** -- describes cache architecture for future use.

```
 +--------------------------- CACHE LAYERS ----------------------------+
 |                                                                      |
 |  [L0] In-flight single-flight map (dedupe concurrent identical work)|
 |  [L1] Run/workflow scoped caches (QMEM, INV, ASK/THINK/REASON)     |
 |  [L2] Team/company shared caches (reusable facts, stable outputs)   |
 |  [L3] Artifact cache (graph_hash -> .apxmobj)                       |
 |                                                                      |
 +----------------------------------------------------------------------+

 Invalidation primitive:
   namespace_epoch(scope_id, namespace) -> u64
   UMEM write -> bump epoch -> stale entries auto-miss
```

---

## Diagram 14: Clear Migration Plan (Phases + Gates)

> **Status: Proposed** -- staged implementation plan.

```
 PHASE 0 - Contracts + Observability
   deliver: ScopeId / ScopePath / ScopePolicy types, namespace epoch,
            trace fields (scope_id, revision, cache_hit)
   gate: all runs emit scope/cache telemetry

 PHASE 1 - Scoped State (no file-tree refactor yet)
   deliver: namespaced keys in STM/LTM, scope-aware ExecutionContext
   gate: zero cross-scope key collision regressions

 PHASE 2 - Runtime Control Plane + Materialization
   deliver: WorkspaceManager + Materializer + StateProjector
   gate: restart/resume from file-backed scopes is deterministic

 PHASE 3 - Multi-level Caching + Single-flight
   deliver: retrieval/tool/LLM/subgraph caches, in-flight dedupe
   gate: measurable duplicate-call reduction

 PHASE 4 - Goal-Driven Scheduling + Capability Unification
   deliver: hierarchical goal tree, goal priority projection
   gate: higher-priority goals complete earlier under contention

 PHASE 5 - Adaptive Condense/Expand
   deliver: condensation registry, reversible expansion fallback
   gate: swap workflow<->tool without caller DAG changes
```

---

## Summary: What Each Diagram Shows

| # | Diagram | Key Point |
|---|---------|-----------|
| 1 | PXM Theory Layers | Runtime IMPLEMENTS the abstract machine -- not separate from it |
| 2 | Current Flat AAM | All state is flat HashMaps shared globally -- no scoping |
| 3 | File Tree = AAM | Each directory IS an AAM instance with its own (B, G, C) |
| 4 | Scoping Rules | Inherit / Isolate / Snapshot / Filter policies for child AAMs |
| 5 | Current vs Proposed | Side-by-side: flat HashMaps vs hierarchical file-backed scopes |
| 6 | Full Stack | File tree (state) + compiler (optimization) + scheduler (execution) |
| 7 | Scoped Transitions | d(AAM_scope, Instr) -> AAM_scope' with isolation |
| 8 | Goal Tree = Dir Tree | Directory nesting IS goal decomposition |
| 9 | Condensation | Replace a directory of files with a single tool definition |
| 10 | .apxm/ Gap | Current structure is halfway there -- needs beliefs/, goals/, tools/ |
| 11 | Runtime Control Plane | Who materializes/manages scope folders and state projection |
| 12 | Company Scope Overlay | Shared context + local isolation across company/team/agent/workflow/run |
| 13 | Cache Plane + Epochs | Deterministic invalidation and single-flight dedupe for scale |
| 14 | Migration Phases | Staged implementation path with objective gates |
