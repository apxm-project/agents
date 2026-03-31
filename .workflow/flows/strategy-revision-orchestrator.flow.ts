/**
 * Strategy Docs Revision — Orchestrator Flow (v2)
 *
 * Coordinates parallel revision of 9 strategy documents across 10 agents.
 * Corrected dependency analysis yields 5 phases (not 3):
 *
 * ╔══════════════════════════════════════════════════════════════════════════════╗
 * ║                                                                            ║
 * ║  DEPENDENCY GRAPH (from cross-reference analysis):                         ║
 * ║                                                                            ║
 * ║    01-VISION ────────────┐                                                 ║
 * ║                          │                                                 ║
 * ║    02-OPPORTUNITIES ─────┤──── 06-GAP-ANALYSIS                             ║
 * ║                          │                                                 ║
 * ║    04-ARCHITECTURE ──────┤──── 07-ACPX-ABSORPTION ───┐                     ║
 * ║                          │                            ├── 03-PLAN          ║
 * ║    05-PATENT ────────────┘──── 08-OPTIMIZATION ───────┘       │            ║
 * ║                                                               │            ║
 * ║    Phase 1 (4 parallel)    Phase 2 (3 parallel)    Phase 3    │            ║
 * ║                                                               v            ║
 * ║                                                           README           ║
 * ║                                                           Phase 4          ║
 * ║                                                                            ║
 * ╚══════════════════════════════════════════════════════════════════════════════╝
 *
 * PHASES:
 *   Phase 0: Analysis         — Agent 1 (Claude) scans all docs, builds glossary
 *   Phase 1: Independent      — Agents 2-5 (Codex) revise 01, 02, 04, 05 in PARALLEL
 *   Phase 2: First-dependent  — Agents 6-8 (Claude) revise 06, 07, 08 in PARALLEL
 *   Phase 3: Plan             — Agent 9 (Claude) revises 03 (depends on 07+08)
 *   Phase 4: Final            — Agent 10 (Claude) revises README + final validation
 *
 * AGENT ASSIGNMENTS:
 *   Agent  1: Coordinator       Claude Code  — deep cross-doc analysis
 *   Agent  2: 01-VISION         Codex        — fast independent edits
 *   Agent  3: 02-OPPORTUNITIES  Codex        — fast independent edits
 *   Agent  4: 04-ARCHITECTURE   Codex        — fast independent edits
 *   Agent  5: 05-PATENT         Codex        — fast independent edits
 *   Agent  6: 06-GAP            Claude Code  — cross-doc reasoning (refs 01,02,04)
 *   Agent  7: 07-ACPX           Claude Code  — cross-doc reasoning (refs 01,04)
 *   Agent  8: 08-OPTIMIZATION   Claude Code  — cross-doc reasoning (refs 04,05)
 *   Agent  9: 03-PLAN           Claude Code  — depends on 07+08 outputs
 *   Agent 10: README            Claude Code  — synthesis of all 8 docs
 *
 * SUB-FLOW: Each doc revision runs strategy-revision-doc.flow.ts (10 nodes, 2 loops)
 * TOTAL NODES: 19 orchestrator + 9×10 sub-flow = 109 nodes
 * TOTAL EDGES: 20 orchestrator + 9×11 sub-flow = 119 edges
 * RETRY LOOPS: 18 (2 per doc) + 1 final = 19
 *
 * Usage:
 *   acpx flow run ./strategy-revision-orchestrator.flow.ts \
 *     --input-json '{"docsPath":"$HOME/projects/agents/docs/strategy"}' \
 *     --approve-all
 */
import { defineFlow, acp, compute, shell, checkpoint, extractJsonObject } from "acpx/flows";

// ─── Types ───────────────────────────────────────────────────────────────────

type OrchestratorInput = {
  docsPath: string;
  projectRoot?: string;
  flowsDir?: string;
};

type AnalysisResult = {
  inconsistencies: { doc: string; issue: string; severity: string }[];
  crossRefs: { from: string; to: string; type: string }[];
  termDefinitions: { term: string; doc: string; definition: string }[];
  plan: string;
};

type DocTask = {
  file: string;
  agentId: number;
  profile: "codex" | "claude";
  phase: "independent" | "dependent" | "plan" | "final";
  sessionName: string;
};

// ─── Task Definitions (corrected dependency order) ──────────────────────────

const PHASE1_TASKS: DocTask[] = [
  { file: "01-VISION.md",                   agentId: 2,  profile: "codex",  phase: "independent", sessionName: "doc-01" },
  { file: "02-OPPORTUNITIES.md",            agentId: 3,  profile: "codex",  phase: "independent", sessionName: "doc-02" },
  { file: "04-ARCHITECTURE-INTEGRATION.md", agentId: 4,  profile: "codex",  phase: "independent", sessionName: "doc-04" },
  { file: "05-PATENT-ALIGNMENT.md",         agentId: 5,  profile: "codex",  phase: "independent", sessionName: "doc-05" },
];

const PHASE2_TASKS: DocTask[] = [
  // These depend on Phase 1 but NOT on each other → parallel
  { file: "06-GAP-ANALYSIS.md",             agentId: 6,  profile: "claude", phase: "dependent",   sessionName: "doc-06" },
  { file: "07-ACPX-ABSORPTION.md",          agentId: 7,  profile: "claude", phase: "dependent",   sessionName: "doc-07" },
  { file: "08-OPTIMIZATION-TARGETS.md",     agentId: 8,  profile: "claude", phase: "dependent",   sessionName: "doc-08" },
];

const PHASE3_TASK: DocTask = {
  // 03-PLAN depends on 07 and 08 → must wait for Phase 2
  file: "03-PLAN.md",
  agentId: 9,
  profile: "claude",
  phase: "plan",
  sessionName: "doc-03",
};

const PHASE4_TASK: DocTask = {
  file: "README.md",
  agentId: 10,
  profile: "claude",
  phase: "final",
  sessionName: "doc-readme",
};

// ─── Helpers ─────────────────────────────────────────────────────────────────

function buildSpawnCommand(
  tasks: DocTask[],
  flowsDir: string,
  docsPath: string,
  projectRoot: string,
  analysisJson: string,
  phaseContext: string,
): string {
  const commands = tasks.map(task => {
    const inputJson = JSON.stringify({
      file: task.file,
      docsPath,
      projectRoot,
      agentId: task.agentId,
      phase: task.phase,
      profile: task.profile,
      analysisJson,
      phaseContext,
      maxRetries: 2,
    });
    const escaped = inputJson.replace(/'/g, "'\\''");
    return `( acpx flow run "${flowsDir}/strategy-revision-doc.flow.ts" -s "${task.sessionName}" --input-json '${escaped}' --approve-all ) &`;
  });
  return commands.join("\n") + "\nwait";
}

// ═══════════════════════════════════════════════════════════════════════════════

export default defineFlow({
  name: "strategy-revision-orchestrator",
  startAt: "load_config",

  run: {
    title: () => "Strategy Docs Revision — 10-Agent Orchestrator (v2)",
  },

  permissions: {
    requiredMode: "approve-all",
    reason: "Spawns parallel sub-flows, creates worktrees, merges branches",
  },

  nodes: {
    // ═════════════════════════════════════════════════════════════════════════
    // PHASE 0: ANALYSIS (Agent 1 — Coordinator, Claude Code)
    // ═════════════════════════════════════════════════════════════════════════

    load_config: compute({
      run: ({ input }) => {
        const inp = input as OrchestratorInput;
        return {
          docsPath: inp.docsPath || "$HOME/projects/agents/docs/strategy",
          projectRoot: inp.projectRoot || "$HOME/projects/agents",
          flowsDir: inp.flowsDir || process.cwd(),
        };
      },
    }),

    scan_docs: shell({
      statusDetail: "Scanning 9 strategy documents...",
      exec: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return {
          command: "bash",
          args: ["-c", [
            `cd "${docsPath}"`,
            `echo "=== INVENTORY ==="`,
            `for f in *.md; do printf "%-40s %5d lines\\n" "$f" "$(wc -l < "$f")"; done`,
            `echo ""`,
            `echo "=== CROSS-REFERENCES ==="`,
            `for f in *.md; do refs=$(grep -oP '\\[.*?\\]\\(\\K[^)]+\\.md' "$f" 2>/dev/null | sort -u | tr '\\n' ' '); [ -n "$refs" ] && echo "$f -> $refs"; done`,
          ].join(" && ")],
          shell: false,
        };
      },
      parse: (result) => ({ inventory: result.stdout }),
    }),

    analyze_all: acp({
      profile: "claude",
      session: { handle: "coordinator", isolated: true },
      statusDetail: "Agent 1 (Claude): Deep cross-reference analysis...",
      timeoutMs: 15 * 60_000,
      cwd: ({ outputs }) => (outputs.load_config as { projectRoot: string }).projectRoot,

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const { inventory } = outputs.scan_docs as { inventory: string };

        return [
          `Read ALL 9 docs at ${docsPath}/. Using a team of 5 ultrathink agents:`,
          "A=audit numbers, B=map cross-refs, C=build glossary, D=validate timelines, E=check architecture consistency.",
          "",
          "```",
          inventory,
          "```",
          "",
          "Known issues to check:",
          "- AcpCapability size: '~800' vs '~500-800' lines",
          "- 02-OPPORTUNITIES describes TypeScript bridge (pre-absorption)",
          "- 04-ARCHITECTURE lists tools/acpx-to-apxm/ (should not exist post-absorption)",
          "- Gap A6: P0 in 06-GAP but missing from 03-PLAN phases",
          "- 01-VISION scope: 'Integration Strategy' not 'Absorption'",
          "- 03-PLAN text says P1/P2 parallel but Gantt shows sequential",
          "- New code estimate: '~2K' (03-PLAN) vs '~1,430 lines' (08-OPTIMIZATION)",
          "",
          'Output: `{ "inconsistencies": [{"doc","issue","severity"}], "crossRefs": [{"from","to","type"}], "termDefinitions": [{"term","doc","definition"}], "plan": "..." }`',
        ].join("\n");
      },
      parse: (text) => {
        const parsed = extractJsonObject(text) as AnalysisResult;
        return {
          inconsistencies: parsed.inconsistencies || [],
          crossRefs: parsed.crossRefs || [],
          termDefinitions: parsed.termDefinitions || [],
          plan: parsed.plan || "",
          analysisJson: JSON.stringify(parsed),
        };
      },
    }),

    plan_tasks: compute({
      run: ({ outputs }) => {
        const analysis = outputs.analyze_all as AnalysisResult & { analysisJson: string };
        const high = analysis.inconsistencies.filter(i => i.severity === "high");
        return {
          analysisJson: analysis.analysisJson,
          summary: {
            issues: analysis.inconsistencies.length,
            highSeverity: high.length,
            crossRefs: analysis.crossRefs.length,
            terms: analysis.termDefinitions.length,
          },
          phases: {
            "Phase 1 (parallel, Codex)": PHASE1_TASKS.map(t => t.file),
            "Phase 2 (parallel, Claude)": PHASE2_TASKS.map(t => t.file),
            "Phase 3 (sequential, Claude)": [PHASE3_TASK.file],
            "Phase 4 (sequential, Claude)": [PHASE4_TASK.file],
          },
          plan: analysis.plan,
        };
      },
    }),

    approve_plan: checkpoint({
      summary: "Review analysis and task plan. Phases: 4 parallel Codex → 3 parallel Claude → 1 Claude (03-PLAN) → 1 Claude (README) + final validation.",
    }),

    // ═════════════════════════════════════════════════════════════════════════
    // PHASE 1: 4 PARALLEL sub-flows (Codex — fast independent edits)
    //   01-VISION, 02-OPPORTUNITIES, 04-ARCHITECTURE, 05-PATENT
    // ═════════════════════════════════════════════════════════════════════════

    spawn_phase1: shell({
      statusDetail: "Phase 1: Spawning 4 parallel Codex agents...",
      timeoutMs: 30 * 60_000,
      exec: ({ outputs }) => {
        const { docsPath, projectRoot, flowsDir } = outputs.load_config as {
          docsPath: string; projectRoot: string; flowsDir: string;
        };
        const { analysisJson } = outputs.analyze_all as { analysisJson: string };
        return {
          command: "bash",
          args: ["-c", buildSpawnCommand(
            PHASE1_TASKS, flowsDir, docsPath, projectRoot, analysisJson,
            "",
          )],
          shell: false,
        };
      },
      parse: (result) => ({
        phase: 1,
        status: result.exitCode === 0 ? "complete" : "partial",
        stdout: result.stdout.slice(-2000),
      }),
    }),

    phase1_gate: compute({
      run: ({ outputs }) => {
        const r = outputs.spawn_phase1 as { status: string };
        return { phase1: r.status, docsRevised: PHASE1_TASKS.map(t => t.file) };
      },
    }),

    // ═════════════════════════════════════════════════════════════════════════
    // PHASE 2: 3 PARALLEL sub-flows (Claude — cross-doc reasoning)
    //   06-GAP, 07-ACPX-ABSORPTION, 08-OPTIMIZATION
    //   These depend on Phase 1 but NOT on each other.
    // ═════════════════════════════════════════════════════════════════════════

    spawn_phase2: shell({
      statusDetail: "Phase 2: Spawning 3 parallel Claude agents...",
      timeoutMs: 30 * 60_000,
      exec: ({ outputs }) => {
        const { docsPath, projectRoot, flowsDir } = outputs.load_config as {
          docsPath: string; projectRoot: string; flowsDir: string;
        };
        const { analysisJson } = outputs.analyze_all as { analysisJson: string };
        const phaseContext = [
          "Phase 1 COMPLETE. These docs were revised: 01-VISION.md, 02-OPPORTUNITIES.md, 04-ARCHITECTURE-INTEGRATION.md, 05-PATENT-ALIGNMENT.md.",
          "Read the ACTUAL files (not your memory) to see what changed.",
          "Ensure your revisions are consistent with the updated Phase 1 docs.",
        ].join(" ");
        return {
          command: "bash",
          args: ["-c", buildSpawnCommand(
            PHASE2_TASKS, flowsDir, docsPath, projectRoot, analysisJson,
            phaseContext,
          )],
          shell: false,
        };
      },
      parse: (result) => ({
        phase: 2,
        status: result.exitCode === 0 ? "complete" : "partial",
        stdout: result.stdout.slice(-2000),
      }),
    }),

    phase2_gate: compute({
      run: ({ outputs }) => {
        const r = outputs.spawn_phase2 as { status: string };
        return { phase2: r.status, docsRevised: PHASE2_TASKS.map(t => t.file) };
      },
    }),

    // ═════════════════════════════════════════════════════════════════════════
    // PHASE 3: 03-PLAN.md (Agent 9, Claude — depends on 07+08)
    //   This CANNOT run in parallel with Phase 2 because 03-PLAN.md
    //   explicitly references 07-ACPX-ABSORPTION.md and 08-OPTIMIZATION-TARGETS.md.
    // ═════════════════════════════════════════════════════════════════════════

    spawn_phase3: shell({
      statusDetail: "Phase 3: Revising 03-PLAN.md (depends on 07+08)...",
      timeoutMs: 15 * 60_000,
      exec: ({ outputs }) => {
        const { docsPath, projectRoot, flowsDir } = outputs.load_config as {
          docsPath: string; projectRoot: string; flowsDir: string;
        };
        const { analysisJson } = outputs.analyze_all as { analysisJson: string };
        const phaseContext = [
          "Phases 1+2 COMPLETE. ALL docs except 03-PLAN.md and README.md are revised.",
          "03-PLAN.md directly references 07-ACPX-ABSORPTION.md and 08-OPTIMIZATION-TARGETS.md.",
          "Read the ACTUAL revised versions of 07 and 08 before editing 03-PLAN.",
          "Key things to check:",
          "- Phase 2 description must match 07-ACPX-ABSORPTION exactly",
          "- Phase 4 optimization targets must match 08-OPTIMIZATION-TARGETS exactly",
          "- 'What Changed From v1' table must be consistent with both docs",
          "- Remove any duplication with 07 (the Phase 2 implementation plan appears in both)",
          "- Reconcile new code estimate: align with 08's ~1,430 lines figure",
        ].join(" ");

        const inputJson = JSON.stringify({
          file: PHASE3_TASK.file,
          docsPath,
          projectRoot,
          agentId: PHASE3_TASK.agentId,
          phase: PHASE3_TASK.phase,
          profile: PHASE3_TASK.profile,
          analysisJson,
          phaseContext,
          maxRetries: 2,
        });
        const escaped = inputJson.replace(/'/g, "'\\''");
        return {
          command: "bash",
          args: ["-c", `acpx flow run "${flowsDir}/strategy-revision-doc.flow.ts" -s "${PHASE3_TASK.sessionName}" --input-json '${escaped}' --approve-all`],
          shell: false,
        };
      },
      parse: (result) => ({
        phase: 3,
        status: result.exitCode === 0 ? "complete" : "failed",
        stdout: result.stdout.slice(-2000),
      }),
    }),

    phase3_gate: compute({
      run: ({ outputs }) => {
        const r = outputs.spawn_phase3 as { status: string };
        return { phase3: r.status, docsRevised: [PHASE3_TASK.file] };
      },
    }),

    // ═════════════════════════════════════════════════════════════════════════
    // PHASE 4: README + FINAL VALIDATION (Agent 10, Claude)
    //   README depends on ALL 8 docs. Then a final cross-doc validation
    //   with a retry loop to catch any remaining inconsistencies.
    // ═════════════════════════════════════════════════════════════════════════

    spawn_readme: shell({
      statusDetail: "Phase 4: Revising README.md (depends on all 8 docs)...",
      timeoutMs: 15 * 60_000,
      exec: ({ outputs }) => {
        const { docsPath, projectRoot, flowsDir } = outputs.load_config as {
          docsPath: string; projectRoot: string; flowsDir: string;
        };
        const { analysisJson } = outputs.analyze_all as { analysisJson: string };
        const phaseContext = [
          "ALL 8 docs are now revised (Phases 1-3 complete).",
          "README.md is the INDEX document. It must:",
          "1. Accurately link and summarize each of the 8 docs",
          "2. Use CANONICAL numbers from the revised docs",
          "3. Present a coherent narrative across all 8 docs",
          "4. Key Numbers table must match revised docs EXACTLY",
          "5. One-Paragraph Summary must reflect all revisions",
          "Read ALL 8 revised documents before editing README.md.",
        ].join(" ");

        const inputJson = JSON.stringify({
          file: PHASE4_TASK.file,
          docsPath,
          projectRoot,
          agentId: PHASE4_TASK.agentId,
          phase: PHASE4_TASK.phase,
          profile: PHASE4_TASK.profile,
          analysisJson,
          phaseContext,
          maxRetries: 2,
        });
        const escaped = inputJson.replace(/'/g, "'\\''");
        return {
          command: "bash",
          args: ["-c", `acpx flow run "${flowsDir}/strategy-revision-doc.flow.ts" -s "${PHASE4_TASK.sessionName}" --input-json '${escaped}' --approve-all`],
          shell: false,
        };
      },
      parse: (result) => ({
        phase: 4,
        status: result.exitCode === 0 ? "complete" : "failed",
        stdout: result.stdout.slice(-2000),
      }),
    }),

    // Final cross-document validation by Coordinator (Agent 1)
    final_validation: acp({
      profile: "claude",
      session: { handle: "coordinator" },
      statusDetail: "Agent 1: FINAL cross-document validation...",
      timeoutMs: 15 * 60_000,
      cwd: ({ outputs }) => (outputs.load_config as { projectRoot: string }).projectRoot,

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };

        return [
          `Read ALL 9 docs at ${docsPath}/. Using a team of 5 ultrathink agents:`,
          "A=verify numbers match everywhere, B=check all links resolve, C=enforce glossary, D=narrative coherence, E=hunt stale pre-absorption refs.",
          "",
          "Fix ANY inconsistency you find. This is the last gate.",
          "",
          'Output: `{ "valid": true|false, "issues_remaining": [...], "fixes_applied": [...], "confidence": 0.0-1.0 }`',
        ].join("\n");
      },
      parse: (text) => {
        const parsed = extractJsonObject(text) as Record<string, unknown>;
        return {
          valid: parsed.valid ?? true,
          issues_remaining: (parsed.issues_remaining as string[]) || [],
          fixes_applied: (parsed.fixes_applied as string[]) || [],
          confidence: (parsed.confidence as number) ?? 0.9,
        };
      },
    }),

    judge_final: compute({
      run: ({ outputs }) => {
        const r = outputs.final_validation as {
          valid: boolean; confidence: number; issues_remaining: string[];
        };
        const pass = r.valid && r.confidence >= 0.8 && r.issues_remaining.length <= 2;
        return { route: pass ? "done" : "retry" };
      },
    }),

    // ═════════════════════════════════════════════════════════════════════════
    // REPORT
    // ═════════════════════════════════════════════════════════════════════════

    generate_report: compute({
      run: ({ outputs }) => {
        const analysis = outputs.analyze_all as AnalysisResult;
        const p1 = outputs.spawn_phase1 as { status: string };
        const p2 = outputs.spawn_phase2 as { status: string };
        const p3 = outputs.spawn_phase3 as { status: string };
        const p4 = outputs.spawn_readme as { status: string };
        const fv = outputs.final_validation as {
          valid: boolean; confidence: number;
          issues_remaining: string[]; fixes_applied: string[];
        };

        return {
          status: "COMPLETE",
          timestamp: new Date().toISOString(),
          summary: {
            documentsRevised: 9,
            issuesFound: analysis.inconsistencies.length,
            termsStandardized: analysis.termDefinitions.length,
            crossRefsChecked: analysis.crossRefs.length,
            finalFixesApplied: fv.fixes_applied.length,
            remainingIssues: fv.issues_remaining.length,
            confidence: fv.confidence,
          },
          phases: {
            "0_analysis": "complete",
            "1_independent_4x_codex": p1.status,
            "2_dependent_3x_claude": p2.status,
            "3_plan_1x_claude": p3.status,
            "4_readme_1x_claude": p4.status,
            "final_validation": fv.valid ? "passed" : "passed_with_notes",
          },
          agents: [
            "Agent 1:  Coordinator (Claude)  — analysis + final validation",
            "Agent 2:  01-VISION (Codex)      — Phase 1 parallel",
            "Agent 3:  02-OPPORTUNITIES (Codex) — Phase 1 parallel",
            "Agent 4:  04-ARCHITECTURE (Codex) — Phase 1 parallel",
            "Agent 5:  05-PATENT (Codex)      — Phase 1 parallel",
            "Agent 6:  06-GAP (Claude)        — Phase 2 parallel",
            "Agent 7:  07-ACPX (Claude)       — Phase 2 parallel",
            "Agent 8:  08-OPTIMIZATION (Claude) — Phase 2 parallel",
            "Agent 9:  03-PLAN (Claude)       — Phase 3 sequential",
            "Agent 10: README (Claude)        — Phase 4 sequential",
          ],
          remaining: fv.issues_remaining,
        };
      },
    }),
  },

  // ═══════════════════════════════════════════════════════════════════════════════
  // EDGES (20 edges, 1 retry loop)
  // ═══════════════════════════════════════════════════════════════════════════════

  edges: [
    // Phase 0
    { from: "load_config",       to: "scan_docs" },
    { from: "scan_docs",         to: "analyze_all" },
    { from: "analyze_all",       to: "plan_tasks" },
    { from: "plan_tasks",        to: "approve_plan" },

    // Phase 1: 4 parallel Codex
    { from: "approve_plan",      to: "spawn_phase1" },
    { from: "spawn_phase1",      to: "phase1_gate" },

    // Phase 2: 3 parallel Claude
    { from: "phase1_gate",       to: "spawn_phase2" },
    { from: "spawn_phase2",      to: "phase2_gate" },

    // Phase 3: 03-PLAN (depends on 07+08)
    { from: "phase2_gate",       to: "spawn_phase3" },
    { from: "spawn_phase3",      to: "phase3_gate" },

    // Phase 4: README + final validation
    { from: "phase3_gate",       to: "spawn_readme" },
    { from: "spawn_readme",      to: "final_validation" },
    { from: "final_validation",  to: "judge_final" },
    {
      from: "judge_final",
      switch: {
        on: "$output.route",
        cases: {
          done:  "generate_report",
          retry: "final_validation",   // ← LOOP: coordinator fixes and re-validates
        },
      },
    },
  ],
});
