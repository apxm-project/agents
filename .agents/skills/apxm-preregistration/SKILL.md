---
name: apxm-preregistration
description: Use before any claim-bearing APXM run (benchmark, evaluation, paper-bound number). Drafts, commits, and verifies a docs/preregistrations/ entry under the project's append-only norm.
user-invocable: true
---

# APXM Preregistration

Load `_shared/apxm-preregistration-rules.md` and
`_shared/apxm-evaluation-rules.md` before broad work.

## When this skill is required

The run is *claim-bearing* if its output backs:

- The APXM paper (anything in `docs/paper/`).
- A claim card under `docs/claims/`.
- A benchmark number cited in a PR description, README, or `docs/`.
- A "X is faster than Y" / "X matches Y in quality" assertion.

If none apply, this skill is optional (but still useful for honesty).

## Steps

1. **Read 2–3 existing preregistrations** in `docs/preregistrations/`
   matching the workload type (priority-lane, review-council, tau2,
   cross-system, J/req). Match style and section ordering.
2. **Draft the file** at
   `docs/preregistrations/<YYYYMMDDTHHMMSS>Z-<plan>-<scenario>-<arm>.md`
   using the template in `_shared/apxm-preregistration-rules.md`.
3. **Confirm the protocol with the user** — explicitly, before
   committing. Surface goal, primary metric, success threshold,
   exclusions, and the artifact path.
4. **Commit** with a message in repo style:
   `prereg(planNN): <plan> <scenario> — <descriptor>`.
5. **Record the commit SHA**. It will be cited in the write-up under
   `docs/evaluation/`.
6. **Hand off** to the run-execution skill (e.g.
   `apxm-priority-lane-bench`, `apxm-review-council-bench`) only after
   the preregistration commit lands.

## What `apxm-finish` will check

- Matching file exists in `docs/preregistrations/`.
- Preregistration commit timestamp is *before* the first artifact in
  `.apxm/evaluation/<scenario>/runs/<UTC>/`.
- Write-up cites the preregistration commit SHA.

## Anti-patterns

- Drafting the prereg *after* the run — the entire point is to freeze
  the protocol before seeing the numbers.
- Rewriting a committed prereg. Append a `-corrective` or `-restart`
  file with a reference instead.
- Drafting without recording the APXM and vLLM-submodule commit SHAs.
- Drafting without an "Exclusions / known confounds" section — known
  confounds left implicit always come back to bite the analysis.

## See also

- `docs/preregistrations/` — the existing corpus.
- `apxm-claim-evidence` — the downstream skill that promotes a
  successful run into a claim card.
