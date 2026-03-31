/**
 * Strategy Docs Revision Flow — 10-Agent Parallel Documentation Pipeline
 *
 * A 67-node acpx workflow that revises all 9 strategy documents at
 * $HOME/projects/agents/docs/strategy/ using a team of 10 agents:
 *
 *   Phase 0: Analysis (Agent 1 — Coordinator)
 *     0.1  load_config        (compute) — Parse input, resolve paths
 *     0.2  scan_docs          (shell)   — Read all 9 docs, collect metadata
 *     0.3  analyze_crossrefs  (acp)     — Identify cross-references, inconsistencies, terms
 *     0.4  plan_revision      (compute) — Create task plan, group by dependency
 *     0.5  checkpoint_plan    (checkpoint) — Human approves the revision plan
 *
 *   Phase 1: Independent Doc Revisions (Agents 2-5, sequential in acpx, parallel in APXM)
 *     For each doc [01-VISION, 02-OPPORTUNITIES, 04-ARCHITECTURE, 05-PATENT]:
 *       1.X.1  create_worktree_X  (shell)   — git worktree add
 *       1.X.2  revise_doc_X       (acp)     — Agent revises the document
 *       1.X.3  validate_X         (acp)     — Validate consistency + cross-refs
 *       1.X.4  judge_valid_X      (compute) — Route: pass or retry
 *       1.X.5  simplify_X         (acp)     — Run simplify skill
 *       1.X.6  regression_X       (shell)   — Check nothing else broke
 *       1.X.7  merge_X            (shell)   — Merge worktree changes
 *
 *   Phase 2: Dependent Doc Revisions (Agents 6-9)
 *     For each doc [03-PLAN, 06-GAP, 07-ACPX-ABSORPTION, 08-OPTIMIZATION]:
 *       2.X.1  create_worktree_X  (shell)
 *       2.X.2  revise_doc_X       (acp)     — Uses Phase 1 outputs for consistency
 *       2.X.3  validate_X         (acp)
 *       2.X.4  judge_valid_X      (compute) — Route: pass or retry
 *       2.X.5  simplify_X         (acp)
 *       2.X.6  regression_X       (shell)
 *       2.X.7  merge_X            (shell)
 *
 *   Phase 3: Final Integration (Agent 10)
 *     3.1  create_worktree_readme  (shell)
 *     3.2  revise_readme           (acp)     — Update index to reflect all changes
 *     3.3  final_consistency       (acp)     — Full cross-doc validation
 *     3.4  judge_final             (compute) — Route: pass or loop back to 3.2
 *     3.5  simplify_readme         (acp)
 *     3.6  merge_readme            (shell)
 *     3.7  generate_report         (compute) — Summary of all changes
 *
 * Total: 67 nodes, 72 edges (including retry loops)
 *
 * Usage:
 *   acpx flow run ./strategy-revision.flow.ts \
 *     --input-json '{"docsPath":"$HOME/projects/agents/docs/strategy"}' \
 *     --approve-all
 */
import { defineFlow, acp, compute, shell, checkpoint, extractJsonObject } from "acpx/flows";

// ─── Types ───────────────────────────────────────────────────────────────────

type RevisionInput = {
  docsPath: string;
  projectRoot?: string;
};

type DocInfo = {
  file: string;
  phase: "independent" | "dependent" | "final";
  agent: number;
  dependencies: string[];
};

type AnalysisResult = {
  inconsistencies: { doc: string; issue: string; severity: string }[];
  crossRefs: { from: string; to: string; type: string }[];
  termDefinitions: { term: string; doc: string; definition: string }[];
  plan: string;
};

type ValidationResult = {
  valid: boolean;
  issues: string[];
  suggestions: string[];
};

// ─── Constants ───────────────────────────────────────────────────────────────

const DOCS: DocInfo[] = [
  // Phase 1: Independent (no cross-doc data dependencies)
  { file: "01-VISION.md",                   phase: "independent", agent: 2, dependencies: [] },
  { file: "02-OPPORTUNITIES.md",            phase: "independent", agent: 3, dependencies: [] },
  { file: "04-ARCHITECTURE-INTEGRATION.md", phase: "independent", agent: 4, dependencies: [] },
  { file: "05-PATENT-ALIGNMENT.md",         phase: "independent", agent: 5, dependencies: [] },
  // Phase 2: Dependent (reference Phase 1 docs)
  { file: "03-PLAN.md",                     phase: "dependent",   agent: 6, dependencies: ["01-VISION.md", "04-ARCHITECTURE-INTEGRATION.md", "07-ACPX-ABSORPTION.md", "08-OPTIMIZATION-TARGETS.md"] },
  { file: "06-GAP-ANALYSIS.md",             phase: "dependent",   agent: 7, dependencies: ["01-VISION.md", "02-OPPORTUNITIES.md", "04-ARCHITECTURE-INTEGRATION.md"] },
  { file: "07-ACPX-ABSORPTION.md",          phase: "dependent",   agent: 8, dependencies: ["01-VISION.md", "04-ARCHITECTURE-INTEGRATION.md"] },
  { file: "08-OPTIMIZATION-TARGETS.md",     phase: "dependent",   agent: 9, dependencies: ["04-ARCHITECTURE-INTEGRATION.md", "05-PATENT-ALIGNMENT.md"] },
  // Phase 3: Final (depends on ALL)
  { file: "README.md",                      phase: "final",       agent: 10, dependencies: ["01-VISION.md", "02-OPPORTUNITIES.md", "03-PLAN.md", "04-ARCHITECTURE-INTEGRATION.md", "05-PATENT-ALIGNMENT.md", "06-GAP-ANALYSIS.md", "07-ACPX-ABSORPTION.md", "08-OPTIMIZATION-TARGETS.md"] },
];

const INDEPENDENT_DOCS = DOCS.filter(d => d.phase === "independent");
const DEPENDENT_DOCS   = DOCS.filter(d => d.phase === "dependent");
const COORDINATOR_SESSION = "coordinator";

// ─── Helpers ─────────────────────────────────────────────────────────────────

function docId(file: string): string {
  return file.replace(/\.md$/, "").replace(/-/g, "_").toLowerCase();
}

function worktreePath(file: string): string {
  return `/tmp/strategy-revision-${docId(file)}`;
}

function branchName(file: string): string {
  return `revision/${docId(file)}`;
}

// ─── Shared prompt builders ─────────────────────────────────────────────────

function buildRevisionPrompt(
  file: string,
  docsPath: string,
  analysis: AnalysisResult,
  phase: string,
): string {
  const docIssues = analysis.inconsistencies.filter(i => i.doc === file);
  const docRefs = analysis.crossRefs.filter(r => r.from === file || r.to === file);

  return [
    `## Task: Revise ${file}`,
    "",
    `You are Agent ${DOCS.find(d => d.file === file)?.agent} in a 10-agent documentation revision team.`,
    `Phase: ${phase}`,
    "",
    "### Objectives",
    "1. Fix all identified inconsistencies in this document",
    "2. Ensure terminology is consistent with the canonical definitions",
    "3. Fix broken or inaccurate cross-references to other documents",
    "4. Ensure numerical claims (lines of code, percentages, timelines) are consistent across docs",
    "5. Improve clarity without changing the document's intent or structure",
    "",
    "### Identified Issues for This Document",
    docIssues.length > 0
      ? docIssues.map(i => `- [${i.severity}] ${i.issue}`).join("\n")
      : "- No specific issues identified (still check for general consistency)",
    "",
    "### Cross-References Involving This Document",
    docRefs.length > 0
      ? docRefs.map(r => `- ${r.from} -> ${r.to}: ${r.type}`).join("\n")
      : "- No cross-references found",
    "",
    "### Canonical Term Definitions",
    analysis.termDefinitions.slice(0, 20).map(t => `- **${t.term}**: ${t.definition} (from ${t.doc})`).join("\n"),
    "",
    "### Instructions",
    `- The document is at: ${docsPath}/${file}`,
    "- Read the FULL document first",
    "- Make targeted edits — preserve structure and intent",
    "- Fix inconsistencies, not style preferences",
    "- After editing, verify your changes are self-consistent",
    "",
    "Output JSON when done:",
    "```json",
    "{",
    '  "changes_made": ["description of change 1", ...],',
    '  "issues_fixed": ["issue that was fixed", ...],',
    '  "remaining_concerns": ["concern that needs cross-doc attention", ...]',
    "}",
    "```",
  ].join("\n");
}

function buildValidationPrompt(
  file: string,
  docsPath: string,
  analysis: AnalysisResult,
): string {
  return [
    `## Task: Validate ${file} After Revision`,
    "",
    "Check that the revised document is consistent and correct:",
    "",
    "1. **Internal consistency**: Do all claims within the document agree?",
    "2. **Cross-reference accuracy**: Do references to other docs match their content?",
    "3. **Terminology**: Are all terms used consistently with canonical definitions?",
    "4. **Numerical accuracy**: Do numbers (lines of code, percentages, weeks) match across docs?",
    "5. **Completeness**: Are there any dangling references or incomplete sections?",
    "",
    `Read the document at: ${docsPath}/${file}`,
    "Also spot-check referenced documents for consistency.",
    "",
    "### Canonical Terms (must match)",
    analysis.termDefinitions.slice(0, 15).map(t => `- **${t.term}**: ${t.definition}`).join("\n"),
    "",
    "Output JSON:",
    "```json",
    "{",
    '  "valid": true | false,',
    '  "issues": ["issue found", ...],',
    '  "suggestions": ["improvement suggestion", ...]',
    "}",
    "```",
  ].join("\n");
}

function buildSimplifyPrompt(file: string, docsPath: string): string {
  return [
    `## Task: Simplify ${file}`,
    "",
    "Review the revised document for:",
    "1. **Redundancy**: Remove repeated information within the document",
    "2. **Clarity**: Simplify overly complex sentences",
    "3. **Conciseness**: Remove filler words and unnecessary qualifications",
    "4. **Code examples**: Ensure code blocks are accurate and minimal",
    "5. **Tables**: Ensure table data is accurate and well-formatted",
    "",
    "DO NOT change:",
    "- Document structure or section headings",
    "- Technical accuracy or architectural decisions",
    "- Cross-references to other documents",
    "- Numerical claims (they've been validated)",
    "",
    `Read and edit: ${docsPath}/${file}`,
    "",
    "Output JSON when done:",
    "```json",
    "{",
    '  "simplifications": ["what you simplified", ...],',
    '  "lines_removed": 0,',
    '  "lines_added": 0',
    "}",
    "```",
  ].join("\n");
}

// ═══════════════════════════════════════════════════════════════════════════════
// FLOW DEFINITION — 67 nodes, 72 edges
// ═══════════════════════════════════════════════════════════════════════════════

export default defineFlow({
  name: "strategy-revision",
  startAt: "load_config",

  run: {
    title: ({ input }) => {
      const { docsPath } = input as RevisionInput;
      return `Strategy docs revision: ${docsPath}`;
    },
  },

  permissions: {
    requiredMode: "approve-all",
    reason: "Flow creates git worktrees, edits documentation files, runs agents, and merges branches",
  },

  nodes: {
    // ═══════════════════════════════════════════════════════════════════════════
    // PHASE 0: ANALYSIS (Nodes 0.1 - 0.5)
    // ═══════════════════════════════════════════════════════════════════════════

    // 0.1 — Parse input, resolve paths
    load_config: compute({
      run: ({ input }) => {
        const { docsPath, projectRoot } = input as RevisionInput;
        return {
          docsPath: docsPath || "$HOME/projects/agents/docs/strategy",
          projectRoot: projectRoot || "$HOME/projects/agents",
          docs: DOCS,
          independentDocs: INDEPENDENT_DOCS,
          dependentDocs: DEPENDENT_DOCS,
          timestamp: new Date().toISOString(),
        };
      },
    }),

    // 0.2 — Read all docs, collect file sizes and structure
    scan_docs: shell({
      statusDetail: "Scanning all 9 strategy documents...",
      exec: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return {
          command: "bash",
          args: ["-c", `cd "${docsPath}" && for f in *.md; do echo "=== $f ===" && wc -l "$f" && head -5 "$f" && echo "---"; done`],
          shell: false,
        };
      },
      parse: (result) => ({
        scan: result.stdout,
        docCount: (result.stdout.match(/=== .+\.md ===/g) || []).length,
      }),
    }),

    // 0.3 — Agent 1 (Coordinator) analyzes cross-references and inconsistencies
    analyze_crossrefs: acp({
      session: { handle: COORDINATOR_SESSION },
      statusDetail: "Agent 1: Analyzing cross-references and inconsistencies...",
      timeoutMs: 10 * 60_000,
      cwd: ({ outputs }) => (outputs.load_config as { projectRoot: string }).projectRoot,

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };

        return [
          "## Task: Comprehensive Strategy Documentation Analysis",
          "",
          "You are Agent 1 (Coordinator) in a 10-agent documentation revision team.",
          "",
          `Read ALL 9 documents at ${docsPath}/:`,
          "- README.md",
          "- 01-VISION.md",
          "- 02-OPPORTUNITIES.md",
          "- 03-PLAN.md",
          "- 04-ARCHITECTURE-INTEGRATION.md",
          "- 05-PATENT-ALIGNMENT.md",
          "- 06-GAP-ANALYSIS.md",
          "- 07-ACPX-ABSORPTION.md",
          "- 08-OPTIMIZATION-TARGETS.md",
          "",
          "### Analyze for:",
          "",
          "1. **Inconsistencies**: Numbers that don't match across docs (e.g., '~800 lines' vs '~500-800 lines')",
          "2. **Cross-references**: Every reference from one doc to another — are they accurate?",
          "3. **Term definitions**: Build a canonical glossary (e.g., 'AcpCapability', 'ModelRouter', 'ContextStack')",
          "4. **Timeline conflicts**: Do week numbers and phase assignments agree?",
          "5. **Architecture conflicts**: Do component descriptions match across docs?",
          "6. **Missing information**: Gaps where one doc claims something another should explain",
          "",
          "### Known issues to investigate:",
          "- AcpCapability code size: '~800 lines' vs '~500-800 lines' — which is canonical?",
          "- ACPX line count: verify '~30K lines TypeScript' claim",
          "- Phase 2 duplication between 03-PLAN.md and 07-ACPX-ABSORPTION.md",
          "- Model name inconsistency: 'claude-opus-4' vs 'claude-sonnet-4' for same tier",
          "- Gap A6 priority mismatch: P0 in 06-GAP but not in 03-PLAN phases",
          "- Compiler pass count: '13 total (+6 new)' in README vs 6 passes in 08-OPTIMIZATION",
          "",
          "Output a COMPREHENSIVE JSON analysis:",
          "```json",
          "{",
          '  "inconsistencies": [',
          '    {"doc": "filename.md", "issue": "description", "severity": "high|medium|low"}',
          "  ],",
          '  "crossRefs": [',
          '    {"from": "source.md", "to": "target.md", "type": "cites|contradicts|extends"}',
          "  ],",
          '  "termDefinitions": [',
          '    {"term": "AcpCapability", "doc": "canonical-source.md", "definition": "concise definition"}',
          "  ],",
          '  "plan": "Summary of what needs to change and in what order"',
          "}",
          "```",
        ].join("\n");
      },

      parse: (text) => {
        const parsed = extractJsonObject(text) as AnalysisResult;
        return {
          inconsistencies: parsed.inconsistencies || [],
          crossRefs: parsed.crossRefs || [],
          termDefinitions: parsed.termDefinitions || [],
          plan: parsed.plan || "Proceed with standard revision order",
        };
      },
    }),

    // 0.4 — Create task plan from analysis
    plan_revision: compute({
      run: ({ outputs }) => {
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return {
          totalIssues: analysis.inconsistencies.length,
          highSeverity: analysis.inconsistencies.filter(i => i.severity === "high").length,
          totalCrossRefs: analysis.crossRefs.length,
          totalTerms: analysis.termDefinitions.length,
          phase1Docs: INDEPENDENT_DOCS.map(d => d.file),
          phase2Docs: DEPENDENT_DOCS.map(d => d.file),
          phase3Doc: "README.md",
          plan: analysis.plan,
        };
      },
    }),

    // 0.5 — Human checkpoint: approve the revision plan
    checkpoint_plan: checkpoint({
      summary: "Review the analysis and revision plan before proceeding with document edits.",
    }),

    // ═══════════════════════════════════════════════════════════════════════════
    // PHASE 1: INDEPENDENT DOC REVISIONS
    // In APXM: these 4 doc pipelines run in PARALLEL
    // In acpx: they run sequentially (one after another)
    // ═══════════════════════════════════════════════════════════════════════════

    // ─── 01-VISION.md (Agent 2) ──────────────────────────────────────────────

    create_worktree_vision: shell({
      statusDetail: "Creating worktree for 01-VISION.md...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        return {
          command: "bash",
          args: ["-c", `cd "${projectRoot}" && git worktree add "${worktreePath("01-VISION.md")}" -b "${branchName("01-VISION.md")}" 2>&1 || echo "Worktree already exists"`],
          shell: false,
        };
      },
      parse: (result) => ({ worktree: worktreePath("01-VISION.md"), branch: branchName("01-VISION.md"), stdout: result.stdout }),
    }),

    revise_vision: acp({
      session: { handle: "agent-2-vision", isolated: true },
      statusDetail: "Agent 2: Revising 01-VISION.md...",
      timeoutMs: 10 * 60_000,
      cwd: () => worktreePath("01-VISION.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return buildRevisionPrompt("01-VISION.md", docsPath, analysis, "Phase 1 (Independent)");
      },
      parse: (text) => extractJsonObject(text),
    }),

    validate_vision: acp({
      session: { handle: "agent-2-vision" },
      statusDetail: "Agent 2: Validating 01-VISION.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("01-VISION.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return buildValidationPrompt("01-VISION.md", docsPath, analysis);
      },
      parse: (text) => {
        const parsed = extractJsonObject(text) as ValidationResult;
        return { valid: parsed.valid ?? true, issues: parsed.issues || [], suggestions: parsed.suggestions || [] };
      },
    }),

    judge_valid_vision: compute({
      run: ({ outputs }) => {
        const validation = outputs.validate_vision as ValidationResult;
        return { route: validation.valid ? "pass" : "retry" };
      },
    }),

    // Retry path loops back to revise_vision (via edge)
    // Pass path continues to simplify

    simplify_vision: acp({
      session: { handle: "agent-2-vision" },
      statusDetail: "Agent 2: Simplifying 01-VISION.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("01-VISION.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return buildSimplifyPrompt("01-VISION.md", docsPath);
      },
      parse: (text) => extractJsonObject(text),
    }),

    regression_vision: shell({
      statusDetail: "Checking for regressions after 01-VISION.md changes...",
      exec: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return {
          command: "bash",
          args: ["-c", `cd "${docsPath}" && grep -rn "01-VISION\\|VISION\\.md" *.md | grep -v "^01-VISION.md:" || echo "No external references found"`],
          shell: false,
        };
      },
      parse: (result) => ({ references: result.stdout, clean: result.exitCode === 0 }),
    }),

    merge_vision: shell({
      statusDetail: "Merging 01-VISION.md changes...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        const wt = worktreePath("01-VISION.md");
        const branch = branchName("01-VISION.md");
        return {
          command: "bash",
          args: ["-c", `cd "${wt}" && git add -A && git commit -m "docs: revise 01-VISION.md for consistency" --allow-empty && cd "${projectRoot}" && git merge "${branch}" --no-edit && git worktree remove "${wt}" --force 2>/dev/null; echo "Done"`],
          shell: false,
        };
      },
      parse: (result) => ({ merged: true, stdout: result.stdout }),
    }),

    // ─── 02-OPPORTUNITIES.md (Agent 3) ──────────────────────────────────────

    create_worktree_opportunities: shell({
      statusDetail: "Creating worktree for 02-OPPORTUNITIES.md...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        return {
          command: "bash",
          args: ["-c", `cd "${projectRoot}" && git worktree add "${worktreePath("02-OPPORTUNITIES.md")}" -b "${branchName("02-OPPORTUNITIES.md")}" 2>&1 || echo "Worktree already exists"`],
          shell: false,
        };
      },
      parse: (result) => ({ worktree: worktreePath("02-OPPORTUNITIES.md"), branch: branchName("02-OPPORTUNITIES.md"), stdout: result.stdout }),
    }),

    revise_opportunities: acp({
      session: { handle: "agent-3-opportunities", isolated: true },
      statusDetail: "Agent 3: Revising 02-OPPORTUNITIES.md...",
      timeoutMs: 10 * 60_000,
      cwd: () => worktreePath("02-OPPORTUNITIES.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return buildRevisionPrompt("02-OPPORTUNITIES.md", docsPath, analysis, "Phase 1 (Independent)");
      },
      parse: (text) => extractJsonObject(text),
    }),

    validate_opportunities: acp({
      session: { handle: "agent-3-opportunities" },
      statusDetail: "Agent 3: Validating 02-OPPORTUNITIES.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("02-OPPORTUNITIES.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return buildValidationPrompt("02-OPPORTUNITIES.md", docsPath, analysis);
      },
      parse: (text) => {
        const parsed = extractJsonObject(text) as ValidationResult;
        return { valid: parsed.valid ?? true, issues: parsed.issues || [], suggestions: parsed.suggestions || [] };
      },
    }),

    judge_valid_opportunities: compute({
      run: ({ outputs }) => {
        const validation = outputs.validate_opportunities as ValidationResult;
        return { route: validation.valid ? "pass" : "retry" };
      },
    }),

    simplify_opportunities: acp({
      session: { handle: "agent-3-opportunities" },
      statusDetail: "Agent 3: Simplifying 02-OPPORTUNITIES.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("02-OPPORTUNITIES.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return buildSimplifyPrompt("02-OPPORTUNITIES.md", docsPath);
      },
      parse: (text) => extractJsonObject(text),
    }),

    regression_opportunities: shell({
      statusDetail: "Checking for regressions after 02-OPPORTUNITIES.md changes...",
      exec: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return {
          command: "bash",
          args: ["-c", `cd "${docsPath}" && grep -rn "02-OPPORTUNITIES\\|OPPORTUNITIES\\.md" *.md | grep -v "^02-OPPORTUNITIES.md:" || echo "No external references found"`],
          shell: false,
        };
      },
      parse: (result) => ({ references: result.stdout, clean: result.exitCode === 0 }),
    }),

    merge_opportunities: shell({
      statusDetail: "Merging 02-OPPORTUNITIES.md changes...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        const wt = worktreePath("02-OPPORTUNITIES.md");
        const branch = branchName("02-OPPORTUNITIES.md");
        return {
          command: "bash",
          args: ["-c", `cd "${wt}" && git add -A && git commit -m "docs: revise 02-OPPORTUNITIES.md for consistency" --allow-empty && cd "${projectRoot}" && git merge "${branch}" --no-edit && git worktree remove "${wt}" --force 2>/dev/null; echo "Done"`],
          shell: false,
        };
      },
      parse: (result) => ({ merged: true, stdout: result.stdout }),
    }),

    // ─── 04-ARCHITECTURE-INTEGRATION.md (Agent 4) ──────────────────────────

    create_worktree_architecture: shell({
      statusDetail: "Creating worktree for 04-ARCHITECTURE-INTEGRATION.md...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        return {
          command: "bash",
          args: ["-c", `cd "${projectRoot}" && git worktree add "${worktreePath("04-ARCHITECTURE-INTEGRATION.md")}" -b "${branchName("04-ARCHITECTURE-INTEGRATION.md")}" 2>&1 || echo "Worktree already exists"`],
          shell: false,
        };
      },
      parse: (result) => ({ worktree: worktreePath("04-ARCHITECTURE-INTEGRATION.md"), stdout: result.stdout }),
    }),

    revise_architecture: acp({
      session: { handle: "agent-4-architecture", isolated: true },
      statusDetail: "Agent 4: Revising 04-ARCHITECTURE-INTEGRATION.md...",
      timeoutMs: 10 * 60_000,
      cwd: () => worktreePath("04-ARCHITECTURE-INTEGRATION.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return buildRevisionPrompt("04-ARCHITECTURE-INTEGRATION.md", docsPath, analysis, "Phase 1 (Independent)");
      },
      parse: (text) => extractJsonObject(text),
    }),

    validate_architecture: acp({
      session: { handle: "agent-4-architecture" },
      statusDetail: "Agent 4: Validating 04-ARCHITECTURE-INTEGRATION.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("04-ARCHITECTURE-INTEGRATION.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return buildValidationPrompt("04-ARCHITECTURE-INTEGRATION.md", docsPath, analysis);
      },
      parse: (text) => {
        const parsed = extractJsonObject(text) as ValidationResult;
        return { valid: parsed.valid ?? true, issues: parsed.issues || [], suggestions: parsed.suggestions || [] };
      },
    }),

    judge_valid_architecture: compute({
      run: ({ outputs }) => {
        const validation = outputs.validate_architecture as ValidationResult;
        return { route: validation.valid ? "pass" : "retry" };
      },
    }),

    simplify_architecture: acp({
      session: { handle: "agent-4-architecture" },
      statusDetail: "Agent 4: Simplifying 04-ARCHITECTURE-INTEGRATION.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("04-ARCHITECTURE-INTEGRATION.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return buildSimplifyPrompt("04-ARCHITECTURE-INTEGRATION.md", docsPath);
      },
      parse: (text) => extractJsonObject(text),
    }),

    regression_architecture: shell({
      statusDetail: "Checking for regressions after 04-ARCHITECTURE changes...",
      exec: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return {
          command: "bash",
          args: ["-c", `cd "${docsPath}" && grep -rn "04-ARCHITECTURE\\|ARCHITECTURE-INTEGRATION\\.md" *.md | grep -v "^04-ARCHITECTURE-INTEGRATION.md:" || echo "No external references found"`],
          shell: false,
        };
      },
      parse: (result) => ({ references: result.stdout, clean: result.exitCode === 0 }),
    }),

    merge_architecture: shell({
      statusDetail: "Merging 04-ARCHITECTURE-INTEGRATION.md changes...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        const wt = worktreePath("04-ARCHITECTURE-INTEGRATION.md");
        const branch = branchName("04-ARCHITECTURE-INTEGRATION.md");
        return {
          command: "bash",
          args: ["-c", `cd "${wt}" && git add -A && git commit -m "docs: revise 04-ARCHITECTURE-INTEGRATION.md for consistency" --allow-empty && cd "${projectRoot}" && git merge "${branch}" --no-edit && git worktree remove "${wt}" --force 2>/dev/null; echo "Done"`],
          shell: false,
        };
      },
      parse: (result) => ({ merged: true, stdout: result.stdout }),
    }),

    // ─── 05-PATENT-ALIGNMENT.md (Agent 5) ───────────────────────────────────

    create_worktree_patent: shell({
      statusDetail: "Creating worktree for 05-PATENT-ALIGNMENT.md...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        return {
          command: "bash",
          args: ["-c", `cd "${projectRoot}" && git worktree add "${worktreePath("05-PATENT-ALIGNMENT.md")}" -b "${branchName("05-PATENT-ALIGNMENT.md")}" 2>&1 || echo "Worktree already exists"`],
          shell: false,
        };
      },
      parse: (result) => ({ worktree: worktreePath("05-PATENT-ALIGNMENT.md"), stdout: result.stdout }),
    }),

    revise_patent: acp({
      session: { handle: "agent-5-patent", isolated: true },
      statusDetail: "Agent 5: Revising 05-PATENT-ALIGNMENT.md...",
      timeoutMs: 10 * 60_000,
      cwd: () => worktreePath("05-PATENT-ALIGNMENT.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return buildRevisionPrompt("05-PATENT-ALIGNMENT.md", docsPath, analysis, "Phase 1 (Independent)");
      },
      parse: (text) => extractJsonObject(text),
    }),

    validate_patent: acp({
      session: { handle: "agent-5-patent" },
      statusDetail: "Agent 5: Validating 05-PATENT-ALIGNMENT.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("05-PATENT-ALIGNMENT.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return buildValidationPrompt("05-PATENT-ALIGNMENT.md", docsPath, analysis);
      },
      parse: (text) => {
        const parsed = extractJsonObject(text) as ValidationResult;
        return { valid: parsed.valid ?? true, issues: parsed.issues || [], suggestions: parsed.suggestions || [] };
      },
    }),

    judge_valid_patent: compute({
      run: ({ outputs }) => {
        const validation = outputs.validate_patent as ValidationResult;
        return { route: validation.valid ? "pass" : "retry" };
      },
    }),

    simplify_patent: acp({
      session: { handle: "agent-5-patent" },
      statusDetail: "Agent 5: Simplifying 05-PATENT-ALIGNMENT.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("05-PATENT-ALIGNMENT.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return buildSimplifyPrompt("05-PATENT-ALIGNMENT.md", docsPath);
      },
      parse: (text) => extractJsonObject(text),
    }),

    regression_patent: shell({
      statusDetail: "Checking for regressions after 05-PATENT changes...",
      exec: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return {
          command: "bash",
          args: ["-c", `cd "${docsPath}" && grep -rn "05-PATENT\\|PATENT-ALIGNMENT\\.md" *.md | grep -v "^05-PATENT-ALIGNMENT.md:" || echo "No external references found"`],
          shell: false,
        };
      },
      parse: (result) => ({ references: result.stdout, clean: result.exitCode === 0 }),
    }),

    merge_patent: shell({
      statusDetail: "Merging 05-PATENT-ALIGNMENT.md changes...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        const wt = worktreePath("05-PATENT-ALIGNMENT.md");
        const branch = branchName("05-PATENT-ALIGNMENT.md");
        return {
          command: "bash",
          args: ["-c", `cd "${wt}" && git add -A && git commit -m "docs: revise 05-PATENT-ALIGNMENT.md for consistency" --allow-empty && cd "${projectRoot}" && git merge "${branch}" --no-edit && git worktree remove "${wt}" --force 2>/dev/null; echo "Done"`],
          shell: false,
        };
      },
      parse: (result) => ({ merged: true, stdout: result.stdout }),
    }),

    // ═══════════════════════════════════════════════════════════════════════════
    // PHASE 1 → PHASE 2 GATE
    // ═══════════════════════════════════════════════════════════════════════════

    phase1_complete: compute({
      run: ({ outputs }) => {
        const results = {
          vision: outputs.merge_vision,
          opportunities: outputs.merge_opportunities,
          architecture: outputs.merge_architecture,
          patent: outputs.merge_patent,
        };
        return { phase: 1, status: "complete", results };
      },
    }),

    // ═══════════════════════════════════════════════════════════════════════════
    // PHASE 2: DEPENDENT DOC REVISIONS
    // These docs reference Phase 1 docs — they must run AFTER Phase 1 completes
    // In APXM: these 4 run in PARALLEL (they depend on Phase 1, not each other)
    // ═══════════════════════════════════════════════════════════════════════════

    // ─── 03-PLAN.md (Agent 6) ────────────────────────────────────────────────

    create_worktree_plan: shell({
      statusDetail: "Creating worktree for 03-PLAN.md...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        return {
          command: "bash",
          args: ["-c", `cd "${projectRoot}" && git worktree add "${worktreePath("03-PLAN.md")}" -b "${branchName("03-PLAN.md")}" 2>&1 || echo "Worktree already exists"`],
          shell: false,
        };
      },
      parse: (result) => ({ worktree: worktreePath("03-PLAN.md"), stdout: result.stdout }),
    }),

    revise_plan: acp({
      session: { handle: "agent-6-plan", isolated: true },
      statusDetail: "Agent 6: Revising 03-PLAN.md (uses Phase 1 outputs)...",
      timeoutMs: 10 * 60_000,
      cwd: () => worktreePath("03-PLAN.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        const phase1 = outputs.phase1_complete as { results: Record<string, unknown> };

        return [
          buildRevisionPrompt("03-PLAN.md", docsPath, analysis, "Phase 2 (Dependent)"),
          "",
          "### IMPORTANT: Phase 1 Changes Already Made",
          "The following documents have ALREADY been revised in Phase 1.",
          "Ensure 03-PLAN.md is consistent with their updated content:",
          `- 01-VISION.md: ${JSON.stringify(phase1.results)}`,
          "- 04-ARCHITECTURE-INTEGRATION.md (revised)",
          "- 05-PATENT-ALIGNMENT.md (revised)",
          "",
          "Pay special attention to:",
          "- Week/phase assignments matching the revised vision",
          "- Code size estimates matching revised architecture doc",
          "- Removing duplication with 07-ACPX-ABSORPTION.md Phase 2 section",
        ].join("\n");
      },
      parse: (text) => extractJsonObject(text),
    }),

    validate_plan: acp({
      session: { handle: "agent-6-plan" },
      statusDetail: "Agent 6: Validating 03-PLAN.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("03-PLAN.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return buildValidationPrompt("03-PLAN.md", docsPath, analysis);
      },
      parse: (text) => {
        const parsed = extractJsonObject(text) as ValidationResult;
        return { valid: parsed.valid ?? true, issues: parsed.issues || [], suggestions: parsed.suggestions || [] };
      },
    }),

    judge_valid_plan: compute({
      run: ({ outputs }) => {
        const validation = outputs.validate_plan as ValidationResult;
        return { route: validation.valid ? "pass" : "retry" };
      },
    }),

    simplify_plan: acp({
      session: { handle: "agent-6-plan" },
      statusDetail: "Agent 6: Simplifying 03-PLAN.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("03-PLAN.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return buildSimplifyPrompt("03-PLAN.md", docsPath);
      },
      parse: (text) => extractJsonObject(text),
    }),

    regression_plan: shell({
      statusDetail: "Checking for regressions after 03-PLAN.md changes...",
      exec: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return {
          command: "bash",
          args: ["-c", `cd "${docsPath}" && grep -rn "03-PLAN\\|PLAN\\.md" *.md | grep -v "^03-PLAN.md:" || echo "No external references found"`],
          shell: false,
        };
      },
      parse: (result) => ({ references: result.stdout, clean: result.exitCode === 0 }),
    }),

    merge_plan: shell({
      statusDetail: "Merging 03-PLAN.md changes...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        const wt = worktreePath("03-PLAN.md");
        const branch = branchName("03-PLAN.md");
        return {
          command: "bash",
          args: ["-c", `cd "${wt}" && git add -A && git commit -m "docs: revise 03-PLAN.md for consistency" --allow-empty && cd "${projectRoot}" && git merge "${branch}" --no-edit && git worktree remove "${wt}" --force 2>/dev/null; echo "Done"`],
          shell: false,
        };
      },
      parse: (result) => ({ merged: true, stdout: result.stdout }),
    }),

    // ─── 06-GAP-ANALYSIS.md (Agent 7) ───────────────────────────────────────

    create_worktree_gap: shell({
      statusDetail: "Creating worktree for 06-GAP-ANALYSIS.md...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        return {
          command: "bash",
          args: ["-c", `cd "${projectRoot}" && git worktree add "${worktreePath("06-GAP-ANALYSIS.md")}" -b "${branchName("06-GAP-ANALYSIS.md")}" 2>&1 || echo "Worktree already exists"`],
          shell: false,
        };
      },
      parse: (result) => ({ worktree: worktreePath("06-GAP-ANALYSIS.md"), stdout: result.stdout }),
    }),

    revise_gap: acp({
      session: { handle: "agent-7-gap", isolated: true },
      statusDetail: "Agent 7: Revising 06-GAP-ANALYSIS.md...",
      timeoutMs: 10 * 60_000,
      cwd: () => worktreePath("06-GAP-ANALYSIS.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return [
          buildRevisionPrompt("06-GAP-ANALYSIS.md", docsPath, analysis, "Phase 2 (Dependent)"),
          "",
          "### Special attention for GAP-ANALYSIS:",
          "- Ensure Gap A6 priority is consistent with 03-PLAN.md phases",
          "- Verify effort estimates match the timeline in 03-PLAN.md",
          "- Cross-check gap descriptions against 04-ARCHITECTURE-INTEGRATION.md",
        ].join("\n");
      },
      parse: (text) => extractJsonObject(text),
    }),

    validate_gap: acp({
      session: { handle: "agent-7-gap" },
      statusDetail: "Agent 7: Validating 06-GAP-ANALYSIS.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("06-GAP-ANALYSIS.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return buildValidationPrompt("06-GAP-ANALYSIS.md", docsPath, analysis);
      },
      parse: (text) => {
        const parsed = extractJsonObject(text) as ValidationResult;
        return { valid: parsed.valid ?? true, issues: parsed.issues || [], suggestions: parsed.suggestions || [] };
      },
    }),

    judge_valid_gap: compute({
      run: ({ outputs }) => {
        const validation = outputs.validate_gap as ValidationResult;
        return { route: validation.valid ? "pass" : "retry" };
      },
    }),

    simplify_gap: acp({
      session: { handle: "agent-7-gap" },
      statusDetail: "Agent 7: Simplifying 06-GAP-ANALYSIS.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("06-GAP-ANALYSIS.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return buildSimplifyPrompt("06-GAP-ANALYSIS.md", docsPath);
      },
      parse: (text) => extractJsonObject(text),
    }),

    regression_gap: shell({
      statusDetail: "Checking for regressions after 06-GAP-ANALYSIS.md changes...",
      exec: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return {
          command: "bash",
          args: ["-c", `cd "${docsPath}" && grep -rn "06-GAP\\|GAP-ANALYSIS\\.md" *.md | grep -v "^06-GAP-ANALYSIS.md:" || echo "No external references found"`],
          shell: false,
        };
      },
      parse: (result) => ({ references: result.stdout, clean: result.exitCode === 0 }),
    }),

    merge_gap: shell({
      statusDetail: "Merging 06-GAP-ANALYSIS.md changes...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        const wt = worktreePath("06-GAP-ANALYSIS.md");
        const branch = branchName("06-GAP-ANALYSIS.md");
        return {
          command: "bash",
          args: ["-c", `cd "${wt}" && git add -A && git commit -m "docs: revise 06-GAP-ANALYSIS.md for consistency" --allow-empty && cd "${projectRoot}" && git merge "${branch}" --no-edit && git worktree remove "${wt}" --force 2>/dev/null; echo "Done"`],
          shell: false,
        };
      },
      parse: (result) => ({ merged: true, stdout: result.stdout }),
    }),

    // ─── 07-ACPX-ABSORPTION.md (Agent 8) ────────────────────────────────────

    create_worktree_absorption: shell({
      statusDetail: "Creating worktree for 07-ACPX-ABSORPTION.md...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        return {
          command: "bash",
          args: ["-c", `cd "${projectRoot}" && git worktree add "${worktreePath("07-ACPX-ABSORPTION.md")}" -b "${branchName("07-ACPX-ABSORPTION.md")}" 2>&1 || echo "Worktree already exists"`],
          shell: false,
        };
      },
      parse: (result) => ({ worktree: worktreePath("07-ACPX-ABSORPTION.md"), stdout: result.stdout }),
    }),

    revise_absorption: acp({
      session: { handle: "agent-8-absorption", isolated: true },
      statusDetail: "Agent 8: Revising 07-ACPX-ABSORPTION.md...",
      timeoutMs: 10 * 60_000,
      cwd: () => worktreePath("07-ACPX-ABSORPTION.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return [
          buildRevisionPrompt("07-ACPX-ABSORPTION.md", docsPath, analysis, "Phase 2 (Dependent)"),
          "",
          "### Special attention for ACPX-ABSORPTION:",
          "- Standardize AcpCapability code size to ONE number (check 01-VISION.md for canonical)",
          "- Remove Phase 2 implementation details that duplicate 03-PLAN.md",
          "- Ensure the capability comparison table matches 04-ARCHITECTURE-INTEGRATION.md",
        ].join("\n");
      },
      parse: (text) => extractJsonObject(text),
    }),

    validate_absorption: acp({
      session: { handle: "agent-8-absorption" },
      statusDetail: "Agent 8: Validating 07-ACPX-ABSORPTION.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("07-ACPX-ABSORPTION.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return buildValidationPrompt("07-ACPX-ABSORPTION.md", docsPath, analysis);
      },
      parse: (text) => {
        const parsed = extractJsonObject(text) as ValidationResult;
        return { valid: parsed.valid ?? true, issues: parsed.issues || [], suggestions: parsed.suggestions || [] };
      },
    }),

    judge_valid_absorption: compute({
      run: ({ outputs }) => {
        const validation = outputs.validate_absorption as ValidationResult;
        return { route: validation.valid ? "pass" : "retry" };
      },
    }),

    simplify_absorption: acp({
      session: { handle: "agent-8-absorption" },
      statusDetail: "Agent 8: Simplifying 07-ACPX-ABSORPTION.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("07-ACPX-ABSORPTION.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return buildSimplifyPrompt("07-ACPX-ABSORPTION.md", docsPath);
      },
      parse: (text) => extractJsonObject(text),
    }),

    regression_absorption: shell({
      statusDetail: "Checking for regressions after 07-ACPX-ABSORPTION.md changes...",
      exec: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return {
          command: "bash",
          args: ["-c", `cd "${docsPath}" && grep -rn "07-ACPX\\|ACPX-ABSORPTION\\.md" *.md | grep -v "^07-ACPX-ABSORPTION.md:" || echo "No external references found"`],
          shell: false,
        };
      },
      parse: (result) => ({ references: result.stdout, clean: result.exitCode === 0 }),
    }),

    merge_absorption: shell({
      statusDetail: "Merging 07-ACPX-ABSORPTION.md changes...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        const wt = worktreePath("07-ACPX-ABSORPTION.md");
        const branch = branchName("07-ACPX-ABSORPTION.md");
        return {
          command: "bash",
          args: ["-c", `cd "${wt}" && git add -A && git commit -m "docs: revise 07-ACPX-ABSORPTION.md for consistency" --allow-empty && cd "${projectRoot}" && git merge "${branch}" --no-edit && git worktree remove "${wt}" --force 2>/dev/null; echo "Done"`],
          shell: false,
        };
      },
      parse: (result) => ({ merged: true, stdout: result.stdout }),
    }),

    // ─── 08-OPTIMIZATION-TARGETS.md (Agent 9) ──────────────────────────────

    create_worktree_optimization: shell({
      statusDetail: "Creating worktree for 08-OPTIMIZATION-TARGETS.md...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        return {
          command: "bash",
          args: ["-c", `cd "${projectRoot}" && git worktree add "${worktreePath("08-OPTIMIZATION-TARGETS.md")}" -b "${branchName("08-OPTIMIZATION-TARGETS.md")}" 2>&1 || echo "Worktree already exists"`],
          shell: false,
        };
      },
      parse: (result) => ({ worktree: worktreePath("08-OPTIMIZATION-TARGETS.md"), stdout: result.stdout }),
    }),

    revise_optimization: acp({
      session: { handle: "agent-9-optimization", isolated: true },
      statusDetail: "Agent 9: Revising 08-OPTIMIZATION-TARGETS.md...",
      timeoutMs: 10 * 60_000,
      cwd: () => worktreePath("08-OPTIMIZATION-TARGETS.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return [
          buildRevisionPrompt("08-OPTIMIZATION-TARGETS.md", docsPath, analysis, "Phase 2 (Dependent)"),
          "",
          "### Special attention for OPTIMIZATION-TARGETS:",
          "- Verify pass count matches what README.md claims ('13 total (+6 new)')",
          "- Ensure compiler pass descriptions match 04-ARCHITECTURE-INTEGRATION.md",
          "- Cross-check estimated improvements with 02-OPPORTUNITIES.md claims",
        ].join("\n");
      },
      parse: (text) => extractJsonObject(text),
    }),

    validate_optimization: acp({
      session: { handle: "agent-9-optimization" },
      statusDetail: "Agent 9: Validating 08-OPTIMIZATION-TARGETS.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("08-OPTIMIZATION-TARGETS.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        return buildValidationPrompt("08-OPTIMIZATION-TARGETS.md", docsPath, analysis);
      },
      parse: (text) => {
        const parsed = extractJsonObject(text) as ValidationResult;
        return { valid: parsed.valid ?? true, issues: parsed.issues || [], suggestions: parsed.suggestions || [] };
      },
    }),

    judge_valid_optimization: compute({
      run: ({ outputs }) => {
        const validation = outputs.validate_optimization as ValidationResult;
        return { route: validation.valid ? "pass" : "retry" };
      },
    }),

    simplify_optimization: acp({
      session: { handle: "agent-9-optimization" },
      statusDetail: "Agent 9: Simplifying 08-OPTIMIZATION-TARGETS.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("08-OPTIMIZATION-TARGETS.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return buildSimplifyPrompt("08-OPTIMIZATION-TARGETS.md", docsPath);
      },
      parse: (text) => extractJsonObject(text),
    }),

    regression_optimization: shell({
      statusDetail: "Checking for regressions after 08-OPTIMIZATION-TARGETS.md changes...",
      exec: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return {
          command: "bash",
          args: ["-c", `cd "${docsPath}" && grep -rn "08-OPTIMIZATION\\|OPTIMIZATION-TARGETS\\.md" *.md | grep -v "^08-OPTIMIZATION-TARGETS.md:" || echo "No external references found"`],
          shell: false,
        };
      },
      parse: (result) => ({ references: result.stdout, clean: result.exitCode === 0 }),
    }),

    merge_optimization: shell({
      statusDetail: "Merging 08-OPTIMIZATION-TARGETS.md changes...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        const wt = worktreePath("08-OPTIMIZATION-TARGETS.md");
        const branch = branchName("08-OPTIMIZATION-TARGETS.md");
        return {
          command: "bash",
          args: ["-c", `cd "${wt}" && git add -A && git commit -m "docs: revise 08-OPTIMIZATION-TARGETS.md for consistency" --allow-empty && cd "${projectRoot}" && git merge "${branch}" --no-edit && git worktree remove "${wt}" --force 2>/dev/null; echo "Done"`],
          shell: false,
        };
      },
      parse: (result) => ({ merged: true, stdout: result.stdout }),
    }),

    // ═══════════════════════════════════════════════════════════════════════════
    // PHASE 2 → PHASE 3 GATE
    // ═══════════════════════════════════════════════════════════════════════════

    phase2_complete: compute({
      run: ({ outputs }) => {
        const results = {
          plan: outputs.merge_plan,
          gap: outputs.merge_gap,
          absorption: outputs.merge_absorption,
          optimization: outputs.merge_optimization,
        };
        return { phase: 2, status: "complete", results };
      },
    }),

    // ═══════════════════════════════════════════════════════════════════════════
    // PHASE 3: FINAL INTEGRATION (Agent 10)
    // README.md + full consistency check with retry loop
    // ═══════════════════════════════════════════════════════════════════════════

    create_worktree_readme: shell({
      statusDetail: "Creating worktree for README.md (final integration)...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        return {
          command: "bash",
          args: ["-c", `cd "${projectRoot}" && git worktree add "${worktreePath("README.md")}" -b "${branchName("README.md")}" 2>&1 || echo "Worktree already exists"`],
          shell: false,
        };
      },
      parse: (result) => ({ worktree: worktreePath("README.md"), stdout: result.stdout }),
    }),

    revise_readme: acp({
      session: { handle: "agent-10-readme", isolated: true },
      statusDetail: "Agent 10: Revising README.md (final integration)...",
      timeoutMs: 10 * 60_000,
      cwd: () => worktreePath("README.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        const analysis = outputs.analyze_crossrefs as AnalysisResult;

        return [
          buildRevisionPrompt("README.md", docsPath, analysis, "Phase 3 (Final Integration)"),
          "",
          "### CRITICAL: This is the index document",
          "ALL 8 other documents have been revised in Phases 1 and 2.",
          "The README must:",
          "1. Accurately link to and summarize each document",
          "2. Use the CANONICAL numbers from the revised documents",
          "3. Present a coherent narrative across all 8 documents",
          "4. The 'Key Numbers' table must match revised docs exactly",
          "5. The 'One-Paragraph Summary' must reflect all revisions",
          "",
          "Read ALL 8 revised documents before editing README.md.",
        ].join("\n");
      },
      parse: (text) => extractJsonObject(text),
    }),

    final_consistency: acp({
      session: { handle: "agent-10-readme" },
      statusDetail: "Agent 10: Full cross-document consistency check...",
      timeoutMs: 15 * 60_000,
      cwd: () => worktreePath("README.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };

        return [
          "## Task: FINAL Cross-Document Consistency Check",
          "",
          "This is the LAST validation gate before the revision is complete.",
          `Read ALL 9 documents at ${docsPath}/ and verify:`,
          "",
          "1. **AcpCapability code size**: Same number in all docs",
          "2. **ACPX line count**: Same number in all docs",
          "3. **Phase timelines**: Consistent across 01-VISION, 03-PLAN, README",
          "4. **Compiler pass counts**: Consistent across README, 08-OPTIMIZATION",
          "5. **Model names**: Consistent tier assignments across all docs",
          "6. **Gap priorities**: 06-GAP priorities match 03-PLAN phase assignments",
          "7. **Key Numbers table**: README matches individual doc claims",
          "8. **Cross-reference links**: All [link](file.md) references resolve correctly",
          "9. **Term definitions**: Canonical terms used consistently everywhere",
          "10. **Architecture diagrams**: ASCII art matches described components",
          "",
          "If you find ANY inconsistency, fix it in the affected document(s).",
          "",
          "Output JSON:",
          "```json",
          "{",
          '  "valid": true | false,',
          '  "issues": ["remaining issue", ...],',
          '  "fixes_applied": ["fix description", ...],',
          '  "suggestions": ["future improvement", ...]',
          "}",
          "```",
        ].join("\n");
      },
      parse: (text) => {
        const parsed = extractJsonObject(text) as ValidationResult & { fixes_applied?: string[] };
        return {
          valid: parsed.valid ?? true,
          issues: parsed.issues || [],
          fixes_applied: parsed.fixes_applied || [],
          suggestions: parsed.suggestions || [],
        };
      },
    }),

    // LOOP: If final consistency check fails, go back to revise_readme
    judge_final: compute({
      run: ({ outputs }) => {
        const validation = outputs.final_consistency as ValidationResult;
        return { route: validation.valid ? "pass" : "retry" };
      },
    }),

    simplify_readme: acp({
      session: { handle: "agent-10-readme" },
      statusDetail: "Agent 10: Final simplification pass on README.md...",
      timeoutMs: 5 * 60_000,
      cwd: () => worktreePath("README.md"),

      prompt: ({ outputs }) => {
        const { docsPath } = outputs.load_config as { docsPath: string };
        return buildSimplifyPrompt("README.md", docsPath);
      },
      parse: (text) => extractJsonObject(text),
    }),

    merge_readme: shell({
      statusDetail: "Merging final README.md changes...",
      exec: ({ outputs }) => {
        const { projectRoot } = outputs.load_config as { projectRoot: string };
        const wt = worktreePath("README.md");
        const branch = branchName("README.md");
        return {
          command: "bash",
          args: ["-c", `cd "${wt}" && git add -A && git commit -m "docs: revise README.md - final integration" --allow-empty && cd "${projectRoot}" && git merge "${branch}" --no-edit && git worktree remove "${wt}" --force 2>/dev/null; echo "Done"`],
          shell: false,
        };
      },
      parse: (result) => ({ merged: true, stdout: result.stdout }),
    }),

    // ═══════════════════════════════════════════════════════════════════════════
    // FINAL REPORT
    // ═══════════════════════════════════════════════════════════════════════════

    generate_report: compute({
      run: ({ outputs }) => {
        const analysis = outputs.analyze_crossrefs as AnalysisResult;
        const finalCheck = outputs.final_consistency as ValidationResult & { fixes_applied?: string[] };

        // Collect all revision outputs
        const revisions = [
          { doc: "01-VISION.md", result: outputs.revise_vision },
          { doc: "02-OPPORTUNITIES.md", result: outputs.revise_opportunities },
          { doc: "04-ARCHITECTURE-INTEGRATION.md", result: outputs.revise_architecture },
          { doc: "05-PATENT-ALIGNMENT.md", result: outputs.revise_patent },
          { doc: "03-PLAN.md", result: outputs.revise_plan },
          { doc: "06-GAP-ANALYSIS.md", result: outputs.revise_gap },
          { doc: "07-ACPX-ABSORPTION.md", result: outputs.revise_absorption },
          { doc: "08-OPTIMIZATION-TARGETS.md", result: outputs.revise_optimization },
          { doc: "README.md", result: outputs.revise_readme },
        ];

        return {
          status: "COMPLETE",
          summary: {
            totalDocsRevised: 9,
            totalIssuesFound: analysis.inconsistencies.length,
            totalFixesApplied: finalCheck.fixes_applied?.length || 0,
            remainingIssues: finalCheck.issues?.length || 0,
            crossRefsValidated: analysis.crossRefs.length,
            termsStandardized: analysis.termDefinitions.length,
          },
          phases: {
            phase1: { docs: ["01-VISION", "02-OPPORTUNITIES", "04-ARCHITECTURE", "05-PATENT"], status: "merged" },
            phase2: { docs: ["03-PLAN", "06-GAP", "07-ACPX-ABSORPTION", "08-OPTIMIZATION"], status: "merged" },
            phase3: { docs: ["README"], status: "merged" },
          },
          revisions,
          finalValidation: finalCheck,
          timestamp: new Date().toISOString(),
        };
      },
    }),
  },

  // ═══════════════════════════════════════════════════════════════════════════════
  // EDGES — 72 edges including 9 retry loops
  // ═══════════════════════════════════════════════════════════════════════════════

  edges: [
    // ─── Phase 0: Analysis pipeline ──────────────────────────────────────────
    { from: "load_config",       to: "scan_docs" },
    { from: "scan_docs",         to: "analyze_crossrefs" },
    { from: "analyze_crossrefs", to: "plan_revision" },
    { from: "plan_revision",     to: "checkpoint_plan" },

    // ─── Phase 0 → Phase 1 ──────────────────────────────────────────────────
    { from: "checkpoint_plan",   to: "create_worktree_vision" },

    // ─── 01-VISION (Agent 2) — with retry loop ─────────────────────────────
    { from: "create_worktree_vision",  to: "revise_vision" },
    { from: "revise_vision",           to: "validate_vision" },
    { from: "validate_vision",         to: "judge_valid_vision" },
    {
      from: "judge_valid_vision",
      switch: {
        on: "$output.route",
        cases: {
          pass:  "simplify_vision",
          retry: "revise_vision",      // ← LOOP: retry revision
        },
      },
    },
    { from: "simplify_vision",         to: "regression_vision" },
    { from: "regression_vision",       to: "merge_vision" },

    // ─── 02-OPPORTUNITIES (Agent 3) — with retry loop ──────────────────────
    { from: "merge_vision",                    to: "create_worktree_opportunities" },
    { from: "create_worktree_opportunities",   to: "revise_opportunities" },
    { from: "revise_opportunities",            to: "validate_opportunities" },
    { from: "validate_opportunities",          to: "judge_valid_opportunities" },
    {
      from: "judge_valid_opportunities",
      switch: {
        on: "$output.route",
        cases: {
          pass:  "simplify_opportunities",
          retry: "revise_opportunities",  // ← LOOP
        },
      },
    },
    { from: "simplify_opportunities",          to: "regression_opportunities" },
    { from: "regression_opportunities",        to: "merge_opportunities" },

    // ─── 04-ARCHITECTURE (Agent 4) — with retry loop ───────────────────────
    { from: "merge_opportunities",             to: "create_worktree_architecture" },
    { from: "create_worktree_architecture",    to: "revise_architecture" },
    { from: "revise_architecture",             to: "validate_architecture" },
    { from: "validate_architecture",           to: "judge_valid_architecture" },
    {
      from: "judge_valid_architecture",
      switch: {
        on: "$output.route",
        cases: {
          pass:  "simplify_architecture",
          retry: "revise_architecture",   // ← LOOP
        },
      },
    },
    { from: "simplify_architecture",           to: "regression_architecture" },
    { from: "regression_architecture",         to: "merge_architecture" },

    // ─── 05-PATENT (Agent 5) — with retry loop ────────────────────────────
    { from: "merge_architecture",              to: "create_worktree_patent" },
    { from: "create_worktree_patent",          to: "revise_patent" },
    { from: "revise_patent",                   to: "validate_patent" },
    { from: "validate_patent",                 to: "judge_valid_patent" },
    {
      from: "judge_valid_patent",
      switch: {
        on: "$output.route",
        cases: {
          pass:  "simplify_patent",
          retry: "revise_patent",         // ← LOOP
        },
      },
    },
    { from: "simplify_patent",                 to: "regression_patent" },
    { from: "regression_patent",               to: "merge_patent" },

    // ─── Phase 1 gate ────────────────────────────────────────────────────────
    { from: "merge_patent",                    to: "phase1_complete" },

    // ─── Phase 1 → Phase 2 ──────────────────────────────────────────────────
    { from: "phase1_complete",                 to: "create_worktree_plan" },

    // ─── 03-PLAN (Agent 6) — with retry loop ──────────────────────────────
    { from: "create_worktree_plan",            to: "revise_plan" },
    { from: "revise_plan",                     to: "validate_plan" },
    { from: "validate_plan",                   to: "judge_valid_plan" },
    {
      from: "judge_valid_plan",
      switch: {
        on: "$output.route",
        cases: {
          pass:  "simplify_plan",
          retry: "revise_plan",           // ← LOOP
        },
      },
    },
    { from: "simplify_plan",                   to: "regression_plan" },
    { from: "regression_plan",                 to: "merge_plan" },

    // ─── 06-GAP (Agent 7) — with retry loop ───────────────────────────────
    { from: "merge_plan",                      to: "create_worktree_gap" },
    { from: "create_worktree_gap",             to: "revise_gap" },
    { from: "revise_gap",                      to: "validate_gap" },
    { from: "validate_gap",                    to: "judge_valid_gap" },
    {
      from: "judge_valid_gap",
      switch: {
        on: "$output.route",
        cases: {
          pass:  "simplify_gap",
          retry: "revise_gap",            // ← LOOP
        },
      },
    },
    { from: "simplify_gap",                    to: "regression_gap" },
    { from: "regression_gap",                  to: "merge_gap" },

    // ─── 07-ACPX-ABSORPTION (Agent 8) — with retry loop ───────────────────
    { from: "merge_gap",                       to: "create_worktree_absorption" },
    { from: "create_worktree_absorption",      to: "revise_absorption" },
    { from: "revise_absorption",               to: "validate_absorption" },
    { from: "validate_absorption",             to: "judge_valid_absorption" },
    {
      from: "judge_valid_absorption",
      switch: {
        on: "$output.route",
        cases: {
          pass:  "simplify_absorption",
          retry: "revise_absorption",     // ← LOOP
        },
      },
    },
    { from: "simplify_absorption",             to: "regression_absorption" },
    { from: "regression_absorption",           to: "merge_absorption" },

    // ─── 08-OPTIMIZATION (Agent 9) — with retry loop ──────────────────────
    { from: "merge_absorption",                to: "create_worktree_optimization" },
    { from: "create_worktree_optimization",    to: "revise_optimization" },
    { from: "revise_optimization",             to: "validate_optimization" },
    { from: "validate_optimization",           to: "judge_valid_optimization" },
    {
      from: "judge_valid_optimization",
      switch: {
        on: "$output.route",
        cases: {
          pass:  "simplify_optimization",
          retry: "revise_optimization",   // ← LOOP
        },
      },
    },
    { from: "simplify_optimization",           to: "regression_optimization" },
    { from: "regression_optimization",         to: "merge_optimization" },

    // ─── Phase 2 gate ────────────────────────────────────────────────────────
    { from: "merge_optimization",              to: "phase2_complete" },

    // ─── Phase 2 → Phase 3 ──────────────────────────────────────────────────
    { from: "phase2_complete",                 to: "create_worktree_readme" },

    // ─── README (Agent 10) — with retry loop ───────────────────────────────
    { from: "create_worktree_readme",          to: "revise_readme" },
    { from: "revise_readme",                   to: "final_consistency" },
    { from: "final_consistency",               to: "judge_final" },
    {
      from: "judge_final",
      switch: {
        on: "$output.route",
        cases: {
          pass:  "simplify_readme",
          retry: "revise_readme",         // ← LOOP: full retry including re-revision
        },
      },
    },
    { from: "simplify_readme",                 to: "merge_readme" },

    // ─── Final report ────────────────────────────────────────────────────────
    { from: "merge_readme",                    to: "generate_report" },
  ],
});
