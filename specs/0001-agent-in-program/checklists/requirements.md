# Requirements Quality Checklist — 0001-agent-in-program

Self-check gate for `spec.md` before handing off to `spec-clarify`.

## No implementation detail
- [x] No stack, framework, API, op, file, or code-structure names appear in the
      spec (behavior described as capabilities, not mechanisms).
- [x] "Host" is described by role (delivers input, renders output), not by
      transport or process detail.
- [x] Context/compaction described as outcomes (stay within limit, retain
      important info), not by internal algorithm.

## Every requirement testable
- [x] FR-001…FR-014 are each a checkable MUST with an observable outcome.
- [x] Each user story has an Independent Test and Given/When/Then scenarios.
- [x] Edge cases enumerated with the expected (testable) behavior.

## Success criteria measurable & tech-agnostic
- [x] SC-001…SC-007 are measurable (counts, turn thresholds, pass rates) and
      user-focused.
- [x] No SC references latency-of-implementation or internal metrics; SC-007
      states the no-recompute property in user-observable terms.

## Scope bounded
- [x] Scope is the conversational agent's authorability and host-agnostic
      behavior; capability installation, credentials, and skill catalogue
      internals are explicitly out of scope (Assumptions).
- [x] Incremental-adoption boundary stated (current host-driven agent remains).

## Assumptions stated
- [x] Reference hosts, "important info", session identity, reuse of existing
      mechanisms, and backward compatibility are all recorded as Assumptions
      rather than left implicit.

## Clarification markers
- [x] Zero `[NEEDS CLARIFICATION]` markers in spec.md; genuinely ambiguous scope
      choices are deferred to `spec-clarify` (the next phase) rather than guessed.

## Open items for spec-clarify
- [x] First-release boundary — RESOLVED 2026-06-15: finish it all, no reduced
      milestone; in-program loop is in scope. (See spec.md Clarifications.)
- [x] Lifecycle rule failure semantics — RESOLVED in plan.md: gate→fail-closed+
      surface, observe→surface+continue.
- [x] Sub-agent default execution — RESOLVED in plan.md: in-process default,
      ACP-profile opt-in.
