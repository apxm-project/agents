---
name: execute-plan
group: Lifecycle
description: Drive a written APXM plan to completion without scope creep. Runs focused per-phase verification, refuses to add features beyond the plan, and surfaces blockers immediately.
user-invocable: true
---

# APXM Execute Plan

Load `_shared/apxm-development-rules.md` before broad work (it points at
the comment and test rules).

Execute against a written plan with disciplined progress and no scope creep.
Use the authorization already established for the task; ask only when scope
or an irreversible external action is genuinely unclear.

## What this skill does

1. **Restate the plan's phase boundaries** at the start. Be explicit
   about what each phase produces and where it ends.
2. **Keep phase progress explicit** in the task update or handoff.
   State each phase's verification result as soon as it is complete; do not
   batch unrelated work into one completion claim.
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
1. State the phase and its expected output.
2. Read the relevant file(s).
3. Make the edit.
4. Run the phase's verification command.
5. If green: report the result and continue; if red, stop and summarize.
```

Use the smallest correct command:

- Touched a `crates/<x>` source? Run that crate's named `test-*` recipe
  (`dekk agents --help`); there is no bare `test -p <x>` scope.
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
