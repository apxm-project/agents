# Requirements Quality Checklist — 0002-apxm-chat-thin-clients

Self-check on `spec.md` (spec-specify gate). Re-run after clarify.

## Content quality

- [x] No implementation detail in `spec.md` requirements (HOW lives in `plan.md`).
- [x] Every FR is a testable MUST statement.
- [x] Success criteria are measurable and tech-agnostic (counts, pass rates, grep
      guards), not "API under Nms".
- [x] Scope is bounded (server contract + CLI thin pipe; no Studio/OS edits).
- [x] Assumptions state 0001 gate, 0006 deferral, OpenAPI tool choice deferred to plan.

## Requirement completeness

- [x] No `[NEEDS CLARIFICATION]` markers remain (clarifications folded in §Clarifications).
- [x] Each user story (US1–US7) has Why / Independent Test / acceptance scenarios.
- [x] Edge cases cover backpressure, permission timeout, concurrent approve, compact
      in-flight, stale resume, unknown session.
- [x] FRs trace to SCs: FR-001/002→SC-001/002; FR-003/004→SC-003; FR-005→SC-004;
      FR-006/007→SC-005; FR-008/009→SC-006; FR-010→SC-007; FR-011/012→SC-008;
      FR-014+ suites→SC-009.

## Constitution alignment (apxm v1.0.0)

- [x] Transport parity (#1) = FR-014, US7 acceptance.
- [x] Host is a pipe (#2) = FR-004, US7, Assumptions on in-program loop.
- [x] AIR-portability (#3) = session state server-side; program unchanged.
- [x] No dead surface (#7) = typed errors replace opaque strings; session API wired.
- [x] Green invariant suites (#10) = SC-009 explicit gate.

## Open clarifications for spec-clarify (resolved)

1. 0006 scope split — resolved: Wave 2 after 0002 (roadmap §6).
2. Studio out of scope — resolved: 0003 consumer only.
3. Legacy CLI loop — resolved: preserved for non-in-program artifacts only.
