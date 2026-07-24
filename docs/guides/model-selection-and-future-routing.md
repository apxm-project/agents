# Choose models and understand future routing

- Architectural status: exact selection is canonical v1; routing is planned
  future work
- Frontend syntax status: implemented source-first Model binding aligned with
  [Author an Agent](creating-an-agent-program.md)
- Future plan: [APXM-owned routing](../agents/future-apxm-routing-plan.md)

## V1: choose exactly

Every `model.call` names one immutable content-addressed `ModelTargetRef`
supplied by the admitted project catalogue. That portable reference pins the
contractual model/checkpoint, configuration and deployment-eligibility
requirements without embedding an endpoint, credential or environment
deployment id. Server binds it once to one exact admitted deployment before
dispatch.

If the deployment is unauthorized, over budget, unhealthy or unavailable, the
call fails with a typed error. APXM does not choose a default, alias, first
healthy target or fallback. If a send may have occurred, APXM never tries a
second model; it reconciles the same target or returns
`ModelOutcomeUnknown`.

Python declares and calls the exact target through `Model`:

```python
from apxm_program import Model

SupportModel = Model[SupportRequest, SupportResponse]("model.support.v1")
response = await SupportModel(request)
```

TypeScript uses the same source concept:

```typescript
import { Model } from "@apxm/frontend";

const SupportModel = Model<SupportRequest, SupportResponse>(
  "model.support.v1",
);
const response = await SupportModel(request);
```

Calling the typed value records a typed Model-invocation intent. Rust selects
`model.call`; the frontend does not connect to a provider or write the AIR/AIS
operation name.

Admission may bind the exact target to APXM-vLLM or another conforming
`ModelInferencePort` implementation. That choice does not change source or
AIR. Backend-specific graph/prefix/priority hints are admitted implementation
metadata, while response, usage, cancellation, failure, and evidence retain the
same provider-neutral contract.

## Future: APXM-owned policies

APXM will later implement its own model and External Agent routing. Lemonade
and RouteLLM are examples studied for routing visibility and evaluation, not
dependencies or adapters.

The future author will opt in explicitly with an immutable policy reference.
The policy names a finite exact candidate set, hard constraints, objective,
budget/price rules, deterministic tie-breaker and evaluation claim. Server
will decide once before dispatch and provide runtime one exact immutable
binding. Studio will explain selected and rejected candidates.

External Agent routing will be a distinct Capability returning one exact ACP
profile decision. Source must then open the session explicitly. Agent Program
selection will continue to use ordinary source control flow and typed
`ProgramRef`s; it will never use either router.

Do not write production source against a route-policy API until the future
plan's contract, shadow, evaluation, drift and Compatibility Set gates have
passed.
