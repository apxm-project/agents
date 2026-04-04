# APXM Canonical IR Design

*Ultrathink: the complete compilation pipeline with a proper IR format.*

---

## The Vision

```
                    ┌─────────────────────────────────────────────────┐
                    │              FRONTENDS (authoring)              │
                    ├─────────────────┬───────────────────────────────┤
                    │   .ais (DSL)    │  Python (agentmate)           │
                    │                 │  Rust (WorkflowBuilder)       │
                    │  agent Foo {    │  @compile                     │
                    │    flow bar()   │  def research(g):             │
                    │    ...          │    g.ask(...)                 │
                    │  }              │                               │
                    └────────┬────────┴───────────────┬───────────────┘
                             │                        │
                             ▼                        ▼
                    ┌─────────────────────────────────────────────────┐
                    │           CANONICAL IR (.air)                   │
                    │         Agent Intermediate Representation       │
                    │                                                 │
                    │  ; agent: CodeReviewCouncil                     │
                    │  ; flow: review                                 │
                    │  ; entry: true                                  │
                    │                                                 │
                    │  %0 = ais.const_str "Review this code"          │
                    │  %1 = ais.ask %0 {backend = "claude"}           │
                    │  %2 = ais.verify %1 {claim = %1}                │
                    │  ais.return %2                                  │
                    │                                                 │
                    └────────────────────┬────────────────────────────┘
                                         │
                                         ▼
                    ┌─────────────────────────────────────────────────┐
                    │              MLIR COMPILER PASSES               │
                    │                                                 │
                    │  • FuseReasoning                                │
                    │  • OptimizePrompts (LLM-augmented)              │
                    │  • DecomposeComplex                             │
                    │  • ScheduleParallel                             │
                    │  • ProfileGuided                                │
                    │                                                 │
                    └────────────────────┬────────────────────────────┘
                                         │
                                         ▼
                    ┌─────────────────────────────────────────────────┐
                    │           ARTIFACT (.apxmobj)                   │
                    │                                                 │
                    │  Magic: b"APXM"                                 │
                    │  Version: 1                                     │
                    │  BLAKE3 hash                                    │
                    │  Serialized ExecutionDAG (bincode)              │
                    │                                                 │
                    └────────────────────┬────────────────────────────┘
                                         │
                                         ▼
                    ┌─────────────────────────────────────────────────┐
                    │               RUNTIME                           │
                    │                                                 │
                    │  • Load artifact                                │
                    │  • Instantiate AAM (Beliefs, Goals, Caps)       │
                    │  • Dataflow scheduler                           │
                    │  • LLM backends + Capability executors          │
                    │                                                 │
                    └─────────────────────────────────────────────────┘
```

---

## Why JSON is Wrong for the IR

The current `.apxm` JSON format is problematic:

```json
{
  "nodes": [
    {"id": 1, "name": "ask1", "op": "ASK", "attributes": {"template_str": "..."}}
  ],
  "edges": [{"from": 1, "to": 2, "dependency": "Data"}]
}
```

**Problems:**
1. **Verbose** — 200+ lines for a 20-node graph
2. **No comments** — can't annotate IR for debugging
3. **No types** — everything is stringly-typed
4. **Slow to parse** — JSON parsing is O(n) with allocations
5. **Not diffable** — JSON diffs are noisy
6. **Not a real IR** — it's a data format pretending to be an IR

**Compare to LLVM IR (.ll):**
```llvm
; Function: add
define i32 @add(i32 %a, i32 %b) {
entry:
  %sum = add i32 %a, %b
  ret i32 %sum
}
```

This is:
- Human-readable
- Has comments (`;`)
- Has types (`i32`)
- Diffable
- Fast to parse (structured grammar)
- A REAL intermediate representation

---

## The Canonical IR: `.air` (Agent IR)

**Proposal:** A text-based IR format that is:
- SSA-form (like LLVM IR)
- Human-readable and writable
- Has comments
- Has types
- Canonical (any frontend produces identical IR for the same semantics)
- Fast to parse (structured grammar, not JSON)

### Syntax Design

```llvm
; ═══════════════════════════════════════════════════════════════════
; Agent: CodeReviewCouncil
; Source: code_review_council.ais
; ═══════════════════════════════════════════════════════════════════

agent @CodeReviewCouncil {
  memory {
    session: stm
    ltm: ltm
  }
}

; ───────────────────────────────────────────────────────────────────
; Flow: review
; Entry: true
; Params: (code: str) -> str
; ───────────────────────────────────────────────────────────────────

flow @CodeReviewCouncil::review(%code: str) -> str {
entry:
  ; Phase 1: Parallel specialist reviews
  %security = ais.ask "Security review: {0}" (%code) {backend = "claude"}
  %quality = ais.ask "Quality review: {0}" (%code) {backend = "claude"}
  %perf = ais.ask "Perf review: {0}" (%code) {backend = "claude"}
  
  ; Phase 2: Synchronize
  %all = ais.wait_all [%security, %quality, %perf]
  
  ; Phase 3: Synthesize
  %merged = ais.merge [%security, %quality, %perf] {strategy = "concat"}
  %report = ais.think "Synthesize findings: {0}" (%merged) {budget = 4096}
  
  ; Phase 4: Store and return
  ais.umem "review_result" (%report) {tier = "ltm"}
  ais.return %report
}
```

### Key Properties

1. **SSA form** — each value defined exactly once (`%name = ...`)
2. **Typed** — `%code: str`, `-> str`
3. **Blocks** — `entry:`, can have `branch:`, `catch:`, etc.
4. **Agent metadata** — `agent @Name { memory {...} }`
5. **Flow signatures** — `flow @Agent::name(params) -> type`
6. **Comments** — `;` for line comments
7. **Attributes** — `{backend = "claude", budget = 4096}`
8. **References** — `%name` for SSA values, `@Name` for agents/flows

---

## The Full Format Stack

| Layer | Extension | Format | Purpose |
|-------|-----------|--------|---------|
| **Source** | `.ais` | Text DSL | Human authoring |
| **Source** | `.py` | Python | Programmatic authoring (agentmate) |
| **Source** | `.rs` | Rust | Programmatic authoring (WorkflowBuilder) |
| **Canonical IR** | `.air` | Text IR | Compiler input, debugging, diffing |
| **Binary IR** | `.airb` | FlatBuffers | Fast toolchain interchange |
| **Artifact** | `.apxmobj` | Binary | Compiled, ready to execute |

### Compilation Paths

```
.ais ──────────┐
               │
.py (agentmate)├──▶ .air (canonical) ──▶ MLIR passes ──▶ .apxmobj
               │
.rs (builder) ─┘
```

Every frontend emits `.air`. The compiler consumes `.air`. One canonical format.

---

## Why This Matters

### 1. Debugging

With JSON IR:
```
Error at node 7
```

With `.air`:
```
error: type mismatch at line 23
  %report = ais.think "..." (%merged)
            ^~~~~~~~~~
  expected: str, got: list<str>
```

### 2. Diffing

Git diff of JSON IR:
```diff
-  {"id": 7, "name": "report", "op": "THINK", "attributes": {"template_str": "...", "budget": 4096}}
+  {"id": 7, "name": "report", "op": "THINK", "attributes": {"template_str": "...", "budget": 8192}}
```

Git diff of `.air`:
```diff
-  %report = ais.think "..." (%merged) {budget = 4096}
+  %report = ais.think "..." (%merged) {budget = 8192}
```

### 3. Optimization Visibility

```
; ═══════════════════════════════════════════════════════════════════
; Pass: FuseReasoning
; Fused nodes: [ask_1, reason_2] -> fused_think_1
; ═══════════════════════════════════════════════════════════════════

; Before:
;   %1 = ais.ask "..." (...)
;   %2 = ais.reason "..." (%1)

; After:
%fused = ais.think "..." (...) {fused_from = ["ask_1", "reason_2"]}
```

### 4. Hand-editing

Sometimes you want to tweak the IR directly. With JSON, it's error-prone. With `.air`, it's just editing text:

```llvm
; Add a verification step
%verified = ais.verify %report {claim = %report, evidence = %code}
ais.return %verified
```

---

## Implementation Path

### Phase 1: Define `.air` grammar
- EBNF grammar for the IR
- Lexer + parser (can reuse MLIR infra or write standalone)
- Round-trip: `.air` → AST → `.air`

### Phase 2: Emit `.air` from all frontends
- `.ais` parser emits `.air` (not JSON)
- agentmate `GraphRecorder.to_air()` emits `.air`
- Rust `WorkflowBuilder.emit_air()` emits `.air`

### Phase 3: Compiler consumes `.air`
- Replace JSON graph loading with `.air` parsing
- Lower `.air` → MLIR directly (or via existing ApxmGraph as intermediate)

### Phase 4: Binary IR for toolchains
- Define `.airb` FlatBuffers schema
- Fast serialization for large graphs
- `.air` ↔ `.airb` conversion tools

---

## Relationship to MLIR

APXM already uses MLIR internally. The question: **why not just use MLIR textual IR directly?**

Answer: MLIR IR is for compiler internals. `.air` is for humans and tooling.

```
.air (human-facing IR)
    │
    ▼
MLIR (compiler-internal IR)
    │
    ▼
Optimized MLIR
    │
    ▼
.apxmobj (artifact)
```

`.air` is simpler than MLIR textual format. It's agent-specific. It doesn't expose MLIR's full generality (regions, nested ops, etc.). It's the "user-facing IR" like LLVM's `.ll` vs internal MachineIR.

---

## The Name: `.air`

- **A**gent **I**ntermediate **R**epresentation
- Short, memorable
- Doesn't conflict with existing extensions
- Mirrors `.ll` (LLVM IR), `.wat` (WebAssembly text)

---

## Summary

| What | Current | Proposed |
|------|---------|----------|
| Source | `.ais` / Python / Rust | Same |
| IR format | `.apxm` JSON | `.air` text IR |
| IR properties | Verbose, no comments, slow | SSA, typed, fast, diffable |
| Binary IR | None | `.airb` (FlatBuffers) |
| Artifact | `.apxmobj` | Same |

**The `.air` format is the missing piece.** It's the canonical representation that all frontends emit and the compiler consumes. It's human-readable, debuggable, and professional.

JSON was fine for prototyping. `.air` is the production format.
