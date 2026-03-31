/**
 * PR Triage Flow — Multi-Step Agent Pipeline
 *
 * A more comprehensive workflow that triages PRs through multiple steps:
 *   1. load_pr (compute) — Parse input
 *   2. fetch_context (action/shell) — Get PR diff + metadata
 *   3. extract_intent (acp) — Agent extracts PR intent
 *   4. classify (compute) — Route: bug fix, feature, docs, or close
 *   5. test_changes (acp) — Agent verifies changes (bug/feature path)
 *   6. final_review (acp) — Agent does final review
 *   7. post_result (compute) — Prepare final output
 *   8. comment_and_close (action/shell) — Close low-quality PR
 *
 * Usage:
 *   acpx flow run ./triage.flow.ts \
 *     --input-json '{"repo":"owner/repo","prNumber":42}' \
 *     --approve-all
 */
import { defineFlow, acp, compute, shell, extractJsonObject } from "acpx/flows";
import { embedSkills } from "./lib/utils.js";

type TriageInput = {
  repo: string;
  prNumber: number;
  projectRoot?: string;
};

const SESSION_HANDLE = "triage";

export default defineFlow({
  name: "pr-triage",
  startAt: "load_pr",

  permissions: {
    requiredMode: "approve-all",
    reason: "Flow executes shell commands and agent actions autonomously",
  },

  nodes: {
    load_pr: compute({
      run: ({ input }) => {
        const { repo, prNumber, projectRoot } = input as TriageInput;
        return {
          repo,
          prNumber,
          projectRoot: projectRoot || process.cwd(),
        };
      },
    }),

    fetch_context: shell({
      statusDetail: "Fetching PR context...",
      exec: ({ outputs }) => {
        const pr = outputs.load_pr as { repo: string; prNumber: number };
        return {
          command: "gh",
          args: [
            "pr", "view", String(pr.prNumber),
            "--repo", pr.repo,
            "--json", "title,body,labels,files,additions,deletions,changedFiles",
          ],
          shell: false,
        };
      },
      parse: (result) => {
        const metadata = JSON.parse(result.stdout);
        return { metadata };
      },
    }),

    fetch_diff: shell({
      statusDetail: "Fetching PR diff...",
      exec: ({ outputs }) => {
        const pr = outputs.load_pr as { repo: string; prNumber: number };
        return {
          command: "gh",
          args: ["pr", "diff", String(pr.prNumber), "--repo", pr.repo],
          shell: false,
        };
      },
      parse: (result) => ({
        diff: result.stdout,
      }),
    }),

    extract_intent: acp({
      session: { handle: SESSION_HANDLE },
      statusDetail: "Extracting PR intent...",
      timeoutMs: 3 * 60_000,
      cwd: ({ outputs }) => (outputs.load_pr as { projectRoot: string }).projectRoot,

      prompt: ({ outputs }) => {
        const pr = outputs.load_pr as { repo: string; prNumber: number; projectRoot: string };
        const ctx = outputs.fetch_context as { metadata: Record<string, unknown> };
        const diff = outputs.fetch_diff as { diff: string };

        return [
          `## Task: Extract Intent for PR #${pr.prNumber}`,
          "",
          "Analyze this PR and determine its intent.",
          "",
          "### PR Metadata",
          "```json",
          JSON.stringify(ctx.metadata, null, 2).slice(0, 5000),
          "```",
          "",
          "### Diff (first 30k chars)",
          "```diff",
          diff.diff.slice(0, 30_000),
          "```",
          "",
          "Output JSON:",
          "```json",
          '{',
          '  "intent": "one-line description of what this PR does",',
          '  "category": "bug_fix" | "feature" | "docs" | "refactor" | "low_quality",',
          '  "risk": "low" | "medium" | "high",',
          '  "reason": "why you classified it this way"',
          '}',
          "```",
        ].join("\n");
      },

      parse: (text) => {
        const parsed = extractJsonObject(text) as Record<string, unknown>;
        return {
          intent: parsed.intent || "unknown",
          category: parsed.category || "feature",
          risk: parsed.risk || "medium",
          reason: parsed.reason || "",
        };
      },
    }),

    classify: compute({
      run: ({ outputs }) => {
        const intent = outputs.extract_intent as { category: string };
        if (intent.category === "low_quality") {
          return { route: "close" };
        }
        return { route: "verify" };
      },
    }),

    test_changes: acp({
      session: { handle: SESSION_HANDLE },
      statusDetail: "Verifying changes...",
      timeoutMs: 5 * 60_000,
      cwd: ({ outputs }) => (outputs.load_pr as { projectRoot: string }).projectRoot,

      prompt: ({ outputs }) => {
        const pr = outputs.load_pr as { projectRoot: string };
        const intent = outputs.extract_intent as {
          intent: string;
          category: string;
          risk: string;
        };

        const skills = embedSkills(pr.projectRoot, ["build", "test"]);

        return [
          skills,
          "",
          "## Task: Verify Changes",
          "",
          `This PR is a **${intent.category}** (risk: ${intent.risk}).`,
          `Intent: ${intent.intent}`,
          "",
          "Please:",
          "1. Build the project to check for compilation errors",
          "2. Run the test suite",
          "3. Check if the changes match the stated intent",
          "",
          "Output JSON:",
          "```json",
          '{',
          '  "builds": true | false,',
          '  "tests_pass": true | false,',
          '  "matches_intent": true | false,',
          '  "issues": ["issue 1", "issue 2"]',
          '}',
          "```",
        ].join("\n");
      },

      parse: (text) => {
        const parsed = extractJsonObject(text) as Record<string, unknown>;
        return {
          builds: parsed.builds ?? true,
          tests_pass: parsed.tests_pass ?? true,
          matches_intent: parsed.matches_intent ?? true,
          issues: (parsed.issues as string[]) || [],
        };
      },
    }),

    final_review: acp({
      session: { handle: SESSION_HANDLE },
      statusDetail: "Final review...",
      timeoutMs: 3 * 60_000,
      cwd: ({ outputs }) => (outputs.load_pr as { projectRoot: string }).projectRoot,

      prompt: ({ outputs }) => {
        const pr = outputs.load_pr as { projectRoot: string };
        const intent = outputs.extract_intent as {
          intent: string;
          category: string;
          risk: string;
        };
        const verification = outputs.test_changes as {
          builds: boolean;
          tests_pass: boolean;
          matches_intent: boolean;
          issues: string[];
        };

        const skills = embedSkills(pr.projectRoot, ["review"]);

        return [
          skills,
          "",
          "## Task: Final Review",
          "",
          `PR category: **${intent.category}** (risk: ${intent.risk})`,
          `Intent: ${intent.intent}`,
          "",
          "### Verification Results",
          `- Builds: ${verification.builds ? "PASS" : "FAIL"}`,
          `- Tests: ${verification.tests_pass ? "PASS" : "FAIL"}`,
          `- Matches intent: ${verification.matches_intent ? "YES" : "NO"}`,
          verification.issues.length > 0
            ? `- Issues: ${verification.issues.join(", ")}`
            : "- No issues found",
          "",
          "Based on the verification results, provide your final verdict.",
          "",
          "Output JSON:",
          "```json",
          '{',
          '  "verdict": "approve" | "request_changes" | "escalate",',
          '  "summary": "one-line summary for the PR comment",',
          '  "comments": ["detailed comment 1", ...]',
          '}',
          "```",
        ].join("\n");
      },

      parse: (text) => {
        const parsed = extractJsonObject(text) as Record<string, unknown>;
        return {
          verdict: parsed.verdict || "escalate",
          summary: parsed.summary || "",
          comments: (parsed.comments as string[]) || [],
        };
      },
    }),

    post_result: compute({
      run: ({ outputs }) => {
        const intent = outputs.extract_intent as { intent: string; category: string };
        const review = outputs.final_review as {
          verdict: string;
          summary: string;
          comments: string[];
        };
        return {
          intent: intent.intent,
          category: intent.category,
          verdict: review.verdict,
          summary: review.summary,
          comments: review.comments,
        };
      },
    }),

    comment_and_close: shell({
      statusDetail: "Closing PR...",
      exec: ({ outputs }) => {
        const pr = outputs.load_pr as { repo: string; prNumber: number };
        const intent = outputs.extract_intent as { reason: string };
        return {
          command: "gh",
          args: [
            "pr", "close", String(pr.prNumber),
            "--repo", pr.repo,
            "--comment", `Closing: ${intent.reason}`,
          ],
          shell: false,
        };
      },
    }),
  },

  edges: [
    { from: "load_pr", to: "fetch_context" },
    { from: "fetch_context", to: "fetch_diff" },
    { from: "fetch_diff", to: "extract_intent" },
    { from: "extract_intent", to: "classify" },
    {
      from: "classify",
      switch: {
        on: "$output.route",
        cases: {
          verify: "test_changes",
          close: "comment_and_close",
        },
      },
    },
    { from: "test_changes", to: "final_review" },
    { from: "final_review", to: "post_result" },
  ],
});
