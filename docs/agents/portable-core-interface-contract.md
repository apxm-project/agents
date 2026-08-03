# APXM portable core interface contract

- Status: canonical APXM v1 owner contract
- Date: 2026-07-16
- Decision: [ADR-0013](../adr/0013-core-semantics-are-closed-and-implementations-enter-through-exact-port-bindings.md)
- Product-neutral scope: [ADR-0028](../../../../docs/adr/0028-apxm-is-the-product-neutral-agent-program-and-inference-core.md)
  (apxm#148)
- Semantic authority: [ADR-0029](../../../../docs/adr/0029-agent-program-source-and-closed-semantics-are-behavior-truth.md)
  (apxm#148)
- Execution and exact-binding authority: [ADR-0030](../../../../docs/adr/0030-execution-inference-evidence-and-deployment-are-exact-and-product-neutral.md)
  (apxm#148)
- Owner: APXM `agents`
- Applies to: contracts/types, AIS, artifact, compiler, runtime, CLI and
  Composition Root wiring, inference/Capability/ACP/handler/confinement/store/
  observer adapters
- Migration: full replacement; no old registry, default, fallback, alias, or
  mixed binding system

## 1. Purpose

This contract defines how `agents` keeps Agent Program meaning fixed while
allowing infrastructure implementations to be replaced. Frontends, compiler,
artifact admission, runtime, product-neutral Invocation Admission, adapters,
inference backends, and evidence must implement one Program Execution Model. A
new implementation may change performance, placement, protocol mechanics, or
attributable evidence; it may not change the meaning or schema of output,
lifecycle, authority, or canonical evidence. A real model or external system
may return a different valid domain value. Exact output equality is required
only for a scripted deterministic fixture with the same injected inputs.

## 2. Normative vocabulary

The canonical terms are in [`CONTEXT.md`](../../CONTEXT.md).

- **Closed Semantic Type**: finite exhaustive APXM meaning for one contract
  digest.
- **Port Contract**: one focused replaceable boundary and its complete typed
  semantics.
- **Implementation Descriptor**: exact signed proof for one implementation of
  one Port Contract.
- **Artifact Semantic Requirement**: authored model, Capability, handler,
  event, or confinement behavior required by one artifact.
- **Deployment Port Slot**: Runtime Profile data naming one exact Port Contract,
  Implementation Descriptor, generated binding-payload contract, and closed
  feature set for one deployment concern.
- **Exact Port Binding**: admitted immutable requirement-to-implementation
  Deployment-Port-Slot-to-implementation association with one generated
  per-port binding payload and binding-admission proof.
- **Runtime Profile**: immutable Compatibility Set data selected explicitly by
  the Composition Root; it is not an admission actor or resolver.
- **Deployment Composition Manifest**: deployment-specific materialization of
  one selected Runtime Profile.
- **Execution Admission** (agents synonym: **Invocation Admission**): the
  product-neutral signed admission envelope from workspace ADR-0030 and the
  master-plan admission contract—exact artifact and invocation identities,
  context digest, resource ceilings, exact Port bindings, exact model target
  when present, expiry/nonce, issuer/audience, and opaque caller correlations.
  It carries no downstream product schema.
- **Composition Root**: outer composition root (ADR-0029) that selects one
  Runtime Profile, supplies exact admitted bindings and deployment resources,
  invokes the shared verifier, and owns constructed implementations.
- **Execution Commit Port**: one atomic authoritative commit of Program state,
  continuation/checkpoint, effect facts, canonical evidence, and output refs.
- **Runtime Instance**: one isolated semantic kernel constructed from a
  Verified Deployment Composition and caller-owned resources.

“Backend” alone is non-normative. Use the precise term: model inference
implementation, confinement implementation, state store, event transport,
telemetry sink, or external system behind an adapter.

## 3. Representation rules

### 3.1 Closed enums and tagged unions

Use closed generated types when a value controls APXM meaning:

- AIR and structural operation kinds;
- lifecycle, cancellation, effect, delivery, and terminal states;
- Hook events and callback phases;
- diagnostic and failure classes;
- authority/effect/risk classifications;
- retry/reconciliation/send states;
- confinement actions and feature values;
- evidence event and usage-provenance kinds; and
- port feature states for an exact contract version.

Rules:

1. one owner schema defines discriminants and wire spellings;
2. Rust matches exhaustively; Python/TypeScript generated unions use a
   `never`/equivalent exhaustiveness test;
3. storage, event, HTTP, evidence, and evidence projections use generated values;
4. semantic/security unions have no generic `Other(String)`, `Unknown(raw)`,
   `Custom`, string fallback, or catch-all behavior; owner-defined epistemic
   variants such as `OutcomeUnknown`, `UsageUnknown`, and
   `CancellationUnconfirmed` remain required closed meaning;
5. an unknown value fails at the earliest contract/admission boundary; and
6. a new variant changes the owner digest and Compatibility Set.

Opaque namespaced metadata is allowed only as attributable evidence. Core must
not branch on it.

### 3.2 Typed references and values

Do not use enums for an extensible implementation catalogue. Do not use a
generic string either. Each family uses a non-convertible typed ref whose value
is content-addressed or bound to a descriptor digest.

Examples of type categories, not frozen Rust identifiers:

```text
ModelImplementationRef
CapabilityImplementationRef
ConfinementImplementationRef
StateStoreImplementationRef
EventTransportImplementationRef
TelemetryImplementationRef
```

Likewise use distinct newtypes for contract ids/digests, Program/Instance/
Invocation/NodeExecution/effect ids, money, duration/deadline, byte/token
limits, endpoint/resource refs, secret refs, and evidence refs. No implicit
conversion joins unrelated authority or implementation identities.

### 3.3 No boolean or string policy soup

A boolean is acceptable only for a truly binary fact with no lifecycle or
future semantic branches. Use a closed state type when `enabled`, `disabled`,
`pending`, `revoked`, `unsupported`, or `unknown` would otherwise be encoded by
several booleans or strings. Configuration does not accept generic
`HashMap<String, Value>` for behavior-affecting fields.

## 4. What remains concrete

These are fixed semantic ownership, not ports:

- FrontendGraph validation and source-map meaning;
- the closed five-operation effect/composition family and separate closed
  AIS-owned structural family including `ais.loop`;
- canonical compiler pass ordering and deterministic optimization behavior;
- artifact encoding/admission invariants;
- graph scheduling and dependency semantics;
- Program Instance/Invocation/yield/return state machines;
- NodeExecution and attempt identity;
- static Hook callback placement, Agent Facade and context flow;
- Capability Grant validation/enforcement and effect commit protocol;
- authored loop identity and committed-iteration evidence meaning;
- canonical cancellation and terminal-commit rules; and
- canonical evidence meaning.

An interface is rejected when its implementations could change one of these.

## 5. Port Contract minimum

Every owner port freezes the applicable parts of this contract and explicitly
marks inapplicable concerns rather than inventing placeholder lifecycle or
effect states:

```text
identity: contract id, digest, owner
types: request, response/stream, closed error/outcome
lifecycle: states, transitions, concurrency, ordering
control: deadline, cancellation, backpressure, drain, shutdown
effects: stable id, send boundary, idempotency, retry, reconcile, uncertainty
security: authority, secret, data, path, network, process, platform ceilings
evidence: canonical facts, attributable facts, redaction
configuration: typed schema, limits, sensitive fields
features: closed version-scoped vocabulary
tests: non-admissible fake, positive, negative, skew, race, crash,
       recovery vectors
```

Methods return typed domain results. They do not throw implementation-specific
exceptions across the boundary, leak provider response objects, or accept an
untyped provider request escape.

## 6. Compiler contract

### 6.1 Inputs and outputs

Public compilation accepts complete in-memory values/bytes/readers plus
explicit compiler options, target/toolchain identity, cancellation, limits,
and source maps. It returns an admitted artifact or closed diagnostics.

It never reads cwd, `PATH`, credentials, ambient service registries, endpoints,
a running external service, or provider configuration. CLI, PyO3, Node-API, and
remote compile bridges are explicit transports to the same compiler contract;
none is a fallback for another.

### 6.2 Semantic requirements in artifacts

The compiler records authored semantic requirements, not deployment ports or
implementations:

```text
ArtifactSemanticRequirements {
  model_targets: ModelTargetRequirement[]
  capabilities: CapabilityDefinitionRequirement[]
  handlers: HandlerRequirement[]
  durable_events: EventSemanticRequirement[]
  confinement: ConfinementFeatureRequirement[]
}
```

Effect and risk classes come from the exact signed model or Capability owner
contract, never from a generic compiler-supplied port field. Source limits are
typed requested ceilings or semantic bounds and never authority. Artifact
requirements contain no state/evidence/observer implementation slot, endpoint,
credential, executable, process, package range, mutable alias, fallback list,
or host placement.

### 6.3 Target-specific lowering

Canonical semantic validation and lowering stay concrete. If APXM admits more
than one real target code generator, the target boundary must accept the same
validated target-neutral IR plus one exact target descriptor and return the
same semantic artifact contract. It may report unsupported closed features; it
cannot add AIR operations, rewrite authority/effects, or make a provider
choice.

No speculative target port is introduced before this condition exists.

## 7. Binding and construction contract

### 7.1 Release, deployment, and invocation scopes

The three scopes are disjoint:

| Scope | Contains | Excludes |
| --- | --- | --- |
| Compatibility Set | exact contracts, libraries, implementation artifacts and descriptors, Runtime Profiles, platform matrices, signatures, SBOM/provenance, and conformance | deployment resource refs, secret refs, credential leases, grants, resource ceilings, invocation ids |
| Deployment Composition Manifest | one explicitly selected Runtime Profile, generated per-port binding payloads, exact configuration digests, stable opaque resource refs, and binding-admission evidence for one installation | Capability Grants, credential leases, resource-ceiling reservations, resolved model effects, execution evidence |
| Execution Admission (Invocation Admission) | exact artifact and invocation identities, context digest, resource ceilings, exact Port bindings, exact model target when present, expiry/nonce, issuer/audience, opaque caller correlations, leases/limits/cancellation/lineage | implementation search, replacement profile, deployment configuration mutation, downstream product schema |

The Runtime Profile is immutable Compatibility Set data. It predeclares exactly
one Implementation Descriptor digest and one generated binding-payload contract
per Deployment Port Slot. The Composition Root selects one profile and supplies
one Deployment Composition Manifest. The library-owned
`verify_deployment_composition` path verifies:

- Port Contract id/digest equality;
- implementation artifact/source and binding-configuration digests;
- required feature subset;
- platform/dependency constraints;
- authority/secret/data/network/process/failure ceilings;
- signatures, SBOM, provenance, vulnerability policy, and conformance evidence;
- the profile's one-to-one slot mapping; and
- each generated per-Port-Contract binding payload.

An embedded Composition Root and a reference runtime Composition Root call this
exact same verifier. The verifier consumes the selected profile and manifest
only; it never searches a catalogue, ranks candidates, chooses a default, or
tries another implementation. Zero, duplicate, missing, or mismatched slot
mappings fail verification. Display names and package coordinates are
presentation only.

### 7.2 Deployment binding

For each Deployment Port Slot, successful verification produces:

```text
ExactPortBinding<BindingPayload> {
  slot: TypedPortSlot,
  deployment_requirement_digest: Digest,
  implementation_descriptor: Digest,
  binding_payload: BindingPayload,
  binding_payload_digest: Digest,
  binding_admission_proof: Digest,
  binding_admission_evidence_ref: EvidenceRef,
}
```

`BindingPayload` is generated from that Port Contract and uses exact typed
configuration and stable opaque resource refs; there is no universal resource
bag. It contains no Capability Grant, credential lease, resource-ceiling reservation,
Invocation/NodeExecution/effect id, resolved model effect, or execution
evidence. The resulting `PortBindingSet` is immutable for that deployment
composition. Revocation or expiry fails the affected operation according to the
owner contract and never triggers rebinding.

### 7.3 Runtime port bundle

The public runtime constructor accepts the Verified Deployment Composition plus
caller-owned instance resources. Its typed bundle uses focused fields or typed
family collections. Collections are keyed by family-specific typed refs and
contain only verified bindings. There is no `Map<String, Box<dyn Any>>`,
universal backend trait, global registry, or late-bound provider lookup.

Construction validates the Runtime Profile's deployment slots and fails before
execution. Artifact Semantic Requirements are checked separately against the
verified composition, and Invocation Admission is checked for every invocation.
An optional deployment port is optional only when the profile does not require
its behavior; absence cannot silently disable a semantic obligation.

### 7.4 Validation phases

- Release intake verifies the Compatibility Set and its exact Runtime Profiles.
- The selected Composition Root invokes `verify_deployment_composition` to
  verify the profile/manifest/descriptor/binding closure and construct exact
  implementations. Embedded and reference-runtime Composition Roots use this
  same path.
- Runtime construction verifies the resulting proof/digests, typed slot-to-
  contract equality, and complete port bundle only. It performs no catalogue
  access, admission, resolution, or search.
- Execution Admission (Invocation Admission) verifies the artifact
  requirements, product-neutral admission facts, resource ceilings, deadlines,
  resolved model binding, and current authority facts without changing the
  deployment composition.
- The effect boundary verifies the current Capability Grant,
  credential/resource lease, revocation, and resource ceilings required by the
  owner contract. It never resolves or rebinds an implementation.

### 7.5 Dispatch style

The contract is independent of Rust dispatch technique. Use generics/static
dispatch for homogeneous performance-sensitive embedding; focused trait
objects for runtime-selected admitted implementations; and generated clients
for process boundaries. Do not commit to a native plugin ABI or dynamic loader
without a separate accepted ADR and conformance contract.

## 8. Runtime Port families

### 8.1 Model inference port

Input includes exact `ResolvedModelBinding`, its referenced verified inference
Port Binding, request/effect id, typed Model Context envelope, response
contract, stream contract, limits, cancellation, and evidence correlation.
Authority and resource-ceiling denial occur during Invocation Admission before
this port is called. A provider refusal is declared `ModelOutput` data, not an
authority decision.

The adapter returns attempt facts, not a source-level result envelope:

```text
ModelAdapterAttemptOutcome =
  NotSent(reason)
  | Accepted(stream_or_receipt)
  | DeliveredTypedFailure(error)
  | ProvenNotAccepted(reason)
  | TransportLost(after_possible_accept)
  | CancellationConfirmed
  | CancellationUnconfirmed

ModelReconciliationResult =
  ProvenCommitted(output, usage_provenance)
  | ProvenNotCommitted
  | StillUnknown(usage_provenance)
```

Runtime maps those facts to the closed Model NodeExecution outcome: committed
success, typed failure, cancelled, or `ModelOutcomeUnknown`. `reconciled` is not
a terminal variant. The Agent Program receives only its declared
`ModelOutput<T>` on committed success or its authored typed error/control-flow
behavior; attempt, usage, reconciliation, and uncertainty metadata remain
canonical evidence.

`DeliveredTypedFailure(error)` is a reconciled terminal attempt fact: runtime
preserves the typed failure, does not retry it, and does not degrade it to
`ModelOutcomeUnknown`. Streaming uses one closed, ordered boundary:

```text
ModelStreamEvent =
  ContentDelta(sequence, content_ref)
  | ToolCallDelta(sequence, content_ref)
  | Heartbeat(sequence)

ModelStreamStep =
  Event(ModelStreamEvent)
  | Terminal(ModelOutcome)
```

Content references remain typed references; neither the adapter seam nor
runtime projects them into inline text. Runtime requires contiguous event
sequence, receives exactly one terminal step, and passes its cancellation token
into every streaming transport step so dependency-supported in-flight
cancellation is observable. Cancellation before dispatch commits `Cancelled`.
After dispatch, a racing event is discarded as `ModelOutcomeUnknown`; an exact
terminal result remains authoritative, including a transport-confirmed
`Cancelled`. Runtime never fabricates confirmed cancellation for an uncertain
external effect.

Runtime rejects any mismatch among `ModelTargetRef`, `ModelDeploymentRef`,
`ResolvedModelBinding` and the inference Exact Port Binding digest. Adapter
identity appears only through that Exact Port Binding rather than a second
duplicated model-binding field.

The port may expose owner-defined prepare/send/stream/reconcile/cancel
operations, but runtime owns node/effect identity. It cannot select another
model, invoke provider-native Tools as APXM Capabilities, or conceal uncertain
send/usage state.

### 8.2 Capability execution port

Input includes exact Capability Definition/implementation binding, operation,
typed args, Acting Principal, Agent Identity, Capability Grant, effect id,
limits, cancellation, and evidence correlation. The final implementation
enforces the complete grant and current credential/resource lease over the real
resource boundary. A pure or read-only Capability returns its declared domain
result. An effectful Capability reports the owner-defined attempt facts
(`not_sent`, `accepted`, `proven_not_accepted`, `transport_lost`, and
cancellation confirmation) plus reconciliation to `proven_committed`,
`proven_not_committed`, or `still_unknown`. Runtime maps those facts to the
Capability NodeExecution outcome; no generic `reconciled` value enters the
program result.

It cannot mint or widen authority, infer permission from Skill/context, or
return provider objects outside the declared result schema.

### 8.3 Program Artifact Access Port

Program artifact access accepts only an exact admitted typed ProgramRef and
returns its immutable artifact/admission proof. It does not own Program
Instance state, invocation authority, deployment composition, catalogue access,
or candidate selection. A missing exact ProgramRef is a typed failure.

### 8.4 Execution Commit Port

The Execution Commit Port is the only authoritative persistence boundary for
Program execution. One compare-and-commit request includes:

- expected Program Instance and Invocation revision/fence;
- the exact legal lifecycle transition;
- next Program Context, runtime state, checkpoint, and continuation where
  applicable;
- stable effect-journal facts, including prepared, accepted/sent, uncertain,
  proven committed, or proven not committed;
- canonical lifecycle, authority, usage, and source-lineage evidence; and
- typed references to already-written digest-addressed Session Output.

It atomically commits every included fact or none. Conflict, stale revision,
fence loss, unavailable storage, and rejected transition are closed typed
results; last-write-wins and partial evidence/state success are forbidden.
Program yield, return, cancellation, failure, uncertain effect, and
reconciliation each become truth only through this port. Queueing remains
outside the Program Instance.

The port itself performs no external effect and cannot claim exactly-once
transport. Stable effect ids, prepared-before-send commits, owner-specific
idempotency, and reconciliation prevent blind duplication across crash and
recovery.

### 8.5 Durable event port

Agents owns portable `Event<T>`, generation-scoped `EventRef<T>`,
`EventOccurrence<T>`, and `EventProvenance` meaning; the EventRef lifecycle and
transition reducers; and portable target-application, activation, effect, and
runtime transition semantics. The port registers/waits/cancels an exact typed
EventRef and consumes one authorized fulfillment/expiry/cancellation.

Durable sources, accepted occurrences, delivery attempts and stable target
application, durable activations and effect work, schedules, retry/DLQ/redrive,
recovery, and operational queries enter only through exact admitted event Port
bindings supplied by the Composition Root. Authority, verification,
connections, and secret custody remain outside the semantic kernel and are
presented to APXM only as product-neutral admission facts and short-lived
leases. Adapters own provider/source protocol interpretation and execution.
Runtime does not poll arbitrary URLs or brokers, and redelivery cannot create a
second semantic fulfillment.

### 8.6 Handler execution port

The port invokes an exact compiled handler binding with typed input, scoped
Agent Facade projection, cancellation, limits, and Confinement Port binding
where required. Output and errors match the handler contract. Handler code
cannot perform a model, Capability, or external effect except through the
compiled Agent Program operations and admitted effect boundary. A Python/Node
worker, Wasm component, or subprocess is implementation, not runtime semantics;
remote worker placement uses its own focused Port Contract when present.

### 8.7 Confinement Port

The owner contract includes:

- exact confinement class and supported feature set;
- lifecycle `prepare`, `attach` where supported, `execute`, `cancel`,
  `terminate`, and cleanup outcomes;
- roots/mounts, filesystem policy/enforcement, symlink and race behavior;
- executable/image/artifact allowlist and digest proof;
- environment and short-lived credential-lease injection rules;
- network/egress/DNS policy;
- CPU/memory/time/process/file/output limits;
- stdout/stderr/file/output classification and evidence;
- cancellation/kill deadlines, `TerminationUnconfirmed`, and
  `CleanupUnknown`; and
- cleanup, orphan detection, and reconciliation.

Docker/OCI isolation, namespaces, microVM, Wasm, and local test confinement may
implement this contract. Deployment scheduling and remote worker placement do
not; they use a separate focused worker boundary when required. Runtime never
selects among implementations. If the exact required feature set is
unavailable, admission fails; there is no unconfined or weaker fallback.
Confinement lifecycle uncertainty is not an external-effect `OutcomeUnknown`
unless the confined work independently crossed a declared effect boundary.

### 8.8 Session Output preparation port

Session Output writes are per actual NodeExecution occurrence, classified,
digest-addressed, size-limited, encrypted as required, and return typed opaque
refs. A prepared blob is not Program truth until its reference is included in a
successful Execution Commit. Failed commits leave an auditable reclaimable
orphan; they never create a visible output record. No runtime default directory
or shared filesystem path is semantic.

### 8.9 Execution observer port

Observers receive redacted canonical event projections for metrics/traces/logs
and are explicitly non-authoritative. They cannot mutate context, cancel work,
grant authority, change outcomes, or block terminal truth indefinitely. An
observer/exporter outage follows the declared bounded policy and never becomes
an alternate evidence store.

### 8.10 Deterministic resources

Clock, id/entropy, cancellation, limits, executor, and shutdown are explicit
instance resources. A focused port is used only when an operation must be
substitutable/tested; plain immutable values or owned handles are preferred
otherwise. Runtime does not create a hidden Tokio runtime or read global clock/
random sources for semantic identities.

## 9. Composition Root law

Only application composition may:

- parse environment, files, command-line, or deployment descriptors;
- construct provider/network/database/object/broker/process clients;
- select one exact signed Runtime Profile and supply one exact Deployment
  Composition Manifest;
- invoke `verify_deployment_composition` and construct only the verified
  implementations/descriptors;
- request an authorized short-lived credential lease for one exact bound
  adapter, inject it without retaining or logging it, and discard it when its
  scope ends;
- install telemetry exporters/subscribers;
- own executor/thread/process lifecycle; and
- report startup/readiness/drain/shutdown state.

Secret custody remains outside APXM; Composition Root access to a lease does
not transfer custody into the semantic kernel. The verifier validates all
configuration against exact generated binding schemas and records active
profile, descriptor, binding-payload, configuration, resource-reference, and
platform digests. A configuration default is valid only when it is an explicit
versioned value in the selected signed profile. A literal hidden in an
application is not a profile default. Downstream products may know APXM; APXM
must build, test, release and run without naming or depending on them.

## 10. First-party implementation rule

Every APXM-provided implementation:

1. resides outside semantic core dependencies;
2. implements one primary Port Contract;
3. accepts supporting resources explicitly;
4. publishes a signed descriptor;
5. passes the shared fake/real conformance suite and negative ceiling tests;
6. enters only through the Composition Root and Exact Port Binding; and
7. has no `cfg(feature = ...)` semantic bypass, internal constructor, global
   registration, or direct core-call path.

APXM may bundle a convenient signed Runtime Profile. Bundling is not privilege
and cannot alter semantics.

A deterministic test fake implements the same Port Contract and passes the
owner vectors, but it is explicitly non-admissible in production. It requires
no release signature, SBOM, Implementation Descriptor, Runtime Profile slot, or
Compatibility Set entry. A fake that appears in a production composition is a
verification failure, never a fallback.

Model Deployments and External Agent Profiles compose exact model/peer facts
with an admitted inference or ACP Client implementation. Downstream product
integrations compose outside APXM and never become Implementation Descriptors
or APXM release dependencies.

## 11. Hardcoding disposition

Every candidate literal/branch/default/direct dependency is classified:

| Class | Required action |
| --- | --- |
| semantic invariant | generate/name under owner contract; exhaustively test |
| policy/configuration | typed signed profile/config; pass explicitly |
| replaceable implementation | focused Port Contract + separate implementation |
| deployment fact | supply explicitly at Composition Root; verify and evidence exact binding without search |
| algorithmic constant | named private constant with invariant/tests |
| prototype behavior | delete at full replacement |

Forbidden in semantic core include provider/vendor/model/agent/confinement/
store/transport names, endpoints, credentials, env vars, paths/executables,
process/container commands, database/object/broker implementations, fallback
lists, retry/time policy defaults, string operations/states, generic
JSON/`Any`, and mutable registries.

Contract-generated identifiers and explicit mathematical/format constants are
not accidental hardcoding; their owner and invariant must be evident.

## 12. Conformance

Each owner suite includes:

- non-admissible deterministic fake and every exact real implementation;
- positive and all closed failure outcomes;
- unknown discriminant/field, contract skew, missing feature, bad descriptor,
  bad binding, bad configuration, wrong platform, and signature mutation;
- concurrency, backpressure, cancellation, deadline, crash, restart, drain,
  shutdown, retry/reconcile, and outcome-unknown;
- authority/secret/path/network/process ceiling attacks;
- first-party bypass and direct-core-call negative scans;
- two Runtime Instances with different implementations and no state leakage;
- replacement comparison holding semantic inputs constant; exact response
  equality is required only against a scripted deterministic upstream fixture;
  real model/external-system runs compare schemas, lifecycle, authority,
  effect-id and evidence invariants; and
- dependency/literal/global/discovery/fallback scans.

Canonical output and lifecycle evidence must remain stable across
implementations for scripted deterministic fixtures with the same injected
clock, entropy, upstream responses, and external state. Real implementations
may return different valid domain values; implementation identity, performance,
placement, and attributable evidence are compared separately and cannot change
the meaning of those values.

## 13. Full replacement

The cutover promotes one port/descriptor/binding generation and deletes old
backend registries, provider switches, direct dependencies, ambient discovery,
default adapters, unconfined/degraded paths, string dispatch, generic plugin
escape hatches, process-global state, compatibility traits/constructors, and
old profiles/readers.

Rollback before the point of no return restores the entire prior release and
matching snapshot. It never mixes target core with old adapters or old core
with target bindings. After the point of no return, repair is forward-only via
a new complete Compatibility Set.

## 14. Acceptance checklist

- [ ] finite semantics are generated closed types with exhaustive tests;
- [ ] implementation catalogues use typed descriptor refs, not enums/strings;
- [ ] every port passes the abstraction test and has one owner;
- [ ] artifacts declare authored semantic requirements only;
- [ ] Compatibility Sets contain exact Runtime Profiles but no deployment or
  invocation facts;
- [ ] each Composition Root explicitly selects a profile and the shared
  `verify_deployment_composition` path creates one immutable binding per
  Deployment Port Slot without search;
- [ ] Invocation Admission is separate from deployment composition and every
  effect revalidates current authority/lease facts;
- [ ] runtime construction receives a Verified Deployment Composition and
  performs no discovery;
- [ ] all required families have non-admissible fake and real conformance
  suites;
- [ ] first-party implementations have no bypass;
- [ ] confinement is exact or admission fails; remote worker placement is not
  hidden in the Confinement Port;
- [ ] one Execution Commit atomically commits Program state, effect facts,
  canonical evidence, and output refs; telemetry remains separate;
- [ ] multi-instance/replacement/failure matrices pass;
- [ ] forbidden dependencies/literals/globals/direct calls are absent;
- [ ] Compatibility Sets pin contracts, implementations, Runtime Profiles,
  platforms and conformance; Deployment Composition Manifests pin exact
  deployment bindings/config/resources; Execution Admission carries exact
  identities, opaque correlations, leases, resource ceilings and model
  effects with no downstream product schema; and
- [ ] no legacy, alias, fallback, dual constructor, or mixed binding system
  remains.
