# APXM Agent Ontology

*The foundational model: what an agent IS, formally defined.*

> **See also:**
> - [AAM theory](../pxm/aam.md) -- the formal Agent Abstract Machine specification
> - [Session implementation](../implementation/runtime/sessions.md) -- how per-node workspaces manifest AAM state
> - [Agent profiles](agent-profiles.md) -- controlling what each node can do
> - [Hierarchical AAM](hierarchical-aam.md) -- file-tree-backed scoping extensions

---

## Part 1: The Agent Triple

An agent is a tuple:

```
Agent = (B, G, C)
```

Where:
- **B (Beliefs)** -- what the agent knows or believes to be true
- **G (Goals)** -- what the agent is trying to achieve
- **C (Capabilities)** -- what the agent can do

This is the **Agent Abstract Machine (AAM)** -- the minimal sufficient state for autonomous behavior.

---

### Beliefs (B)

Beliefs are the agent's knowledge store. They have structure:

```
B = {
    STM:      HashMap<String, Value>,   // Short-term: recent context, working memory
    LTM:      VectorStore<Value>,       // Long-term: persistent knowledge, semantic search
    Episodic: AppendLog<Event>,         // Execution traces, for reflection
}
```

**Key properties:**
- STM is volatile (cleared per execution or session)
- LTM persists across executions (MemoCache, SQLite)
- Episodic is append-only (`trace.ndjson`)

**Belief transitions:**
Every non-trivial operation is a belief transition:
```
B[t] --op--> B[t+1]
```

Examples:
- `QMEM`: reads from B (no mutation)
- `UMEM`: writes to B
- `REASON`: reads B, transforms, writes B'
- `VERIFY`: confirms or retracts beliefs
- `REFLECT`: integrates episodic trace into LTM
- `INV`/`EXC`: external world feeds back into B

See [Memory operations](../implementation/ais/memory-ops.md) for the AIS-level specification of each.

---

### Goals (G)

Goals are what the agent is trying to achieve. They have lifecycle:

```
G = PriorityQueue<Goal>

Goal = {
    id:          GoalId,
    description: String,
    priority:    u32,
    status:      Pending | Active | Completed | Failed | Cancelled,
    parent_id:   Option<GoalId>,
}
```

**Goal sources:**
1. **External injection** -- the input prompt (`TASK="..."`)
2. **Decomposition** -- PLAN op breaks one goal into subgoals
3. **Dynamic update** -- UPDATE_GOAL modifies goals mid-execution
4. **Satisfaction** -- RETURN signals goal achieved

**Goal hierarchy (GoalTree):**
Goals form a tree. Parent completion depends on children:
```
CompletionPolicy = AllChildren | AnyChild | Manual
```

When a child completes, the runtime checks if the parent should auto-complete.

---

### Capabilities (C)

Capabilities are what the agent can do:

```
C = HashMap<String, Capability>

Capability = {
    name:        String,
    description: String,
    input_schema: JsonSchema,
    available:   bool,
}
```

**Two kinds of capabilities:**

1. **Primitive ops** -- the AIS operations (INV, EXC, QMEM, THINK, etc.)
   - Built into the runtime
   - Atomic execution
   - Defined effect on (B, G)

2. **Skills** -- named, reusable APXM subgraphs
   - Composed of primitive ops
   - Have declared preconditions and effects
   - Can invoke other skills or agents via FLOW_CALL

A skill IS a graph. See [AIS reference](../pxm/ais.md) for the full operation catalogue.

---

## Part 2: Skills as Graphs

A skill is a compiled APXM subgraph with metadata:

```
Skill = {
    name:          String,
    preconditions: Vec<BeliefCondition>,   // What must be true in B before invoking
    effects:       Vec<BeliefEffect>,      // What changes in B after completion
    graph:         AISGraph,               // The actual implementation
}
```

**Example:**

```
skill: code_review
preconditions:
  - beliefs.has("code_to_review")
  - beliefs.has("review_criteria")
effects:
  - beliefs.set("review_result", output)
  - beliefs.set("issues_found", count)
graph: [
  ASK(template="Summarize the code: {{code_to_review}}") -> $summary
  REASON(template="Apply criteria {{review_criteria}} to {{summary}}") -> $analysis
  VERIFY(claim=$analysis, evidence=$code_to_review) -> $verified
  UMEM(key="review_result", value=$verified)
  RETURN($verified)
]
```

**Why this matters:**

1. **Composability** -- skills call skills via FLOW_CALL
2. **Isolation** -- each skill invocation gets its own AAM scope (see Part 3)
3. **Declarative effects** -- the compiler knows what beliefs each skill reads/writes
4. **Optimization** -- the compiler can fuse, parallelize, or reorder skill invocations (see [optimization passes](../optimization/passes.md))

## Part 3: Hierarchical AAM and Context Isolation

Each skill invocation gets its own scoped AAM. When a skill is invoked via FLOW_CALL or DELEGATE, the runtime creates a **child scope**:

```
ScopeSpec = {
    beliefs:      ScopePolicy,
    goals:        ScopePolicy,
    capabilities: ScopePolicy,
}

ScopePolicy = Inherit | Isolate | Snapshot | Filter(Vec<String>)
```

**Policy semantics:**

| Policy | Meaning |
|--------|---------|
| `Inherit` | Child sees parent's state. Child writes ARE visible to parent. |
| `Isolate` | Child starts empty. No leakage either direction. |
| `Snapshot` | Child gets a copy at invocation time. Writes stay local. |
| `Filter(keys)` | Child only sees listed keys (snapshot semantics for those). |

When `code_review.skill` runs, it might get:
```rust
ScopeSpec {
    beliefs: Filter(["code_to_review", "review_criteria"]),  // Only see what it needs
    goals: Isolate,                                           // Own goal stack
    capabilities: Inherit,                                    // Can use parent's tools
}
```

The skill runs in isolation -- it cannot see or modify the parent's other beliefs. It returns a result, and the parent decides what to do with it.

For the full file-tree-backed extension of this model, see [Hierarchical AAM](hierarchical-aam.md).

---

### Why This Matters: Context Window Management

LLMs have finite context windows. If the agent has a flat (B, G, C), every operation sees everything and the context explodes.

With hierarchical AAM:

```
Parent Agent (B_parent, G_parent, C_parent)
    |
    +-- FLOW_CALL(code_review, scope=Snapshot)
    |       |
    |       +-- Child Scope (B_child, G_child, C_child)
    |               |
    |               +-- Only sees filtered beliefs
    |               +-- Has own goal stack
    |               +-- Returns result to parent
    |
    +-- FLOW_CALL(security_audit, scope=Isolate)
            |
            +-- Child Scope (empty B, own G, inherited C)
                    |
                    +-- Starts fresh
                    +-- Cannot see code_review results unless passed explicitly
```

**The per-node workspace (`nodes/{id}_{name}/`) is the physical manifestation of this.** See [sessions](../implementation/runtime/sessions.md) for the directory layout.

Each node has:
- `CLAUDE.md` -- assembled context for THIS node only
- `output.json` -- result of THIS node
- `skills/` -- skills available to THIS node
- `trace.ndjson` -- what THIS node did

The filesystem IS the hierarchical AAM rendered to disk.

---

### Context Assembly via ContextStack

The runtime assembles each node's context window:

```
ContextStack.assemble(node_id, profile) -> Context
```

Where `profile` controls:
- Which beliefs to include (scope filtering)
- Which prior outputs to include (token dependencies)
- Which skills to expose
- What depth of history to show

Different profiles for different agent roles:
- **Architect profile**: sees high-level goals, system design beliefs
- **Coder profile**: sees code beliefs, implementation details
- **Reviewer profile**: sees code + tests + criteria

Same agent, same skill, different context based on profile. See [Agent profiles](agent-profiles.md) for the design.

---

## Part 4: Skills Invoking Agents

The hierarchy goes deeper. Skills can invoke other agents:

```
FLOW_CALL(target="agent:security-auditor", task="audit {{code}}")
DELEGATE(agent_id="frontend-specialist", task=$subtask)
SPAWN_AGENT(name="ephemeral-worker", capabilities=[...])
```

**An agent IS an AAM with a default skill (its main graph).**

When Agent A invokes Agent B:
1. Agent B is instantiated with its own (B, G, C)
2. The task becomes B's initial goal
3. B executes its skill graph
4. B returns a result
5. A receives the result into its own B

**Multi-agent = hierarchical AAM with cross-agent FLOW_CALL.** See [multi-agent guide](../guides/multi-agent.md) for practical usage.

```
Agent: Orchestrator
    |
    +-- DELEGATE(agent=Coder, task="implement feature")
    |       |
    |       +-- Agent: Coder (own AAM)
    |               +-- skills: [code_gen, refactor, test_write]
    |               +-- Returns: implementation
    |
    +-- DELEGATE(agent=Reviewer, task="review {{implementation}}")
    |       |
    |       +-- Agent: Reviewer (own AAM)
    |               +-- skills: [code_review, security_audit]
    |               +-- Returns: review_result
    |
    +-- MERGE(implementation, review_result) -> final_output
```

Each agent is isolated. The orchestrator controls what flows between them.

## Part 5: The Full Picture

```
+-------------------------------------------------------------------+
|                         AGENT                                      |
|                                                                    |
|  +-------------------------------------------------------------+  |
|  | BELIEFS (B)                                                  |  |
|  |  +-- STM (working memory, recent context)                   |  |
|  |  +-- LTM (persistent knowledge, MemoCache)                  |  |
|  |  +-- Episodic (execution traces, trace.ndjson)              |  |
|  +-------------------------------------------------------------+  |
|                                                                    |
|  +-------------------------------------------------------------+  |
|  | GOALS (G)                                                    |  |
|  |  +-- Input prompt (external injection)                      |  |
|  |  +-- Decomposed subgoals (via PLAN)                         |  |
|  |  +-- GoalTree with CompletionPolicy                         |  |
|  +-------------------------------------------------------------+  |
|                                                                    |
|  +-------------------------------------------------------------+  |
|  | CAPABILITIES (C)                                             |  |
|  |  +-- Primitive Ops (AIS operations)                         |  |
|  |  +-- Skills (named APXM subgraphs)                          |  |
|  |       +-- code_review.skill   -> [ASK -> REASON -> VERIFY]  |  |
|  |       +-- debug.skill         -> [REASON -> EXC -> REFLECT] |  |
|  |       +-- research.skill      -> [INV -> THINK -> UMEM]     |  |
|  +-------------------------------------------------------------+  |
|                                                                    |
|  +-------------------------------------------------------------+  |
|  | EXECUTION                                                    |  |
|  |  +-- Skill invocation creates child AAM scope               |  |
|  |  +-- ScopePolicy controls belief/goal/capability sharing    |  |
|  |  +-- ContextStack assembles LLM context per node            |  |
|  |  +-- Per-node workspace = physical AAM manifestation        |  |
|  +-------------------------------------------------------------+  |
|                                                                    |
+-------------------------------------------------------------------+
```

---

## Part 6: Formal Definitions

### Definition 1: Agent
```
Agent := (B, G, C, S)
where:
  B = Beliefs    : STM x LTM x Episodic
  G = Goals      : PriorityQueue<Goal> with GoalTree
  C = Capabilities : PrimitiveOps U Skills
  S = Skills     : Map<SkillId, Skill>
```

### Definition 2: Skill
```
Skill := (name, pre, eff, graph)
where:
  name  : String
  pre   : Set<BeliefCondition>     -- preconditions on B
  eff   : Set<BeliefEffect>        -- effects on B after execution
  graph : AISGraph                 -- the implementation
```

### Definition 3: Execution Step
```
exec : (AAM, Op) -> AAM'

For operation op at time t:
  (B_t, G_t, C_t) --op--> (B_{t+1}, G_{t+1}, C_{t+1})

The transition depends on op's effect signature.
```

### Definition 4: Skill Invocation
```
invoke_skill : (AAM_parent, Skill, ScopeSpec) -> (AAM_child, Result)

1. Create AAM_child by applying ScopeSpec to AAM_parent
2. Execute Skill.graph in AAM_child
3. Return result to AAM_parent
4. AAM_parent decides how to integrate result into its B
```

### Definition 5: Agent Invocation
```
invoke_agent : (AAM_caller, AgentId, Task) -> Result

1. Load Agent definition (its default skill + config)
2. Create fresh AAM for agent
3. Set Task as initial goal
4. Execute agent's skill graph
5. Return result to caller
```

---

## Part 7: Why This Architecture

### 7.1 Context Window Efficiency

Problem: LLMs have finite context. Flat agent state blows up.

Solution: Hierarchical AAM with scope filtering. Each skill sees only what it needs.

### 7.2 Composability

Problem: Building complex agents from scratch is hard.

Solution: Skills are reusable subgraphs. Compose them via FLOW_CALL. An agent is just a skill with an identity.

### 7.3 Isolation and Safety

Problem: Agents can interfere with each other, leak context, or cause side effects.

Solution: ScopePolicy controls sharing. Snapshot = no leakage. Isolate = clean slate.

### 7.4 Compilation and Optimization

Problem: Dynamic agent systems are hard to analyze or optimize.

Solution: Skills declare preconditions and effects. The [compiler](../implementation/compiler/overview.md) can:
- Check that belief dependencies are satisfied
- Fuse adjacent skills with compatible effects
- Parallelize skills with non-overlapping effects
- Optimize prompts based on skill context

### 7.5 Debugging and Reflection

Problem: Multi-agent systems are hard to debug.

Solution: Per-node workspaces. Each skill invocation has its own trace. Episodic memory is append-only. REFLECT can analyze its own history. See [debugging guide](../guides/debugging.md).

---

## Part 8: Connection to Research

### CoALA (Cognitive Architectures for Language Agents)

CoALA proposes modular memory (working/episodic/semantic/procedural), a structured action space, and a generalized decision process.

APXM implements all of this:
- STM = working, LTM = semantic, Episodic = episodic
- Procedural memory = Skills (compiled subgraphs)
- Action space = AIS ops
- Decision process = scheduler + goal priority

### ReAct (Reasoning + Acting)

ReAct interleaves reasoning traces with actions. In APXM, every reasoning op (THINK, REASON, REFLECT) produces a trace. Every action (INV, EXC) updates beliefs. The interleaving is structural in the graph.

### Meta Semi-Formal Reasoning (2603.01896)

Key insight: Structure prevents skipping cases or making unsupported claims. In APXM, the typed instruction set IS the structure. VERIFY requires evidence. WAIT_ALL requires all upstream tokens. The scheduler enforces it.

---

## Part 9: Implementation Status

**Already implemented in APXM:**

| Concept | Implementation |
|---------|----------------|
| AAM triple | `apxm-ais/src/aam.rs` |
| Beliefs (B) | `Beliefs` struct with HashMap |
| Goals (G) | `Goals` with priority queue, `GoalTree` |
| Capabilities (C) | `Capabilities` registry |
| Memory tiers | `apxm-ais/src/memory.rs` (STM/LTM/Episodic) |
| ScopePolicy | `apxm-core/src/types/aam.rs` |
| ScopeSpec | `apxm-core/src/types/aam.rs` |
| Per-node workspace | `nodes/{id}_{name}/` directory structure |
| Context assembly | ContextStack + profiles |

**To be implemented:**

| Concept | Status |
|---------|--------|
| Skill artifact (`.skill` files) | Design phase |
| Skill preconditions/effects | Not yet |
| Skill registry | REGISTER_CAPABILITY exists, needs extension |
| Multi-agent DELEGATE | Op exists, needs hierarchical AAM wiring |

---

## Summary

> An **agent** is `(Beliefs, Goals, Capabilities)`.
>
> **Capabilities** include both primitive ops and **skills**.
>
> A **skill** is a named APXM subgraph with declared preconditions and effects.
>
> Skill invocation creates a **child AAM scope** controlled by **ScopePolicy**.
>
> This gives each skill its **own isolated context** -- solving the context window problem.
>
> The **per-node workspace** is the physical manifestation of hierarchical AAM.
>
> **Multi-agent** = hierarchical AAM across agent boundaries via FLOW_CALL/DELEGATE.

The agent is not a skill graph. The agent HAS skill graphs as its capabilities. The agent IS the (B, G, C) triple that provides context and goals for those skills to execute in.

---

*This document is the conceptual foundation of APXM. For the formal specification, see [AAM](../pxm/aam.md). For implementation, see [runtime architecture](../implementation/architecture.md).*
