---
name: execute-plan
group: Lifecycle
description: Drive an APXM plan to completion without scope creep. Tracks phases with the harness's task tracker, runs focused per-phase verification, refuses to add features beyond the plan, and surfaces blockers immediately. Invoke only after plan produces an approved plan.
user-invocable: true
---

# APXM Execute Plan

Load `_shared/apxm-development-rules.md` before broad work (it points at
the comment and test rules).

Execute against an approved plan with disciplined progress and no scope
creep. Invoke only after `plan` and an explicit user approval.

## What this skill does

1. **Restate the plan's phase boundaries** at the start. Be explicit
   about what each phase produces and where it ends.
2. **Track phases with the current harness's task tracker**.
   Mark each `in_progress` when starting and `completed` as soon as
   done — do not batch.
3. **After each phase, run focused verification** from the plan's
   "Verification per phase" section. Do not run the full workspace
   when a per-crate check suffices.
4. **Refuse scope creep**. If something tempting but out-of-scope
   appears (a refactor, a comment cleanup, a docs fix), surface it as
   a follow-up task and keep going. Do not silently expand the plan.
5. **Surface blockers immediately**. If a verification fails, a
   precondition turns out wrong, or the approach hits a dead end:
   stop, summarize, and re-invoke `plan` rather than working
   around the blocker.
6. **Honor the AIS codegen contract**. After any `.td` or
   TableGen-shim edit, run `dekk agents build-dialect` then
   `dekk agents codegen` *before* the next phase begins.

## Per-phase rhythm

```
1. Mark task in_progress.
2. Read the relevant file(s).
3. Make the edit.
4. Run the phase's verification command.
5. If green: mark completed, mark next task in_progress, repeat.
6. If red: stop, summarize, ask for guidance.
```

Use the smallest correct command:

- Touched a `crates/<x>` source? Run that crate's named `test-*` recipe
  (`dekk agents --help`); `dekk agents test -p <x>` does not scope anything.
- Touched the CLI? `dekk agents test-cli`.
- Touched Python frontend? `dekk agents test-python-frontend`.
- Touched `.td`? `build-dialect && codegen` *then* the test commands.
- Touched anything that lint might care about?
  Run the relevant focused Dekk check for the changed surface.

## Anti-patterns

- Running the full workspace test when one crate changed.
- Marking a task completed when verification failed.
- "Bonus" edits not in the plan ("while I'm in this file I'll also
  fix…"). Even good ideas wait for the next plan.
- Continuing past a failed Dekk check without addressing or
  acknowledging it.
- Skipping `build-dialect`/`codegen` after `.td` edits and then being
  confused by phantom type errors.

## Done condition

All plan phases completed and verified. Then hand off to
`simplify`.
