# AIS Language + ACP Agents: Full Investigation

*What exists, what's missing, and the full vision.*

---

## What Already Exists

### The .ais Language IS already designed

Looking at the examples, APXM already has a beautiful agent language:

```
// code_review_council.ais
agent SecurityReviewer {
    flow review(code: str) -> str {
        ask(prompt: "..." + code, backend: "apxm") -> findings
        return findings
    }
}

agent CodeReviewCouncil {
    memory {
        session: STM
        ltm: LTM
    }

    @entry flow review(code: str) -> str {
        SecurityReviewer.review(code) -> security_findings
        QualityReviewer.review(code) -> quality_findings
        PerformanceReviewer.review(code) -> perf_findings
        
        ask(prompt: "Synthesize: " + security_findings + ...) -> final_report
        return final_report
    }
}
```

**This is already the right abstraction.** Multiple agents in one file. Typed flows. Direct agent-to-agent calls (`SecurityReviewer.review(code)`). Memory declarations. `@entry` annotation. Parallel execution implicit from dataflow.

### The Gap: No Parser

The compiler (`apxm-driver/src/compiler/mod.rs`) explicitly rejects `.ais` files:

```rust
if matches!(ext, Some("mlir" | "ais")) {
    return Err(DriverError::Driver(
        "Graph-only compile path requires ApxmGraph JSON/binary input".to_string(),
    ));
}
```

The `.ais` files are **documentation/design artifacts** right now, not compilable source. The compiler only accepts `.apxm` JSON graphs.

**The .ais language is the authoring format that doesn't have a compiler yet.**

---

## What Claude Code and Codex Actually Are

Looking at `architect-implement-review.apxm`:

```json
{
  "op": "SPAWN_AGENT",
  "attributes": {
    "agent_name": "architect",
    "profile": "claude"
  }
}
```

```json
{
  "op": "SPAWN_AGENT",
  "attributes": {
    "agent_name": "coder",
    "profile": "codex"
  }
}
```

Claude and Codex are **ACP agents** — they run as separate processes that communicate via the ACP protocol. They're not LLM backends (like `backend: "claude"` in ASK ops). They're full agents with:
- Their own execution environment
- File system access (their workspace)
- The ability to write and run code
- Their own internal loop

In the APXM model:

```
APXM Runtime
├── LLM Backends (for ASK/THINK/REASON ops)
│   ├── "apxm"    → enterprise LLM gateway (Anthropic via X-Custom-Gateway-Key)
│   ├── "claude"  → Direct Anthropic API
│   └── "openai"  → OpenAI API
│
└── ACP Agents (for COMMUNICATE/SPAWN_AGENT/DELEGATE ops)
    ├── "claude"  → Claude Code (--print --permission-mode bypassPermissions)
    ├── "codex"   → OpenAI Codex CLI
    └── custom    → Any agent implementing ACP protocol
```

**The key distinction:**
- `ask(backend: "claude")` → single LLM call, returns text
- `COMMUNICATE(recipient: "architect", protocol: "acp")` → persistent agent with memory, tool use, multi-turn conversation

---

## The Full Vision: Every .ais IS an Agent

Right now:
```
.apxm JSON → compiler → .apxmobj → runtime
.ais         (no parser, not compilable)
```

The vision:
```
.ais source → AIS parser → ApxmGraph → compiler → .apxmobj → runtime
```

The `.ais` file declares one or more agents. Each agent is an AAM with flows as capabilities. The parser lowers it to the JSON graph IR. The compiler optimizes and produces an artifact. The runtime executes.

**Every `.apxm` execution IS an agent execution.** But currently you write the IR by hand (JSON). With an AIS parser, you write the agent language and the compiler generates the IR.

---

## How ACP Agents (Claude/Codex) Communicate

From the `architect-implement-review.apxm` example, the pattern is:

```
SPAWN_AGENT(agent_name: "architect", profile: "claude")
CONST_STR("Design the CHECKPOINT op...")
COMMUNICATE(recipient: "architect", protocol: "acp")  ← sends task, gets response
```

In AIS syntax this would be:
```
agent ArchitectImplementReview {
    @entry flow run() -> str {
        // Spawn persistent ACP agents
        spawn_agent(name: "architect", profile: "claude") -> arch_agent
        spawn_agent(name: "coder", profile: "codex") -> code_agent
        
        // Send task to architect (ACP call)
        communicate(
            to: arch_agent,
            message: "Design the CHECKPOINT op..."
        ) -> design
        
        // Send design to coder (ACP call)
        communicate(
            to: code_agent,
            message: "Implement this design:\n" + design
        ) -> implementation
        
        // Architect reviews coder's work (ACP call)
        communicate(
            to: arch_agent,
            message: "Review this implementation:\n" + implementation
        ) -> review
        
        return review
    }
}
```

**ACP agents are APXM's bridge to the coding world.** Claude Code has its own project context, file system, git integration. Codex has code execution. The APXM graph orchestrates them.

---

## The Agent Hierarchy

```
.ais Agent (APXM-native)
│   ├── Flows = capabilities (compiled AIS ops)
│   ├── Memory = STM/LTM/Episodic
│   └── Communicates with other .ais agents via FLOW_CALL
│
ACP Agent (external process)
│   ├── Claude Code (full coding agent)
│   ├── Codex (code execution + generation)
│   └── Any custom ACP-compliant agent
│   └── Communicates via COMMUNICATE op
│
LLM Backend (pure inference)
    ├── Used by ASK/THINK/REASON/PLAN ops
    └── Stateless: prompt in, text out
```

---

## What Needs to Be Built

### Priority 1: AIS Parser

This unlocks the entire authoring experience. The `.ais` files exist and are beautiful. They just need a parser.

**Grammar sketch:**
```
program     := agent_def+
agent_def   := 'agent' IDENT '{' memory_decl? flow_def+ '}'
memory_decl := 'memory' '{' (IDENT ':' tier)+ '}'
tier        := 'STM' | 'LTM' | 'Episodic'
flow_def    := '@entry'? 'flow' IDENT '(' params ')' '->' type '{' stmt+ '}'
stmt        := op_call '->' IDENT
             | agent_call '->' IDENT
             | 'return' IDENT
             | control_flow
op_call     := IDENT '(' named_args ')'
agent_call  := IDENT '.' IDENT '(' args ')'
```

**Output**: `Vec<AgentDef>` → lowered to `ApxmGraph` (existing format).

Multiple agents in one file compile to a multi-graph with named entry points.

### Priority 2: Agent Registry

When one `.ais` agent calls `SecurityReviewer.review(code)`, the runtime needs to find `SecurityReviewer`. This requires:

```rust
pub struct AgentRegistry {
    agents: HashMap<String, AgentDef>,  // name → compiled agent
    search_path: Vec<PathBuf>,          // where to look for .ais files
}
```

Resolution order:
1. Same file (multiple agents in one `.ais`)
2. Same directory
3. `~/.apxm/agents/`
4. Project `.apxm/agents/`

### Priority 3: Wire ACP agents as first-class

Right now `profile: "claude"` in SPAWN_AGENT is a string. It should resolve to an `AgentProfile` that knows:
- It's an ACP agent (not a flow-call agent)
- Its executable path (`claude --print --permission-mode bypassPermissions`)
- Its capability set (what tools it has)
- Its communication protocol

---

## The Unified Model

```
Agent = (B, G, C)

C = {
    // APXM-native capabilities
    primitive_ops: [ASK, THINK, REASON, ...],     // 40 AIS ops
    flows: Map<String, AISGraph>,                 // compiled skills
    
    // External capabilities  
    acp_agents: Map<String, ACPAgent>,            // Claude Code, Codex, etc.
    llm_backends: Map<String, LLMBackend>,        // for inference ops
}
```

An agent can:
1. Call primitive ops directly (`ask(...)`)
2. Call its own other flows (`self.validate(...)`)
3. Call another APXM agent's flow (`SecurityReviewer.review(code)`)
4. Spawn and communicate with an ACP agent (`spawn_agent(profile: "claude")`)
5. Use an LLM backend for inference (`ask(backend: "apxm", ...)`)

**All of these are capabilities in C.** The agent doesn't care if a capability is local, remote, compiled, or live — it calls it the same way.

---

## Summary

| What | Status |
|------|--------|
| `.ais` language design | ✅ Done — beautiful examples exist |
| `.ais` parser | ❌ Missing — compiler rejects `.ais` files |
| ACP agent integration (Claude/Codex) | ✅ Works via COMMUNICATE/SPAWN_AGENT |
| Agent registry (multi-agent resolution) | ❌ Missing |
| AgentProfile for ACP agents | ⚠️ Partial — `profile: "claude"` is a string |
| Unified capability model | ⚠️ Designed, not fully wired |

**The single highest-leverage thing to build: the AIS parser.**

Everything else (multi-agent, ACP integration, skill registry) flows naturally from having a first-class authoring language that compiles. Right now you're writing IR by hand. The examples already show what the language should be. Build the parser and the rest clicks into place.
