/**
 * PR Code Review Flow
 *
 * A 6-node acpx workflow that reviews a pull request:
 *   1. load_pr (compute) — Parse input
 *   2. fetch_diff (action/shell) — Get PR diff via gh CLI
 *   3. review_code (acp) — Agent reviews the diff
 *   4. judge_verdict (compute) — Route based on review verdict
 *   5. post_approval (action/shell) — Approve the PR
 *   6. post_changes (action/shell) — Request changes on the PR
 *
 * Usage:
 *   acpx flow run ./review.flow.ts \
 *     --input-json '{"repo":"owner/repo","prNumber":42}' \
 *     --approve-all
 */
import { defineFlow, acp, compute, shell, extractJsonObject } from "acpx/flows";
import { embedSkills } from "./lib/utils.js";

type ReviewInput = {
  repo: string;
  prNumber: number;
  projectRoot?: string;
};

type ReviewOutput = {
  verdict: "approve" | "request_changes";
  summary: string;
  comments: string[];
};

const SESSION_HANDLE = "review";

export default defineFlow({
  name: "pr-review",
  startAt: "load_pr",

  permissions: {
    requiredMode: "approve-all",
    reason: "Flow executes shell commands and agent actions autonomously",
  },

  nodes: {
    load_pr: compute({
      run: ({ input }) => {
        const { repo, prNumber, projectRoot } = input as ReviewInput;
        return {
          repo,
          prNumber,
          projectRoot: projectRoot || process.cwd(),
        };
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
        linesChanged: result.stdout.split("\n").length,
      }),
    }),

    review_code: acp({
      session: { handle: SESSION_HANDLE },
      statusDetail: "Agent reviewing code...",
      timeoutMs: 5 * 60_000,
      cwd: ({ outputs }) => (outputs.load_pr as { projectRoot: string }).projectRoot,

      prompt: ({ outputs }) => {
        const pr = outputs.load_pr as { repo: string; prNumber: number; projectRoot: string };
        const diff = outputs.fetch_diff as { diff: string; linesChanged: number };

        const skills = embedSkills(pr.projectRoot, ["review"]);

        return [
          skills,
          "",
          "## Task",
          "",
          `Review the following PR diff (${diff.linesChanged} lines) from ${pr.repo}#${pr.prNumber}.`,
          "",
          "Analyze for:",
          "1. Correctness — does the code do what it intends?",
          "2. Security — buffer overflows, injection, unsafe operations",
          "3. Style — does it follow project conventions?",
          "4. Performance — any obvious regressions?",
          "",
          "Output your review as JSON:",
          "```json",
          '{',
          '  "verdict": "approve" | "request_changes",',
          '  "summary": "one-line summary",',
          '  "comments": ["comment 1", "comment 2"]',
          '}',
          "```",
          "",
          "## Diff",
          "",
          "```diff",
          diff.diff.slice(0, 50_000),
          "```",
        ].join("\n");
      },

      parse: (text) => {
        const parsed = extractJsonObject(text) as ReviewOutput;
        return {
          verdict: parsed.verdict || "request_changes",
          summary: parsed.summary || "No summary provided",
          comments: parsed.comments || [],
        };
      },
    }),

    judge_verdict: compute({
      run: ({ outputs }) => {
        const review = outputs.review_code as ReviewOutput;
        return { route: review.verdict };
      },
    }),

    post_approval: shell({
      statusDetail: "Approving PR...",
      exec: ({ outputs }) => {
        const pr = outputs.load_pr as { repo: string; prNumber: number };
        const review = outputs.review_code as ReviewOutput;
        return {
          command: "gh",
          args: [
            "pr", "review", String(pr.prNumber),
            "--repo", pr.repo,
            "--approve",
            "--body", review.summary,
          ],
          shell: false,
        };
      },
    }),

    post_changes: shell({
      statusDetail: "Requesting changes...",
      exec: ({ outputs }) => {
        const pr = outputs.load_pr as { repo: string; prNumber: number };
        const review = outputs.review_code as ReviewOutput;
        const body = [
          review.summary,
          "",
          ...review.comments.map((c: string) => `- ${c}`),
        ].join("\n");
        return {
          command: "gh",
          args: [
            "pr", "review", String(pr.prNumber),
            "--repo", pr.repo,
            "--request-changes",
            "--body", body,
          ],
          shell: false,
        };
      },
    }),
  },

  edges: [
    { from: "load_pr", to: "fetch_diff" },
    { from: "fetch_diff", to: "review_code" },
    { from: "review_code", to: "judge_verdict" },
    {
      from: "judge_verdict",
      switch: {
        on: "$output.route",
        cases: {
          approve: "post_approval",
          request_changes: "post_changes",
        },
      },
    },
  ],
});
