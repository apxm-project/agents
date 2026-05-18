# Dogfooding harness — APXM evaluating its own development artifacts

Status: **scaffold only (2026-05-18)**.

## Purpose

The most credible agent-quality claim APXM can make is the one where
APXM is the customer: it runs against the team's own GitHub issues,
PRs, and release artifacts, with the team's own labels / merge
decisions / accept-as-is rates as ground truth.

This directory hosts three Dekk commands that exercise APXM against
APXM's own dev artifacts. They are deliberately the most modest agent
tasks APXM can run — small N, clear ground truth, no LLM-judge
contamination.

## Commands (target)

### `dekk apxm dogfood triage`

Feeds the last N days of GitHub issues into an APXM graph that:
1. classifies them (bug / feature / question / noise),
2. suggests assignees from a CODEOWNERS-style ownership table,
3. emits a Markdown digest.

**Ground truth**: human-applied issue labels and assignee fields after
the digest is published. Agreement rate is the headline number.

### `dekk apxm dogfood review`

Feeds an open PR into an APXM graph that runs the project's existing
review checklist:
- test coverage (touched files have test changes?),
- no-legacy-vllm lint (`tools/scripts/check_no_legacy_vllm.py`
  references any new code in `crates/` or `tools/`?),
- CHANGELOG entry (`[Unreleased]` updated when public surface
  changes?),
- claim discipline (if any file in `docs/claims/` is touched, the PR
  description must cite the pre-registration).

Posts a draft review comment.

**Ground truth**: human-reviewer agreement rate (does the human
reviewer accept the draft's findings, modify them, or reject them?).

### `dekk apxm dogfood release-notes`

Synthesizes `CHANGELOG.md` `[Unreleased]` from the commit range since
the last tag. Output is a Markdown diff against the current
`[Unreleased]` block.

**Ground truth**: maintainer accept-as-is rate when the draft is
proposed for the next release commit.

## Implementation gates

- [ ] Each command is a `dekk apxm dogfood <subcommand>` entry in
      `.dekk.toml`.
- [ ] Each command consumes a live APXM graph (built via the standard
      `apxm.GraphRecorder` / `@compile` Python frontend), dispatched
      against the registered zoo backend selected via
      `APXM_BENCHMARK_BACKEND` (matches the convention from
      `examples/python/benchmarks/workloads/_config.py`).
- [ ] Each command writes a structured JSON report at
      `.apxm/evaluation/agentic/dogfood/<TIMESTAMP>/<cmd>.json` with
      the task-quality column set: `inputs`, `outputs`,
      `ground_truth`, `agreement_rate`, `pass_at_1`, `mean_steps`,
      `mean_tool_calls`, `dag_critical_path_ms`.
- [ ] Manifest line written alongside the report citing
      `apxm_sha`, `vllm_fork_sha`, `zoo_snapshot_ref`,
      `eval_window_start`, `eval_window_end`, `n_artifacts`.

## Methodology constraints

Methodology constraints:

1. **No LLM-judge grading.** Every quality measurement must reduce to
   a binary agreement against a human decision (label, assignee,
   accept-as-is). LLM-as-judge inflates accuracy claims and is
   explicitly out of scope.
2. **Pre-registration before the run.** The
   pre-registration file in `docs/preregistrations/` must commit
   BEFORE the eval window starts. The window's start timestamp is the
   git commit time of the pre-registration.
3. **Honest negatives.** Cells where APXM-on does NOT improve over
   flat-http on agreement rate MUST be reported.

## Why the scaffold ships before the implementation

The dogfooding commands are referenced from MASTER but did not exist.
The scaffold + this README give the next agent a concrete attach point.
Implementation begins when an operator explicitly takes this work.

A complementary approach already exists for one slice:
`tools/scripts/apxm_plan_as_graph_dogfood.py` (different scope — that
script tests plan-as-graph emission on real prompts, not issue
triage / PR review / release notes). The dogfooding commands
are the broader surface; the plan-as-graph dogfood script is
complementary, not redundant.
