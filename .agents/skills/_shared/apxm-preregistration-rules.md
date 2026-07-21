# Shared rule — APXM preregistration rules

Load before any session that will run a claim-bearing benchmark or
evaluation (see `apxm-evaluation-rules` for the definition).

## Norm

Every claim-bearing run requires a **committed** preregistration
*before* the run starts. The preregistration freezes the protocol — what
you're measuring, how you're measuring it, what counts as success — so
the result can't be reverse-engineered from the data.

## File layout

- Location: `evaluation/<scenario>/preregistration.json`.
- Append-only. Once committed, do not edit. If the protocol changes,
  create a new scenario bundle and retain the prior bundle as evidence.

## Minimum required fields

- Scenario and dataset identity.
- Disjoint optimization and held-out split digests and case counts.
- Stable baseline and candidate arm identifiers.
- Backend evidence kind and exact backend revision.
- Portable repository, revision, and bundle provenance.
- Metric and decision rule fixed before execution.

Use `dekk agents offline-prompt-evaluation` for deterministic recorded
bundles or `dekk agents observed-prompt-evaluation` for registered backend
execution.

Skipping preregistration makes the run unable to back a claim, regardless
of how good the numbers look.

## `finish` interlock

`finish` will refuse to claim "done" on a claim-bearing run if:

- No matching committed `evaluation/<scenario>/preregistration.json` exists.
- The preregistration's commit timestamp is *after* the first artifact
  in `.apxm/evaluation/<scenario>/runs/<UTC>/`.
- The decision summary doesn't cite the preregistration digest.

## Existing examples

See `evaluation/workflow-optimization/` and
`evaluation/workflow-optimization-observed/`.
