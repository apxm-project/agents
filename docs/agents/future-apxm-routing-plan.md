# Future APXM-owned model and External Agent routing plan

- Status: planned future work; not executable APXM v1 semantics
- Decision boundary: [ADR-0012](../adr/0012-acp-uses-explicit-capabilities-selection-is-not-runtime-semantics.md)
- Current prerequisite: [ACP interoperability and exact-selection plan](acp-and-routing-full-replacement-plan.md)

## 1. Intent

APXM will implement its own inspectable router after exact-selection v1 is
complete and evidenced. Lemonade and RouteLLM informed the questions around
gateway visibility, cost/quality calibration and reproducible decisions; they
are not dependencies, adapters, compatibility promises or owners.

The router has two separate products:

1. a model router that resolves an immutable policy to one exact model
   deployment before `model.call` dispatch; and
2. an External Agent router Capability that selects one exact admitted ACP
   profile before source opens a session.

There is no APXM Program router. Program composition remains source-authored.

## 2. Preconditions

Work starts only after:

- a new routing ADR is accepted after v1 exact-selection evidence; this plan
  alone authorizes no schema or implementation;
- v1 exact model/profile references and catalogues are authoritative;
- runtime ModelRouter/AgentRouter/fallback code is absent;
- Server produces immutable exact bindings;
- Auth, budget, price and locality facts are queryable through owner contracts;
- exact-selection outcome-unknown and no-fallback suites pass; and
- evaluation has representative tasks, quality judgments, cost/latency facts
  and protected-subgroup/risk slices.

## 3. Contract design

The contract phase defines, without enabling execution:

- `apxm.model-route-policy.v1`;
- `apxm.model-route-candidate-set.v1`;
- `apxm.model-route-decision.v1`;
- `apxm.external-agent-route-policy.v1`;
- `apxm.external-agent-route-decision.v1`;
- `ResolvedModelBinding` provenance extensions; and
- simulator, evaluation-claim and drift-state schemas.

A policy pins hard eligibility, finite candidates, objective, price/budget
rules, deterministic tie-breaker, scorer/evaluation digests and no-eligible
behavior. A decision records every selected/rejected candidate and exact input
fact. Secret values and private prompt content are excluded or transformed
under an explicitly evaluated feature contract.

## 4. Owner architecture

| Owner | Future responsibility |
| --- | --- |
| `agents` | authoring/compiler route-reference types plus the `ResolvedModelBinding` validation and evidence contract |
| Server | catalogue snapshots, resolution, hard eligibility, budget/price admission, final pre-dispatch decision and immutable binding |
| runtime | validate and execute the immutable binding; never select or score a target |
| Auth | authorization/data-locality ceiling and secret custody |
| `adapters` | optional pure deterministic/learned scorer, with no dispatch or authority |
| `eval` | benchmark design, calibration, quality/cost/safety claims, subgroup tests and drift gates |
| Studio | policy builder, simulator, compare/explain, promotion and evidence views |

## 5. Delivery stages

1. **Offline simulator:** resolve recorded requests against frozen catalogues;
   no production dispatch.
2. **Deterministic policies:** hard filters plus transparent configured ranking;
   shadow decisions only.
3. **Evaluation gate:** compare exact-selection baselines and preregister
   quality, cost, latency, calibration, safety and fairness thresholds.
4. **Learned scorer research:** train/version scorers on permitted features,
   publish claim cards and adversarial/distribution-shift results.
5. **Shadow production:** produce decisions and counterfactuals without
   changing the exact selected target.
6. **Explicit opt-in:** enable policy refs only for approved companies/programs
   and candidate sets; exact refs remain semantically unchanged.
7. **Continuous monitoring:** drift, regret, constraint violations, spend,
   latency and outcome-unknown rates can disable the policy, never substitute
   a target after send.

## 6. Non-negotiable failure rules

- No eligible target returns a typed route-unavailable result.
- A scorer error cannot activate a default or first candidate.
- A health change after the decision cannot silently reroute.
- Dispatch happens once, only after decision admission.
- After possible send, no other model/profile may receive the same effect.
- Disabling a route policy stops new decisions but preserves prior evidence.
- Rollback means publishing a new policy/Compatibility Set or disabling the
  policy; it never changes an existing decision record.

## 7. Studio and evidence

Studio must show policy version/digest, exact candidate snapshot, hard filters,
scores/objective, rejected reasons, price/budget facts, selected target,
confidence/calibration bounds, evaluation claim, shadow-versus-actual result,
drift state and terminal effect evidence. Administrators can simulate, compare,
approve, disable and audit; they cannot edit an immutable published policy.

## 8. Promotion gate

Routing becomes executable only after deterministic replay, mutation, skew,
authority, locality, budget, calibration, quality, safety, subgroup, drift,
load, cancellation and outcome-unknown suites pass and a signed Compatibility
Set pins the policy/scorer/catalogue/evaluation closure under that new accepted
ADR. Until then, all
route-reference source examples are explicitly non-executable future examples.
