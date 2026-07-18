# Workflow optimization roadmap

> **Archived pre-canonical optimization evidence; non-authoritative.** Direct
> AIR, cognition-op fusion, and prototype memory transforms below are retired
> by [ADR-0009](../adr/0009-air-has-five-public-semantic-operations.md).
> Canonical v1 optimization work must first update the v1 contract and P3/P9
> plan.

This document records the prototype optimization design. Any canonical v1
optimization is complete only when its compiler analysis, runtime behavior,
frontend parity, generated consumers, permission/effect semantics, replay
behavior, documentation, tests, and evaluation evidence agree.

The governing rule is correctness before profitability. A transform must first
prove that dependencies, prompt roles, effects, authority, approvals,
determinism, idempotency, replay constraints, and backend capabilities permit
the change. Token, cost, latency, quality, parallelism, and cache reuse are
considered only after legality is established.

## Milestone 0: truthful foundations

The compiler pipeline separates the following responsibilities:

- MLIR transformation passes mutate IR.
- Analyses compute reusable facts without mutating IR.
- Artifact validation rejects malformed or unsupported output.
- Finalizers serialize and publish completed artifacts.
- Diagnostics report failures without masquerading as transformations.

O3 iteration is bounded and stops at convergence. Shared-prefix metadata has
one backend-independent meaning. Prompt inputs carry typed roles for system,
user, tool, control, and dependency-only content so lowering and optimization
cannot synthesize, reorder, or prune them as interchangeable strings.

Python and TypeScript frontends must lower through FrontendGraph to equivalent
compiler-owned AIR contracts. Production compilation rejects incomplete DSPy behavior;
experimental DSPy work cannot silently alter production artifacts.

Acceptance evidence:

- pass-manager tests distinguish transforms, analyses, validation, finalizers,
  and diagnostics;
- bounded-convergence tests cover stable and changing O3 pipelines;
- differential artifacts prove frontend parity;
- prompt-role tests cover preservation and illegal pruning;
- production tests prove incomplete optimization backends fail closed.

## Reusable legality analyses

The pipeline exposes reusable analyses for:

- prompt contracts and typed input roles;
- effect and authority requirements;
- DAG use and dependency readiness;
- token and context cost;
- profile cost and backend availability;
- backend legality and capability support.

Legality decisions include dependency shape, side effects, active grants,
approval policy, determinism, idempotency, replay safety, confinement, and
backend capabilities. Analyses are cached by stable artifact identity and are
invalidated when their inputs change.

Optimization summaries are emitted with artifacts so runtime and evaluation
surfaces can explain which transforms were applied, rejected, or left
experimental.

## Dependency-driven workflow execution

Workflow execution is readiness-driven rather than phase-barrier-driven. A
node becomes runnable when its typed dependencies and authority constraints are
satisfied. The scheduler uses weighted critical paths to prioritize ready work
without violating dependencies, effect order, grants, approvals, or replay
boundaries.

Checkpoint placement follows typed effect and replay evidence. Memoization and
batching are enabled only when compiler analysis proves them legal. Cache keys
include the inputs and implementation identity required to prevent semantic or
authority aliasing.

Acceptance evidence includes dependency-order traces, critical-path tests,
effect-order tests, approval and permission parity, memoization rejection tests,
and replay tests for both accepted and rejected boundaries.

## Prompt, context, and usage planning

Context assembly, tool-result budgeting, conversation compaction, prompt
segmentation, model-call reservation, and usage reconciliation share one typed
planning model. Planning preserves mandatory prompt roles and dependency-only
inputs, accounts for backend context limits, and records observed rather than
invented usage.

Model selection comes from configured backend policy and capability evidence.
The compiler and runtime do not embed provider names, deployment profiles,
output roots, or machine-specific paths.

Acceptance evidence includes request-boundary assertions, context-plan events,
tool-result budget tests, compaction tests, reservation reconciliation, and
observed token and latency metrics.

## Offline prompt evaluation

Status: the local contract validator is implemented in
[`tools/scripts/offline_prompt_evaluation.py`](../../tools/scripts/offline_prompt_evaluation.py).
It validates bundle integrity and evidence binding; it is not a completion
claim for a backend, runtime, or cross-repository optimization rollout.
The observed execution surface is implemented in
[`tools/scripts/observed_prompt_evaluation.py`](../../tools/scripts/observed_prompt_evaluation.py).
It executes fixed prompt arms through an explicitly registered backend, records
portable request and provider-response files, binds them through execution
records and receipts, and then invokes the offline validator. Its focused tests
are in
[`tools/tests/test_observed_prompt_evaluation.py`](../../tools/tests/test_observed_prompt_evaluation.py),
and the preregistered live bundle is under
[`evaluation/workflow-optimization-observed/`](../../evaluation/workflow-optimization-observed/).

Prompt optimization runs offline against held-out, preregistered evaluation
bundles. Production execution does not self-modify prompts from live outcomes.
The evidence contract requires a dataset identity and revision, a split policy,
separate digest-and-cardinality commitments for optimization and held-out
inputs, distinct baseline and candidate identities, structured backend
evidence, and portable provenance. Backend evidence identifies a backend id and
revision, classifies itself as either a deterministic fixture or an observed
backend run, and uses the matching measurement source. Provenance contains only
a portable owner/repository identifier, lowercase revision digest, and safe
bundle identifier. It also fixes one supported quality metric and the minimum
candidate-mean and delta thresholds used to accept a candidate.

The runner rejects a bundle unless both input digests and cardinalities match
the preregistration, optimization and held-out case ids are disjoint, each
result row identifies its preregistered arm, and token-count and latency fields
are present and valid. Output rows have closed schemas: arbitrary emitted
metadata is rejected. It rejects unstructured backend evidence, a measurement
source that conflicts with its evidence kind, and provenance that can embed
machine-local paths. Evidence records only bundle-relative input paths, content
digests, the declared portable provenance, the preregistration reference, the
evaluated working-tree fingerprint, and the runner digest; it does not embed
machine-specific paths or require network-only state. The resulting summary
records the computed decision, quality scores, the declared measurement source,
an explicit telemetry classification, and token and latency totals and means
for both arms.

A deterministic fixture declares `measurement_source=fixture-values`; its
summary marks `is_backend_telemetry=false`. Its token and latency fields are
contract inputs, never model telemetry, and do not support live-backend quality,
cost, or performance claims. An observed backend run can declare
`measurement_source=backend-telemetry` only when every arm/case output carries
a portable SHA-256 evidence reference. The referenced receipt must bind the
preregistered backend id/revision, arm id, case id, output digest, token count,
and latency value. Each receipt also binds a request digest and a recorded
backend-execution document. That document references the exact portable request
and provider response, records the provider response identity and model, and
binds the preregistered backend capability evidence. A claim-bearing observed
run additionally requires current owner gates.
The focused evidence tests
[`tools/tests/test_offline_prompt_evaluation.py`](../../tools/tests/test_offline_prompt_evaluation.py)
pin the canonical repository fixture, closed output schemas, receipt-binding
rejections, portable provenance records, and aggregate backend-classified
measurement fields across more than one held-out case. The required
`dekk agents test` owner gate invokes this focused suite through
[`.dekk.toml`](../../.dekk.toml).

## Durable effects and replay

Link v1 capability calls remain non-authoritative for replay. A replayable host
effect uses the negotiated AHI v2 prepare/commit path:

1. Runtime supplies execution, graph, node, invocation, dispatch, grant, and
   approval identity.
2. Server persists verified approval evidence.
3. OS binds the preparation to the active host attachment and verifies the
   host-signed commit.
4. Server durably stores the exact preparation, commit, enrolled public key,
   and content-free public receipt before observers see the receipt.
5. Partial replay may skip the prior host effect only after loading that
   durable record and re-verifying preparation/commit/receipt binding plus the
   Ed25519 signature over the UTF-8 effect digest.

Native, Python, TypeScript, and Link v1 effects remain replay-ineligible unless
an owning implementation later supplies an equally strict evidence contract.
Missing, conflicting, malformed, or unverifiable evidence fails closed.

## Experimental work

The following remain experimental and are disabled unless explicitly selected
by an experiment surface:

- ASK fusion;
- untyped memory condensation;
- semantic caching;
- speculative execution.

Experimental results are not production capability claims and do not relax
permission, effect, approval, determinism, or replay checks.

## Completion gates

The roadmap is complete only when all of the following are present:

- focused compiler, frontend, runtime, scheduler, permission, effect, and replay
  tests;
- generated Rust, TypeScript, and Python consumers refreshed from their owning
  contracts;
- differential artifacts and execution traces;
- backend-classified token and latency measurements, with deterministic fixture
  values excluded from real-backend quality, cost, and performance claims and
  observed values bound to portable receipts plus owner evidence;
- held-out quality results with preregistered bundles and the owning backend's
  execution attestation;
- backend capability evidence;
- current documentation for compiler, runtime, AHI, and Host SDK behavior;
- repository-local Dekk checks and tests passing for every affected owner.

Passing compilation alone is not completion. A completion claim must point to
the evidence bundle that records these gates and the revisions evaluated.
