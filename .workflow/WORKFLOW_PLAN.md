# Agentic Workflow Orchestration Plan

## Vision

Build a **programmable workflow system** that defines coding tasks as directed graphs
and dispatches them to any coding agent (Claude Code, Codex, OpenClaw, Gemini, Cursor)
through the **Agent Client Protocol (ACP)** via **acpx flows**.

Inspired by `quantum-main/.ops/` (189 tickets, 12 teams, 8-stage pipeline, 89% completion
rate in one week) — but generalized, agent-portable, and graph-based.

---

## Architecture

```
                     YOU
                      |
                      v
          +-----------------------+
          |   Flow Definitions    |  .flow.ts files (TypeScript)
          |   (Workflow Graphs)   |  defineFlow({ nodes, edges, startAt })
          +-----------+-----------+
                      |
          +-----------v-----------+
          |        acpx           |  npm install -g acpx@latest
          |   (Orchestrator)      |  Headless CLI for ACP
          |                       |  Session mgmt, tracing, bundles
          +--+--------+--------+-+
             |        |        |
             v        v        v
        +---------+  +------+  +----------+
        | Claude  |  |Codex |  | OpenClaw |  ...+ Gemini, Cursor, Copilot
        | Code    |  | CLI  |  | Gateway  |
        | (ACP)   |  |(ACP) |  | (ACP)    |
        +---------+  +------+  +----------+
             |        |        |
             v        v        v
          YOUR CODEBASE (git worktrees, branches, PRs)
```

### Protocol Stack

| Layer | Technology | Role |
|-------|-----------|------|
| **Workflow Definition** | acpx flows (.flow.ts) | Graph structure, routing, I/O |
| **Orchestration** | acpx CLI | Session lifecycle, tracing, dispatch |
| **Agent Protocol** | ACP (JSON-RPC over stdio) | Standardized agent communication |
| **Agent Adapters** | claude-agent-acp, codex-acp | Translate ACP to native agent SDKs |
| **Coding Agents** | Claude Code, Codex, OpenClaw | The actual reasoning + code generation |
| **Tool Protocol** | MCP | Agent-to-tool connections |

---

## Phase 0: Environment Setup

### Prerequisites
```bash
# Node.js >= 22.12.0 (required by acpx)
conda install -c conda-forge nodejs=22

# acpx (the orchestrator)
npm install -g acpx@latest

# OpenClaw (optional — if using as an agent backend)
npm install -g openclaw@latest

# Claude Code (already installed — verify ACP adapter works)
npx @zed-industries/claude-agent-acp --help

# Codex CLI (if using)
npm install -g @openai/codex
```

### Configuration
```json
// ~/.acpx/config.json
{
  "defaultAgent": "claude",
  "defaultPermissions": "approve-all",
  "nonInteractivePermissions": "deny",
  "ttl": 300,
  "timeout": 1800,
  "format": "text"
}
```

### Verify Connectivity
```bash
# Test Claude Code via ACP
acpx claude exec 'echo "hello from Claude Code"'

# Test Codex via ACP
acpx codex exec 'echo "hello from Codex"'

# Test OpenClaw via ACP (requires running Gateway)
openclaw onboard --install-daemon
acpx openclaw exec 'echo "hello from OpenClaw"'
```

---

## Phase 1: Simple Linear Flows (Week 1)

### Goal: First working flow that dispatches to Claude Code

#### 1.1 — Hello World Flow
```typescript
// flows/hello.flow.ts
import { acp, compute, defineFlow, extractJsonObject } from "acpx/flows";

export default defineFlow({
  name: "hello-world",
  startAt: "greet",
  nodes: {
    greet: acp({
      prompt: ({ input }) =>
        `Return JSON: {"message": "Hello from Claude Code!", "input": "${(input as any).task}"}`,
      parse: (text) => extractJsonObject(text),
    }),
    done: compute({ run: ({ outputs }) => outputs.greet }),
  },
  edges: [{ from: "greet", to: "done" }],
});
```

Run: `acpx --approve-all flow run ./flows/hello.flow.ts --input-json '{"task":"test"}'`

#### 1.2 — Code Review Flow
```typescript
// flows/review.flow.ts
import { acp, shell, compute, defineFlow, extractJsonObject } from "acpx/flows";

export default defineFlow({
  name: "code-review",
  startAt: "get_diff",
  nodes: {
    get_diff: shell({
      exec: () => ({ command: "git", args: ["diff", "HEAD~1"] }),
      parse: (result) => ({ diff: result.stdout }),
    }),
    review: acp({
      prompt: ({ outputs }) => [
        "Review this git diff for bugs, style issues, and improvements.",
        "Return JSON: {\"issues\": [{\"severity\": \"P0|P1|P2\", \"file\": \"...\", \"line\": N, \"description\": \"...\"}], \"summary\": \"...\"}",
        "",
        "```diff",
        (outputs.get_diff as any).diff,
        "```",
      ].join("\n"),
      parse: (text) => extractJsonObject(text),
    }),
    report: compute({
      run: ({ outputs }) => {
        const review = outputs.review as any;
        return {
          total_issues: review.issues?.length ?? 0,
          p0_count: review.issues?.filter((i: any) => i.severity === "P0").length ?? 0,
          summary: review.summary,
          issues: review.issues,
        };
      },
    }),
  },
  edges: [
    { from: "get_diff", to: "review" },
    { from: "review", to: "report" },
  ],
});
```

#### 1.3 — Bug Fix Flow
```typescript
// flows/fix-bug.flow.ts
import { acp, shell, defineFlow, extractJsonObject } from "acpx/flows";

export default defineFlow({
  name: "fix-bug",
  permissions: { requiredMode: "approve-all", requireExplicitGrant: true,
    reason: "This flow modifies code and runs tests" },
  startAt: "understand",
  nodes: {
    understand: acp({
      prompt: ({ input }) => [
        `Analyze this bug and identify the root cause.`,
        `Bug description: ${(input as any).bug}`,
        `Return JSON: {"root_cause": "...", "files_to_modify": ["..."], "fix_strategy": "..."}`,
      ].join("\n"),
      parse: (text) => extractJsonObject(text),
    }),
    fix: acp({
      prompt: ({ outputs }) => [
        `Fix the bug based on this analysis:`,
        JSON.stringify(outputs.understand, null, 2),
        `Make the minimal changes needed. After fixing, return JSON:`,
        `{"files_modified": ["..."], "changes_summary": "..."}`,
      ].join("\n"),
      parse: (text) => extractJsonObject(text),
    }),
    test: shell({
      exec: () => ({ command: "pytest", args: ["tests/", "-x", "--tb=short"],
        allowNonZeroExit: true, timeoutMs: 120000 }),
      parse: (result) => ({
        passed: result.exitCode === 0,
        output: result.combinedOutput.slice(-2000),
      }),
    }),
    report: acp({
      prompt: ({ outputs }) => {
        const test = outputs.test as any;
        if (test.passed) {
          return `Tests passed. Summarize the fix. Return JSON: {"status": "fixed", "summary": "..."}`;
        }
        return [
          `Tests failed after the fix. Analyze and fix:`,
          test.output,
          `Return JSON: {"status": "needs_retry", "analysis": "..."}`,
        ].join("\n");
      },
      parse: (text) => extractJsonObject(text),
    }),
  },
  edges: [
    { from: "understand", to: "fix" },
    { from: "fix", to: "test" },
    { from: "test", to: "report" },
  ],
});
```

---

## Phase 2: Multi-Agent Flows (Week 2)

### Goal: Different agents for different tasks in one workflow

#### 2.1 — Multi-Agent Code Review (Claude reviews, Codex validates)
```typescript
// flows/multi-agent-review.flow.ts
import { acp, shell, compute, defineFlow, extractJsonObject } from "acpx/flows";

export default defineFlow({
  name: "multi-agent-review",
  startAt: "get_changes",
  nodes: {
    get_changes: shell({
      exec: () => ({ command: "git", args: ["diff", "HEAD~1"] }),
      parse: (result) => ({ diff: result.stdout }),
    }),

    // Claude Code does the deep review
    claude_review: acp({
      profile: "claude",
      prompt: ({ outputs }) => [
        "Do a thorough code review of this diff.",
        "Focus on: correctness, security, performance, maintainability.",
        "Return JSON: {\"issues\": [{\"severity\": \"P0|P1|P2\", \"description\": \"...\", \"suggestion\": \"...\"}], \"approval\": \"approve|request_changes\"}",
        "",
        (outputs.get_changes as any).diff,
      ].join("\n"),
      parse: (text) => extractJsonObject(text),
    }),

    // Codex validates the review findings
    codex_validate: acp({
      profile: "codex",
      session: { isolated: true },
      prompt: ({ outputs }) => [
        "A code reviewer found these issues. Validate each one.",
        "For each issue, determine if it's a real problem or a false positive.",
        "Return JSON: {\"validated\": [{\"original\": \"...\", \"valid\": true|false, \"reason\": \"...\"}]}",
        "",
        JSON.stringify((outputs.claude_review as any).issues, null, 2),
      ].join("\n"),
      parse: (text) => extractJsonObject(text),
    }),

    synthesize: compute({
      run: ({ outputs }) => {
        const review = outputs.claude_review as any;
        const validation = outputs.codex_validate as any;
        const confirmed = validation.validated?.filter((v: any) => v.valid) ?? [];
        return {
          approval: confirmed.length === 0 ? "approve" : review.approval,
          confirmed_issues: confirmed,
          false_positives: validation.validated?.filter((v: any) => !v.valid) ?? [],
        };
      },
    }),
  },
  edges: [
    { from: "get_changes", to: "claude_review" },
    { from: "claude_review", to: "codex_validate" },
    { from: "codex_validate", to: "synthesize" },
  ],
});
```

#### 2.2 — Implement + Review Pipeline (Inspired by .ops/ pipeline)
```typescript
// flows/implement-and-review.flow.ts
import { acp, shell, compute, checkpoint, defineFlow, extractJsonObject } from "acpx/flows";

const MAIN_SESSION = { handle: "main" };

export default defineFlow({
  name: "implement-and-review",
  permissions: { requiredMode: "approve-all", requireExplicitGrant: true,
    reason: "This flow creates branches, modifies code, and opens PRs" },
  startAt: "plan",
  nodes: {
    // Step 1: Claude plans the implementation
    plan: acp({
      session: MAIN_SESSION,
      prompt: ({ input }) => [
        `Plan the implementation for this task:`,
        `Task: ${(input as any).task}`,
        `Codebase: ${(input as any).codebase || "."}`,
        ``,
        `Return JSON: {"plan": "...", "files": ["..."], "complexity": "simple|medium|complex", "route": "implement"}`,
      ].join("\n"),
      parse: (text) => extractJsonObject(text),
    }),

    // Step 2: Claude implements (shared session = has context from plan)
    implement: acp({
      session: MAIN_SESSION,
      prompt: ({ outputs }) => [
        `Now implement the plan you just created.`,
        `Make all necessary code changes.`,
        `When done, return JSON: {"files_modified": ["..."], "summary": "...", "route": "test"}`,
      ].join("\n"),
      parse: (text) => extractJsonObject(text),
      timeoutMs: 600000,
    }),

    // Step 3: Run tests
    run_tests: shell({
      exec: ({ input }) => ({
        command: (input as any).test_command || "pytest",
        args: (input as any).test_args || ["tests/", "-x"],
        allowNonZeroExit: true,
        timeoutMs: 300000,
      }),
      parse: (result) => ({
        passed: result.exitCode === 0,
        output: result.combinedOutput.slice(-3000),
        route: result.exitCode === 0 ? "review" : "fix_tests",
      }),
    }),

    // Step 4a: Fix failing tests (loops back)
    fix_tests: acp({
      session: MAIN_SESSION,
      prompt: ({ outputs }) => [
        `Tests failed. Fix them:`,
        (outputs.run_tests as any).output,
        `Return JSON: {"fixed": true, "route": "test"}`,
      ].join("\n"),
      parse: (text) => extractJsonObject(text),
    }),

    // Step 4b: Codex does independent review
    review: acp({
      profile: "codex",
      session: { isolated: true },
      prompt: ({ outputs }) => [
        `Review these changes for correctness, security, and quality.`,
        `Implementation summary: ${(outputs.implement as any).summary}`,
        `Files modified: ${(outputs.implement as any).files_modified?.join(", ")}`,
        `Return JSON: {"approval": "approve|request_changes", "issues": [...], "route": "approve"|"request_changes"}`,
      ].join("\n"),
      parse: (text) => extractJsonObject(text),
    }),

    // Step 5: Human checkpoint for merge decision
    approve: checkpoint({
      summary: "Review complete. Awaiting human approval to merge.",
    }),

    // Step 6: Fix review findings
    address_feedback: acp({
      session: MAIN_SESSION,
      prompt: ({ outputs }) => [
        `Address these review findings:`,
        JSON.stringify((outputs.review as any).issues, null, 2),
        `Return JSON: {"addressed": true, "summary": "..."}`,
      ].join("\n"),
      parse: (text) => extractJsonObject(text),
    }),
  },
  edges: [
    { from: "plan", to: "implement" },
    { from: "implement", to: "run_tests" },
    { from: "run_tests", switch: {
      on: "$.route",
      cases: { review: "review", fix_tests: "fix_tests" },
    }},
    { from: "fix_tests", to: "run_tests" },
    { from: "review", switch: {
      on: "$.route",
      cases: { approve: "approve", request_changes: "address_feedback" },
    }},
    { from: "address_feedback", to: "run_tests" },
  ],
});
```

---

## Phase 3: Ticket-Driven Flows (Week 3)

### Goal: Bridge the .ops/ ticketing concept into acpx flows

#### 3.1 — Ticket Executor Flow
A flow that reads a ticket JSON file and executes it through an agent:

```typescript
// flows/ticket-execute.flow.ts
import { acp, shell, compute, defineFlow, extractJsonObject } from "acpx/flows";
import { readFileSync } from "fs";

export default defineFlow({
  name: "ticket-execute",
  permissions: { requiredMode: "approve-all", requireExplicitGrant: true,
    reason: "Executes ticket work: modifies code, creates branches, runs tests" },
  startAt: "load_ticket",
  nodes: {
    load_ticket: compute({
      run: ({ input }) => {
        const ticketPath = (input as any).ticket_path;
        const ticket = JSON.parse(readFileSync(ticketPath, "utf-8"));
        return { ticket, route: "execute" };
      },
    }),

    execute: acp({
      prompt: ({ outputs }) => {
        const t = (outputs.load_ticket as any).ticket;
        return [
          `# Ticket ${t.id}: ${t.title}`,
          `**Priority**: ${t.priority} | **Type**: ${t.type}`,
          `**Description**: ${t.description}`,
          ``,
          `## Actions`,
          ...(t.actions || []).map((a: string, i: number) => `${i + 1}. ${a}`),
          ``,
          `## References`,
          ...(t.refs || []).map((r: string) => `- ${r}`),
          ``,
          `Implement all actions. When done, return JSON:`,
          `{"files_modified": [...], "summary": "...", "status": "done"|"blocked"}`,
        ].join("\n");
      },
      parse: (text) => extractJsonObject(text),
      timeoutMs: 900000,
    }),

    verify: shell({
      exec: () => ({
        command: "pytest", args: ["tests/", "-x", "--tb=short"],
        allowNonZeroExit: true, timeoutMs: 180000,
      }),
      parse: (result) => ({
        tests_passed: result.exitCode === 0,
        output: result.combinedOutput.slice(-2000),
      }),
    }),

    finalize: compute({
      run: ({ outputs }) => ({
        ticket: (outputs.load_ticket as any).ticket.id,
        result: outputs.execute,
        tests: outputs.verify,
      }),
    }),
  },
  edges: [
    { from: "load_ticket", to: "execute" },
    { from: "execute", to: "verify" },
    { from: "verify", to: "finalize" },
  ],
});
```

Run: `acpx --approve-all flow run ./flows/ticket-execute.flow.ts --input-json '{"ticket_path": ".ops/tickets/T150.json"}'`

#### 3.2 — Sprint Flow (Batch Ticket Execution)
A wrapper script that feeds multiple tickets through the flow:

```python
#!/usr/bin/env python3
# scripts/run-sprint.py
"""Execute a sprint: run top unblocked tickets through acpx flows."""

import json, subprocess, sys
from pathlib import Path

OPS_DIR = Path(__file__).parent.parent / ".ops"

def load_tickets():
    tickets = []
    for f in sorted((OPS_DIR / "tickets").glob("T*.json")):
        tickets.append(json.loads(f.read_text()))
    return tickets

def get_unblocked(tickets):
    done_ids = {t["id"] for t in tickets if t["status"] in ("done", "cancelled")}
    unblocked = []
    for t in tickets:
        if t["status"] != "open":
            continue
        deps = set(t.get("depends_on", []))
        if deps <= done_ids:
            unblocked.append(t)
    return sorted(unblocked, key=lambda t: {"P0": 0, "P1": 1, "P2": 2, "P3": 3}[t["priority"]])

def run_ticket(ticket):
    ticket_path = OPS_DIR / "tickets" / f"{ticket['id']}.json"
    print(f"\n{'='*60}")
    print(f"  Executing {ticket['id']}: {ticket['title']} [{ticket['priority']}]")
    print(f"{'='*60}\n")
    result = subprocess.run(
        ["acpx", "--approve-all", "--format", "json", "--timeout", "900",
         "flow", "run", "./flows/ticket-execute.flow.ts",
         "--input-json", json.dumps({"ticket_path": str(ticket_path)})],
        capture_output=True, text=True
    )
    return result.returncode == 0

def main():
    tickets = load_tickets()
    unblocked = get_unblocked(tickets)
    max_tickets = int(sys.argv[1]) if len(sys.argv) > 1 else 3
    print(f"Sprint: {len(unblocked)} unblocked tickets, running top {max_tickets}")
    for ticket in unblocked[:max_tickets]:
        success = run_ticket(ticket)
        status = "done" if success else "blocked"
        print(f"  -> {ticket['id']}: {status}")

if __name__ == "__main__":
    main()
```

---

## Phase 4: Skill-Aware Agent Routing (Week 4)

### Goal: Route tasks to the best agent based on skill/complexity

```typescript
// flows/skill-router.flow.ts
import { acp, compute, defineFlow, extractJsonObject } from "acpx/flows";

// Map skills to best-suited agent profiles
const SKILL_AGENT_MAP: Record<string, string> = {
  // Claude Code: deep reasoning, architecture, complex refactoring
  "architecture": "claude",
  "compiler-craft": "claude",
  "mlir-engineering": "claude",
  "quantum-algorithms": "claude",
  // Codex: fast execution, straightforward implementations
  "systems-engineering": "codex",
  "documentation": "codex",
  "kernel-optimization": "codex",
  // Default
  "default": "claude",
};

export default defineFlow({
  name: "skill-router",
  startAt: "classify",
  nodes: {
    classify: acp({
      profile: "claude",
      session: { isolated: true },
      prompt: ({ input }) => [
        `Classify this task into ONE primary skill category:`,
        `Categories: ${Object.keys(SKILL_AGENT_MAP).join(", ")}`,
        `Task: ${(input as any).task}`,
        `Return JSON: {"skill": "...", "complexity": "simple|medium|complex", "route": "execute"}`,
      ].join("\n"),
      parse: (text) => extractJsonObject(text),
    }),

    route: compute({
      run: ({ outputs }) => {
        const { skill, complexity } = outputs.classify as any;
        const agent = SKILL_AGENT_MAP[skill] || SKILL_AGENT_MAP["default"];
        // Override: complex tasks always go to Claude
        const finalAgent = complexity === "complex" ? "claude" : agent;
        return { skill, complexity, agent: finalAgent, route: "execute" };
      },
    }),

    execute: acp({
      // Dynamic agent selection based on routing
      profile: "claude", // Note: acpx resolves profile at definition time,
                         // so dynamic routing requires the resolveAgent callback
                         // in FlowRunnerOptions. For now, default to claude.
      prompt: ({ input, outputs }) => [
        `Execute this task:`,
        `${(input as any).task}`,
        `Skill area: ${(outputs.route as any).skill}`,
        `Complexity: ${(outputs.route as any).complexity}`,
        `Return JSON: {"status": "done", "summary": "..."}`,
      ].join("\n"),
      parse: (text) => extractJsonObject(text),
      timeoutMs: 600000,
    }),
  },
  edges: [
    { from: "classify", to: "route" },
    { from: "route", to: "execute" },
  ],
});
```

> **Note**: True dynamic agent selection per-node requires extending acpx's
> `resolveAgent` callback or using separate named flows per agent.
> This is an area where we'd contribute to acpx upstream.

---

## Phase 5: Production Workflow System (Week 5-6)

### 5.1 — Full CI/CD Pipeline Flow (Mirrors .ops/ pipeline.json)

The full 8-stage pipeline from .ops/ expressed as an acpx flow:
`Assign -> Execute -> Branch -> PR -> Review -> Address -> Build -> Merge`

This would be the flagship flow (~500-800 lines), modeled after the
`pr-triage.flow.ts` example (1,385 lines, 24 nodes).

### 5.2 — Flow Library Structure
```
flows/
  core/
    hello.flow.ts              # Phase 1: smoke test
    review.flow.ts             # Phase 1: code review
    fix-bug.flow.ts            # Phase 1: bug fixing
  multi-agent/
    multi-review.flow.ts       # Phase 2: Claude + Codex review
    implement-review.flow.ts   # Phase 2: implement + review pipeline
  ticket/
    ticket-execute.flow.ts     # Phase 3: single ticket execution
    sprint.flow.ts             # Phase 3: batch sprint execution
  routing/
    skill-router.flow.ts       # Phase 4: skill-based routing
  pipeline/
    full-pipeline.flow.ts      # Phase 5: 8-stage CI/CD pipeline
  lib/
    ticket-loader.ts           # Shared: ticket JSON loading
    skill-config.ts            # Shared: skill/XP configuration
    agent-profiles.ts          # Shared: agent profile mappings
```

### 5.3 — Monitoring & Observability

acpx automatically produces run bundles at `~/.acpx/flows/runs/<runId>/`:
- `trace.ndjson` — every event (node start, ACP prompt, response, outcome)
- `manifest.json` — run metadata (status, timing, sessions)
- `artifacts/` — SHA256-addressed prompts and responses
- `sessions/` — full ACP conversation transcripts

These can be visualized with the `replay-viewer` React app from acpx examples.

---

## Phase 6: APXM Integration (Later)

Once the acpx flow system is proven, bridge to APXM:
- **ApxmGraph -> acpx flow compiler**: Translate APXM's graph IR to .flow.ts
- **DISPATCH_EXTERNAL AIS operation**: New operation for external agent dispatch
- **ExternalAgentCapability trait**: ACP client in APXM runtime
- **Metrics ingestion**: Import acpx trace.ndjson into APXM metrics

This is deferred — get the acpx system working first.

---

## Key Constraints & Risks

| Constraint | Impact | Mitigation |
|-----------|--------|------------|
| **No Node.js installed** | Can't run acpx yet | `conda install nodejs=22` |
| **Claude ACP adapter is third-party** | Reported stability issues | Test thoroughly; fallback to `claude -p` subprocess |
| **acpx flows are sequential** | No parallel node execution | Use named sessions for context sharing; parallel at sprint level |
| **acpx flows are experimental** | API may change | Pin acpx version; abstract flow creation |
| **Anthropic rejected native ACP** | Won't get first-party support | The adapter works; community-maintained |
| **Dynamic agent routing** | `profile` is static per node definition | Contribute dynamic resolution upstream; workaround with separate flows |

---

## Quick Start Checklist

- [ ] Install Node.js 22+: `conda install -c conda-forge nodejs=22`
- [ ] Install acpx: `npm install -g acpx@latest`
- [ ] Create `~/.acpx/config.json` with defaults
- [ ] Test: `acpx claude exec 'say hello'`
- [ ] Create `flows/` directory
- [ ] Write `flows/hello.flow.ts` (Phase 1.1)
- [ ] Run: `acpx --approve-all flow run ./flows/hello.flow.ts --input-json '{"task":"test"}'`
- [ ] Write `flows/review.flow.ts` (Phase 1.2)
- [ ] Write multi-agent flow (Phase 2.1)
- [ ] Write ticket executor flow (Phase 3.1)
- [ ] Build sprint runner script (Phase 3.2)

---

## References

- **acpx**: https://github.com/openclaw/acpx (1.8k stars, MIT, TypeScript)
- **ACP spec**: https://github.com/agentclientprotocol/agent-client-protocol (2.6k stars)
- **Claude ACP adapter**: https://github.com/zed-industries/claude-agent-acp
- **Codex ACP adapter**: https://github.com/cola-io/codex-acp
- **OpenClaw**: https://github.com/openclaw/openclaw (341k stars)
- **ClawHub**: https://github.com/openclaw/clawhub (skill marketplace)
- **Oracle Agent Spec**: https://github.com/oracle/agent-spec (portable graph IR)
- **quantum-main/.ops/**: The inspiration system (189 tickets, 12 teams, 8-stage pipeline)
