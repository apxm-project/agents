/**
 * Strategy Doc Revision — Single Document Sub-Flow (v2)
 *
 * Reusable 10-node pipeline that revises ONE strategy document.
 * Spawned in PARALLEL by the orchestrator, each in its own worktree + named session.
 *
 * PIPELINE (10 nodes, 2 retry loops):
 *
 *   load_input ──> create_worktree ──> revise_doc ──> validate_doc ──> judge_valid
 *                                          ^                              │
 *                                          └──────── retry (max 2) ───────┤
 *                                                                         v
 *                                      simplify_doc ──> regression_check ──> judge_regression
 *                                          ^                                      │
 *                                          └──────── fix (max 1) ─────────────────┤
 *                                                                                 v
 *                                                               collect_diff ──> merge_changes
 *
 * AGENT SELECTION (per step):
 *   revise_doc:       Codex (Phase 1) or Claude (Phase 2-4) — set via input.profile
 *   validate_doc:     Claude Code — always, cross-doc reasoning required
 *   simplify_doc:     Claude Code — always, runs /simplify skill
 *   Others:           shell or compute (no agent)
 *
 * ACPX constraints: sequential execution. In APXM, this sub-flow becomes
 * a DELEGATE sub-DAG that the dataflow scheduler can compose with others.
 *
 * Usage:
 *   acpx flow run ./strategy-revision-doc.flow.ts -s doc-01 \
 *     --input-json '{"file":"01-VISION.md",...}' --approve-all
 */
import { defineFlow, acp, compute, shell, extractJsonObject } from "acpx/flows";

// ─── Types ───────────────────────────────────────────────────────────────────

type DocRevisionInput = {
  file: string;
  docsPath: string;
  projectRoot: string;
  agentId: number;
  phase: "independent" | "dependent" | "plan" | "final";
  profile: "codex" | "claude";
  analysisJson: string;
  phaseContext?: string;
  maxRetries?: number;
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

// ─── Helpers ─────────────────────────────────────────────────────────────────

function docId(file: string): string {
  return file.replace(/\.md$/, "").replace(/-/g, "_").toLowerCase();
}

// Use PID to avoid worktree collisions when running in parallel
function worktreePath(file: string): string {
  return `/tmp/strategy-revision-${docId(file)}-${process.pid}`;
}

function branchName(file: string): string {
  return `revision/${docId(file)}`;
}

// ═══════════════════════════════════════════════════════════════════════════════

export default defineFlow({
  name: "strategy-doc-revision",
  startAt: "load_input",

  run: {
    title: ({ input }) => {
      const { file, agentId, phase } = input as DocRevisionInput;
      return `Agent ${agentId} [${phase}]: ${file}`;
    },
  },

  permissions: {
    requiredMode: "approve-all",
    reason: "Creates git worktree, edits documentation, merges branch",
  },

  nodes: {
    // ── 1. Parse input ──────────────────────────────────────────────────────
    load_input: compute({
      run: ({ input }) => {
        const inp = input as DocRevisionInput;
        let analysis: AnalysisResult;
        try {
          analysis = JSON.parse(inp.analysisJson || "{}");
        } catch {
          analysis = { inconsistencies: [], crossRefs: [], termDefinitions: [], plan: "" };
        }
        return {
          file: inp.file,
          docsPath: inp.docsPath,
          projectRoot: inp.projectRoot,
          agentId: inp.agentId,
          phase: inp.phase,
          profile: inp.profile || "codex",
          analysis,
          phaseContext: inp.phaseContext || "",
          worktree: worktreePath(inp.file),
          branch: branchName(inp.file),
          maxRetries: inp.maxRetries ?? 2,
          retryCount: 0,
        };
      },
    }),

    // ── 2. Create isolated worktree ─────────────────────────────────────────
    create_worktree: shell({
      statusDetail: "Creating git worktree...",
      exec: ({ outputs }) => {
        const { projectRoot, worktree, branch } = outputs.load_input as {
          projectRoot: string; worktree: string; branch: string;
        };
        return {
          command: "bash",
          args: ["-c", [
            `cd "${projectRoot}"`,
            `git worktree remove "${worktree}" --force 2>/dev/null; true`,
            `git branch -D "${branch}" 2>/dev/null; true`,
            `git worktree add "${worktree}" -b "${branch}"`,
            `echo "WORKTREE_READY: ${worktree} on branch ${branch}"`,
          ].join(" && ")],
          shell: false,
        };
      },
      parse: (result) => ({
        ready: result.exitCode === 0,
        stdout: result.stdout,
      }),
    }),

    // ── 3. Revise the document ──────────────────────────────────────────────
    revise_doc: acp({
      session: { handle: "revision", isolated: true },
      statusDetail: "Revising document...",
      timeoutMs: 10 * 60_000,
      cwd: ({ outputs }) => (outputs.load_input as { worktree: string }).worktree,

      prompt: ({ outputs }) => {
        const { file, docsPath, agentId, phase, analysis, phaseContext } =
          outputs.load_input as {
            file: string; docsPath: string; agentId: number;
            phase: string; analysis: AnalysisResult; phaseContext: string;
          };

        // Check if this is a retry (validation issues exist from prior attempt)
        const priorValidation = outputs.validate_doc as ValidationResult | undefined;
        const isRetry = priorValidation && !priorValidation.valid;

        const docIssues = analysis.inconsistencies?.filter(
          (i: { doc: string }) => i.doc === file
        ) || [];
        const docRefs = analysis.crossRefs?.filter(
          (r: { from: string; to: string }) => r.from === file || r.to === file
        ) || [];

        const lines = [
          `Revise ${file}. Using a team of 5 ultrathink agents:`,
          "A=fix numbers/facts, B=fix cross-refs, C=enforce terminology, D=improve clarity, E=guard structure.",
          "",
          `Read ${docsPath}/${file} then edit it.`,
        ];

        if (isRetry) {
          lines.push(
            "",
            "RETRY — fix these validation failures:",
            ...priorValidation.issues.map((i: string) => `- ${i}`),
          );
        }

        if (docIssues.length > 0) {
          lines.push("", "Known issues:");
          lines.push(...docIssues.map((i: { severity: string; issue: string }) =>
            `- [${i.severity}] ${i.issue}`));
        }

        if (docRefs.length > 0) {
          lines.push("", "Cross-refs:");
          lines.push(...docRefs.map((r: { from: string; to: string; type: string }) =>
            `- ${r.from} -> ${r.to}: ${r.type}`));
        }

        if ((analysis.termDefinitions || []).length > 0) {
          lines.push("", "Canonical terms:");
          lines.push(...(analysis.termDefinitions || []).slice(0, 15).map(
            (t: { term: string; definition: string; doc: string }) =>
              `- **${t.term}**: ${t.definition} (${t.doc})`
          ));
        }

        if (phaseContext) {
          lines.push("", phaseContext);
        }

        lines.push(
          "",
          'Output: `{ "changes_made": [...], "issues_fixed": [...], "remaining_concerns": [...] }`',
        );

        return lines.join("\n");
      },
      parse: (text) => extractJsonObject(text),
    }),

    // ── 4. Validate consistency (always Claude — cross-doc reasoning) ───────
    validate_doc: acp({
      profile: "claude",
      session: { handle: "validation", isolated: true },
      statusDetail: "Validating consistency...",
      timeoutMs: 5 * 60_000,
      cwd: ({ outputs }) => (outputs.load_input as { worktree: string }).worktree,

      prompt: ({ outputs }) => {
        const { file, docsPath, analysis } = outputs.load_input as {
          file: string; docsPath: string; analysis: AnalysisResult;
        };

        const termLines = (analysis.termDefinitions || []).slice(0, 10).map(
          (t: { term: string; definition: string }) =>
            `- **${t.term}**: ${t.definition}`
        );

        return [
          `Validate ${file}. Using a team of 5 ultrathink agents:`,
          "A=internal consistency, B=cross-ref accuracy, C=terminology, D=numerical accuracy, E=completeness.",
          "",
          `Read ${docsPath}/${file} and spot-check referenced docs.`,
          ...(termLines.length > 0 ? ["", "Canonical terms:", ...termLines] : []),
          "",
          'Output: `{ "valid": true|false, "issues": [...], "suggestions": [...] }`',
        ].join("\n");
      },
      parse: (text) => {
        const parsed = extractJsonObject(text) as ValidationResult;
        return {
          valid: parsed.valid ?? true,
          issues: parsed.issues || [],
          suggestions: parsed.suggestions || [],
        };
      },
    }),

    // ── 5. Route: pass or retry (with max retry guard) ──────────────────────
    judge_valid: compute({
      run: ({ outputs }) => {
        const validation = outputs.validate_doc as ValidationResult;
        const config = outputs.load_input as { maxRetries: number; retryCount: number };

        if (validation.valid) {
          return { route: "pass" };
        }
        // Enforce max retries to prevent infinite loops
        if (config.retryCount >= config.maxRetries) {
          return { route: "pass" }; // Proceed despite issues — final validation catches them
        }
        // Increment retry counter
        config.retryCount++;
        return { route: "retry" };
      },
    }),

    // ── 6. Simplify (Claude Code — uses /simplify skill) ────────────────────
    simplify_doc: acp({
      profile: "claude",
      session: { handle: "simplify", isolated: true },
      statusDetail: "Running /simplify skill via Claude Code...",
      timeoutMs: 5 * 60_000,
      cwd: ({ outputs }) => (outputs.load_input as { worktree: string }).worktree,

      prompt: ({ outputs }) => {
        const { file, docsPath } = outputs.load_input as {
          file: string; docsPath: string;
        };

        return [
          `Run /simplify on ${docsPath}/${file}. Using a team of 5 ultrathink agents:`,
          "A=find redundancy, B=simplify complex sentences, C=cut filler, D=verify tables/code, E=guard accuracy.",
          "",
          "Use the /simplify skill. Do NOT change structure, cross-references, or numbers.",
          "",
          'Output: `{ "simplifications": [...], "lines_removed": N, "lines_added": N }`',
        ].join("\n");
      },
      parse: (text) => extractJsonObject(text),
    }),

    // ── 7. Regression check (grep for broken refs) ──────────────────────────
    regression_check: shell({
      statusDetail: "Checking regressions...",
      exec: ({ outputs }) => {
        const { file, docsPath } = outputs.load_input as {
          file: string; docsPath: string;
        };
        const basename = file.replace(/\.md$/, "");
        // Check that other docs referencing this one still have valid references
        return {
          command: "bash",
          args: ["-c", [
            `cd "${docsPath}"`,
            `echo "=== External references to ${file} ==="`,
            `grep -rn "${basename}" *.md | grep -v "^${file}:" | head -20 || echo "(none)"`,
            `echo "=== Broken links in ${file} ==="`,
            `grep -oP '\\[.*?\\]\\(\\K[^)]+\\.md' "${file}" 2>/dev/null | while read ref; do [ ! -f "$ref" ] && echo "BROKEN: $ref"; done || echo "(all links valid)"`,
          ].join(" && ")],
          shell: false,
        };
      },
      parse: (result) => ({
        output: result.stdout,
        hasBrokenLinks: result.stdout.includes("BROKEN:"),
        clean: !result.stdout.includes("BROKEN:"),
      }),
    }),

    // ── 8. Route: clean or fix regressions ──────────────────────────────────
    judge_regression: compute({
      run: ({ outputs }) => {
        const regression = outputs.regression_check as { hasBrokenLinks: boolean };
        return { route: regression.hasBrokenLinks ? "fix" : "pass" };
      },
    }),

    // ── 9. Capture diff for report ──────────────────────────────────────────
    collect_diff: shell({
      statusDetail: "Collecting diff...",
      exec: ({ outputs }) => {
        const { worktree } = outputs.load_input as { worktree: string };
        return {
          command: "bash",
          args: ["-c", `cd "${worktree}" && git add -A && git diff --cached --stat 2>/dev/null || echo "No changes"`],
          shell: false,
        };
      },
      parse: (result) => ({
        diffStat: result.stdout,
        hasChanges: !result.stdout.includes("No changes"),
      }),
    }),

    // ── 10. Commit and merge ────────────────────────────────────────────────
    merge_changes: shell({
      statusDetail: "Merging...",
      exec: ({ outputs }) => {
        const { file, projectRoot, worktree, branch } = outputs.load_input as {
          file: string; projectRoot: string; worktree: string; branch: string;
        };
        return {
          command: "bash",
          args: ["-c", [
            `cd "${worktree}"`,
            `git add -A`,
            `git diff --cached --quiet && echo "NO_CHANGES" && exit 0`,
            `git commit -m "docs: revise ${file} for consistency"`,
            `cd "${projectRoot}"`,
            `git merge "${branch}" --no-edit`,
            `git worktree remove "${worktree}" --force 2>/dev/null; true`,
            `git branch -D "${branch}" 2>/dev/null; true`,
            `echo "MERGED: ${file}"`,
          ].join(" && ")],
          shell: false,
        };
      },
      parse: (result) => ({
        merged: !result.stdout.includes("NO_CHANGES"),
        stdout: result.stdout,
      }),
    }),
  },

  // ── EDGES (11 edges, 2 loops) ─────────────────────────────────────────────
  edges: [
    { from: "load_input",        to: "create_worktree" },
    { from: "create_worktree",   to: "revise_doc" },
    { from: "revise_doc",        to: "validate_doc" },
    { from: "validate_doc",      to: "judge_valid" },
    {
      from: "judge_valid",
      switch: {
        on: "$output.route",
        cases: {
          pass:  "simplify_doc",
          retry: "revise_doc",              // ← LOOP 1: validation retry (max 2)
        },
      },
    },
    { from: "simplify_doc",      to: "regression_check" },
    { from: "regression_check",  to: "judge_regression" },
    {
      from: "judge_regression",
      switch: {
        on: "$output.route",
        cases: {
          pass: "collect_diff",
          fix:  "revise_doc",               // ← LOOP 2: fix broken links
        },
      },
    },
    { from: "collect_diff",      to: "merge_changes" },
  ],
});
