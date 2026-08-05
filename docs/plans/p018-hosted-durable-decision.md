# P-018 — Hosted durable checkpoint/output decision (G3)

- Status: **no-build** (closed for G3)
- Date: 2026-08-05
- Owner: APXM `agents`
- Issue: [agents#39](https://github.com/apxm-project/agents/issues/39)
- Authority: [ADR-0028](https://github.com/apxm-project/apxm/blob/main/docs/adr/0028-apxm-is-the-product-neutral-agent-program-and-inference-core.md), [ADR-0029](https://github.com/apxm-project/apxm/blob/main/docs/adr/0029-agent-program-source-and-closed-semantics-are-behavior-truth.md), [ADR-0030](https://github.com/apxm-project/apxm/blob/main/docs/adr/0030-execution-inference-evidence-and-deployment-are-exact-and-product-neutral.md); master-plan P-018 / G3

## Decision

**No-build** a hosted durable checkpoint/output reference implementation in APXM.

APXM keeps:

1. the product-neutral atomic [`ExecutionCommitPort`](../../crates/runtime/kernel/src/commit.rs) contract; and
2. mandatory owner-local **in-memory** and **filesystem** conformance adapters in
   [`apxm-commit-local`](../../crates/runtime/commit-local).

APXM does **not** ship a hosted (networked / multi-tenant / remotely retained)
checkpoint or output service, empty hosted abstraction, feature flag, or
speculative topology for that job.

## Decision dossier

- [x] Characterize checkpoint/output size, retention, restart and portability
      requirements (see bounds below; proven by owner-local suite).
- [x] Identify the exact customer-neutral job not met by current owner-local
      persistence — **none**. Downstream composition roots inject
      `ExecutionCommitPort` for any stronger durability topology.
- [x] If building: n/a (no-build).
- [x] If not building: prove owner-local persistence meets the stated bounds;
      confirm absence of hosted placeholders/flags/speculative services.
- [x] Record the decision and evidence before G3 closes.

## Owner-local bounds

| Concern | Owner-local bound | Evidence |
| --- | --- | --- |
| Size | Serialized tuple ≤ 8 MiB; larger fails closed | `tuple_size_bound_fails_closed` |
| Throughput | No throughput measurement or SLA is claimed; single-writer ownership is a correctness constraint | no benchmark is part of this decision |
| Retention | ≤ 10_000 idempotent commit results; directory lifetime for filesystem | `MAX_COMMIT_RESULTS` + suite |
| Restart | Filesystem store survives process reopen via atomic JSON rename | `filesystem_owner_local_survives_reopen_and_is_portable` |
| Portability | Directory of `execution-commit-local.v2.json` is the portable unit | copy/reopen in suite |

## Customer-neutral job analysis

| Candidate hosted job | Met by owner-local Port? | Notes |
| --- | --- | --- |
| Atomic checkpoint + output refs + evidence commit | Yes | `ExecutionCommitPort` five-member write set |
| Resume after process restart | Yes | filesystem adapter |
| Crash / replay / outcome_unknown without blind replay | Yes | suite + #36 G3 admission/runtime slice |
| Multi-tenant remote durability | Out of APXM | The accepted product-neutral boundary leaves stronger topology to an injected Port binding |
| Mandatory multi-service topology | Rejected | The accepted deployment decision keeps APXM composable and product-neutral |

No unmet product-neutral job remains that requires an APXM-hosted service beyond
the Port plus owner-local conformance.

## Permanent boundary

- Keep every contract, package, fixture, configuration default, image and
  release product-neutral.
- Do not import, fetch, name, configure, test against or release-pin any
  downstream product.
- Preserve one-way dependency: downstream products may consume released APXM
  artifacts; APXM never depends on them.
- Do not add aliases, dual paths, mixed generations, hidden fallbacks or
  product-plane services.
- Do not weaken Execution Admission behavior from agents#36.

## Relationship to G3 / agents#36

- agents#36 owns Execution Admission, effects, confinement, and atomic commit
  semantics; P-018 must not weaken that boundary.
- The explicit no-build result is now published in default-branch authority
  through this dossier, the evidence record, and the G3 checklist cross-link.
- Issue #39 may remain open for review or coordination, but the hosted
  durable build/no-build decision is no longer unpublished or conditional on a
  side branch.
