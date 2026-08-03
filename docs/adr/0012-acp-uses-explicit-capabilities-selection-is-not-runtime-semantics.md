---
status: accepted
date: 2026-07-16
decision: D-001G
owner: APXM agents
amended_by: ADR-0013
---

# ACP uses explicit Capabilities; selection is not runtime semantics

## Context

The prototype embeds ACP subprocess selection, `SPAWN_AGENT`, `COMMUNICATE`,
mutable agent profiles, and model fallback policy inside the runtime. That
conflicts with accepted Program composition, five-operation AIR, exact injected
adapters, explicit context, and the rule that the runtime cannot invent a
conversation, loop, provider, model, or retry.

APXM must support external ACP agents in v1 and preserve a clean semantic
boundary for future APXM-owned model or External Agent routing. The question is
where those effects appear in an Agent Program and how the compiler/runtime
preserve authority and evidence without adding another operation family.

## Decision

This owner ADR implements the exact-binding and no-routing rules in
[workspace ADR-0030](../../../../docs/adr/0030-execution-inference-evidence-and-deployment-are-exact-and-product-neutral.md).

### No AIR expansion

AIR remains exactly:

```text
model.call
capability.invoke
program.new
program.invoke
await.event
```

ACP session operations are ordinary typed Capabilities. V1 requires explicit
exact model and External Agent Profile selection. Future External Agent route
selection may be a typed Capability, and future model route selection may be a
typed pre-dispatch phase of `model.call`; neither is an AIR operation, hidden
frontend function call, or runtime fallback.

FrontendGraph and the artifact record only:

- exact Capability Definition references for external-agent operations;
- exact content-addressed `ModelTargetRef` on each v1 model call;
- declared request/result types and effect classes;
- required exact adapter/profile contract digests; and
- source maps and evidence annotations.

They contain no shell command, process id, endpoint guess, package range,
mutable agent/model name, credential, fallback list, or host placement.

### External ACP agents

The source contract is an equivalent typed library over `capability.invoke`:

```python
session = await external_agents.open(CodeReviewAgentProfile)
try:
    review = await session.prompt(CodeReviewPrompt(diff=diff))
finally:
    await session.close()
```

```typescript
const session = await externalAgents.open(CodeReviewAgentProfile);
try {
  const review = await session.prompt({ diff });
} finally {
  await session.close();
}
```

These are target contract examples, not claims about the prototype frontend.
Each wrapper call lowers to `capability.invoke`. The `ExternalAgentSessionRef`
is a scoped opaque Capability handle whose durable session record is bound to
one Program Instance/root lineage (or one one-shot Program Invocation), Acting
Principal, invoking Agent Identity, exact profile, and grant lease. The same
Program Instance may retain it in typed Program Context across invocations. It
cannot be serialized into artifact metadata, escape in output, be forged, act
as a Program Instance, or cross an owner/principal/profile/lease boundary.

Opening a session negotiates ACP protocol and capabilities against one exact
signed External Agent Profile. Prompt input is an explicit typed projection;
the ACP peer receives no automatic AAM preamble, entire Program Context,
parent transcript, Skill content, environment, credential, or filesystem.

ACP client-directed file, terminal and permission requests are separate nested
Capability invocations carrying the
original External Agent Profile, session, Acting Principal, Agent Identity,
root Program Invocation, resource, operation and effect lineage. The adapter
cannot implement `ApproveAll` as authority. An ACP permission response only
communicates the result of the outer admission/approval decision for that
exact reverse request. An attenuated Capability gateway may be admitted
separately; it is never an ACP reverse method. Agent-native tools that remain
inside the peer stay inside the outer effect under mandatory profile
confinement and attributed evidence.

The ACP peer's prompt loop, model calls, internal subagents and private state
remain one external Capability effect from APXM's perspective. APXM records
protocol updates, reported Tool/plan/message data, reverse requests, usage and
terminal outcome as attributed external evidence; it never fabricates APXM
NodeExecutions for the peer's internals or treats reported usage as
authoritative APXM usage evidence without source/provenance classification.

### Internal Program selection

Selection among APXM Agent Programs remains ordinary source control flow over
a finite set of statically imported typed references:

```python
route = await select_specialist(request)  # pure source or admitted Capability
match route:
    case Specialist.RESEARCH:
        result = await Researcher.invoke(request)
    case Specialist.REVIEW:
        result = await Reviewer.invoke(request)
```

There is no Agent Router inside `program.invoke`. `ProgramRef` and
`ExternalAgentProfileRef` are distinct non-convertible types. A route result
cannot contain an arbitrary Program reference.

### Exact model selection and future routing

The v1 author chooses an exact model:

```python
answer = await model.call(request, model=ExactModel)
```

```typescript
const answer = await model.call(request, {
  model: ExactModel,
});
```

The compiler preserves the exact portable model-target requirement. The
selected signed Runtime Profile and Deployment Composition Manifest predeclare
exactly one `ModelTargetRef -> ModelDeploymentRef -> ExactPortBindingRef`
mapping. The Composition Root, through the library-owned
`verify_deployment_composition(...)` contract, validates and materializes one
immutable `ResolvedModelBinding` before runtime dispatch. It never searches,
ranks, or substitutes candidates. Runtime validates and invokes only that
binding. Zero, duplicate, missing, unavailable, or ineligible is a typed
failure and does not activate a default target.

APXM will implement its own model router as future work. A future frontend may
also accept an exact `ModelRoutePolicyRef`; the compiler will bind the policy
and candidate-set digests, and a separately accepted ADR will define the
pre-dispatch decision owner. The runtime will receive only the resulting
immutable `ResolvedModelBinding`. No third-party router is an APXM dependency
or architectural owner.

If dispatch may have occurred, the router cannot select another model for the
same effect id. Safe retry requires proof from the selected adapter's exact
idempotency/reconciliation contract; otherwise the node ends as
`ModelOutcomeUnknown`. Circuit breaker state may exclude a candidate before
dispatch but is never described as successful failover.

The future route decision is immutable evidence with policy/candidate/model/backend
digests, hard-constraint results, rejected candidates, calibrated score or
deterministic ranking inputs, budget/price facts and effect id. The model
response and usage facts reference it but do not overwrite it.

### Adapter and profile admission

`agents` owns the stable ACP Client and inference ports, exact model/profile
reference types, lifecycle invariants, source bindings, product-neutral
admission verification, and evidence semantics. For v1 it owns the
`ModelTargetRef` and `ResolvedModelBinding` validation/evidence contracts.
An outer Composition Root supplies one pre-signed deployment composition,
opaque external correlations, and exact admitted bindings; it invokes the
shared verifier owned by `agents`. The Composition Root is not an
implementation resolver: the selected composition already names the exact
mapping. Only after future routing is separately promoted by a new ADR may a
pre-dispatch policy decision produce the immutable binding before runtime
dispatch. Runtime/adapter own only the live transport/process and runtime only
validates and executes the immutable model binding. Concrete Claude, Codex,
other exact admitted ACP, and inference implementations are separately
admitted adapters outside the runtime kernel. A future pure learned scorer may
be an admitted adapter, but it cannot dispatch or override admission.

An adapter is usable only when the Compatibility Set pins its exact production
Implementation Descriptor, artifact, dependencies, platform, Port Contract,
protocol/capability matrix, confinement/network ceiling, limits,
SBOM/provenance, and conformance evidence. An `ExternalAgentProfile` is a
separate deployment composition that fixes the exact peer artifact/command or
endpoint, ACP adapter binding, credential-method reference,
workspace/confinement, and limits. Runtime Profile data predeclares the
required typed slots; the Deployment Composition Manifest fixes the actual
profile and resources. A source-level display name cannot resolve an
executable.

APXM's Claude claim is for the exact `claude-agent-acp` adapter, which may use
the Claude Agent SDK internally. Its Codex claim is for an exact admitted Codex
ACP adapter. APXM core depends on neither vendor SDK. Neither
claim is restated as a native vendor protocol guarantee. Generic ACP support
is a migration/development input only; production support means an exact
immutable profile passed its negotiated matrix, not universal behavioral
equivalence among agents.

### Lifecycle and failure

- Session open is replay-safe per logical Capability occurrence and never
  creates duplicate live peers after recovery without reconciliation.
- A prompt is single-flight per session unless the exact profile contract
  explicitly proves concurrency; the default is fail-busy.
- Program cancellation requests cancellation of the active ACP prompt, fences new reverse
  requests, waits a bounded deadline, then closes/terminates according to the
  adapter contract.
- Process exit, stream EOF, protocol error, adapter crash, timeout, denied
  reverse request, unconfirmed cancel and unconfirmed close are distinct typed
  outcomes.
- No error silently launches a different profile, command, model, transport,
  Confinement implementation, or local fake responder.
- `session.close` alone terminates the APXM external-agent session; `lost` and
  `outcome_unknown` are terminal and later cleanup cannot rewrite them as
  `closed`. V1 pools no process/session across principals or Program Instances.
- Session output, protocol transcript references and peer-created files follow
  the same classified per-NodeExecution output and purpose-bound access rules.

## Prototype disposition

The full replacement deletes or rebuilds:

- `SPAWN_AGENT`, `COMMUNICATE`, `HANDOFF`, `DELEGATE`, `INV(acp)` and their
  handlers/builders/examples;
- runtime `AgentRouter`, ProcessTable authority, `agent_route = "auto"`,
  `preferred_profiles` and string recipient dispatch;
- automatic AAM/system-preamble injection;
- handwritten ACP schema/constants that are not generated or verified against
  the pinned ACP v1 schema;
- host-local environment credential lookup and ACP `ApproveAll` authority;
- unpinned `npx` commands, mutable package ranges and generic arbitrary-command
  production profiles;
- automatic model fallback tags/profile chains and post-send substitution; and
- any evidence mapping that calls an ACP prompt an APXM Turn or trusts
  peer-reported usage as authoritative APXM metering, including mapping ACP
  `usage_update.used/size` into native input/output tokens, calling the native
  model accountant, or fabricating a model identity for the outer Capability.

The target ACP adapter may reuse proven framing, lifecycle and Confinement code
only after it satisfies the new interfaces and conformance suite. No reader,
alias, translator or dual path preserves the old operation model.

## Consequences

The normative ACP/routing contract and full-replacement plan become required
companions to the Agent Program composition/AIR contract. Python/TypeScript
goldens must prove that external-agent wrappers emit only
`capability.invoke`, exact model selection emits only `model.call`, and
internal specialist choice emits ordinary control flow plus `program.invoke`.
Release gates cover exact Claude, Codex and other admitted ACP adapters, protocol
skew/capability omission, reverse-request authority, cancellation/close,
Confinement, supply chain and outcome uncertainty. The separate future-routing
plan adds routing evaluation gates before any route reference becomes
executable.

Canonical v1 contains no route-policy, candidate-set, scorer, route-decision,
or resolver-port field. The future plan is non-executable architecture work;
adding any such schema requires a new accepted ADR and Compatibility Set.
