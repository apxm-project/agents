# ACP interoperability and selection contract

- Status: normative APXM v1 contract; future routing sections are planned, not executable v1 semantics
- Owner: APXM `agents` (product-neutral semantic types and ports)
- Binding ADR: [ADR-0012](../adr/0012-acp-uses-explicit-capabilities-selection-is-not-runtime-semantics.md)
- Workspace authority: [ADR-0028](../../../../docs/adr/0028-apxm-is-the-product-neutral-agent-program-and-inference-core.md),
  [ADR-0029](../../../../docs/adr/0029-agent-program-source-and-closed-semantics-are-behavior-truth.md),
  [ADR-0030](../../../../docs/adr/0030-execution-inference-evidence-and-deployment-are-exact-and-product-neutral.md)
  (paths and titles as published in apxm#148; `Invocation Admission` is the
  agents synonym for product-neutral Execution Admission)

## 1. Purpose

This contract makes APXM work with Claude, Codex, and other admitted coding
agents through ACP without creating a second APXM runtime. It also prevents
four unrelated selection concerns from collapsing into one ambiguous router.

| Concern | V1 contract | Future contract | Final decision owner |
| --- | --- | --- | --- |
| APXM Program selection | source chooses a statically imported `ProgramRef` | unchanged | source/compiler |
| model selection | source names one exact `ModelTargetRef`; the verified Deployment Composition predeclares one exact deployment/adapter mapping and Invocation Admission materializes it | future plan may propose an immutable route policy | product-neutral Invocation Admission before dispatch |
| External Agent selection | source names one exact `ExternalAgentProfileRef`; session Invocation Admission uses that profile and the verified ACP Client Port Binding | source may invoke `external_agent.route` | product-neutral Invocation Admission through Capability |
| event routing | exact Port bindings durably accept, deliver, and apply occurrences under Agents-owned Event/EventRef semantics | unchanged | exact event Port bindings over Agents semantics |

No runtime router, backend registry, adapter, product UI, or provider alias may
choose among these on behalf of source or Invocation Admission.

## 2. Defensible interoperability promise

> **Canonical target versus prototype.** Current `SPAWN_AGENT`/
> `COMMUNICATE`, generic process registry, AAM turn-zero injection, ambient
> credentials, and native token-accounting paths are pre-canonical evidence.
> They do not implement this contract and are deleted in the full replacement.

APXM v1 acts as an ACP Client and can invoke an exact, admitted ACP Agent
profile through typed External Agent Capabilities. The ACP Client adapter code
is selected by the Runtime Profile and verified Deployment Composition; the
External Agent Profile separately pins the exact peer and references that
adapter's Implementation Descriptor. Support is asserted only for the exact
implementation/profile/ACP feature matrix admitted by the Compatibility Set
and Deployment Composition Manifest.

The first conformance targets are:

- Claude through the admitted `claude-agent-acp` adapter over the Claude Agent
  SDK; this is not a claim that Claude Code natively implements ACP;
- Codex through the admitted ACP `codex-acp` adapter over the Codex App Server;
  this is not a claim that the Codex CLI natively exposes ACP; and
- another ACP agent only after its exact artifact, platform, negotiated
  features, confinement and protocol behavior pass the same admission suite.

An adapter may use the vendor SDK/App Server behind its boundary, but APXM core
depends only on the exact ACP Client Port, generated protocol types, and
admitted Implementation Descriptor/Profile. Vendor SDKs never become core
semantic, runtime, admission, or evidence dependencies.

APXM v1 does not expose Agent Programs as ACP Agents. ACP is not a product
integration or webhook transport. The canonical v1 transport profile is
JSON-RPC over an admitted stdio subprocess. A future network transport
requires a separate accepted protocol contract and cannot be inferred from ACP
compatibility.

## 3. Program surface

Frontend libraries expose typed wrappers whose calls all lower to
`capability.invoke`:

```text
external_agent.session.open(profile, workspace, projected_context)
  -> ExternalAgentSessionRef
external_agent.prompt(session, prompt)
  -> O
external_agent.session.cancel(session, reason)
  -> None
external_agent.session.close(session)
  -> None
```

The source-visible result is deliberately plain. Prompt returns only the
declared semantic output `O` after the outer Capability NodeExecution commits.
Cancel and close return `None` after confirmed idempotent success, including an
already-terminal session. Denial occurs during Invocation Admission before
adapter dispatch. Cancelled, failed, and `ExternalAgentOutcomeUnknown` are
closed typed Capability failures used by authored try/catch; they are not
fields in a result envelope. Session ids, ACP metadata, adapter/model identity,
protocol messages, attempts, reconciliation, usage, cost, timings, and process
facts are evidence/query data and never result metadata.

`ExternalAgentSessionRef` is opaque, scoped and non-forgeable. Its durable
session record is bound to one owning `ProgramInstanceRef` and its root
instance lineage or, for one-shot use, one `ProgramInvocationRef`; Acting
Principal; invoking APXM Agent Identity; exact profile; and grant lease. A
stateful Program may retain it only in that same Program Instance's typed
Program Context for later invocations. It cannot escape in output, be forged,
move to an unrelated instance/invocation/principal, or become a
`ProgramInstanceRef`. It contains no credential, authority, process snapshot or
mutable profile.

Opening, prompting, cancelling, and closing are distinct effects with distinct
stable effect ids. The compiler rejects implicit global sessions, arbitrary
commands, raw ACP JSON, process ids, direct stdio handles, or profile strings.

## 4. Exact profile contract

`apxm.external-agent-profile.v1` is immutable and signed. It pins:

- profile id and digest;
- ACP protocol/schema version and official SDK/schema digest;
- exact ACP Client Adapter Implementation Descriptor digest;
- exact external peer artifact/endpoint, executable and dependency-lock
  digests;
- platform and architecture;
- launch descriptor without a shell interpolation surface;
- required and optional negotiated ACP features;
- authentication-method references, never plaintext credentials;
- allowed workspace roots, writable roots and path mapping;
- executable, environment and network-egress ceilings;
- MCP gateway references and their attenuated scopes;
- process, memory, output, duration and concurrency limits;
- cancellation/close deadlines and termination policy;
- redaction, retention and evidence classification;
- SBOM, provenance, vulnerability policy and conformance evidence.

Production rejects runtime package downloads, SemVer ranges, mutable tags,
ambient `PATH` resolution, arbitrary commands, plaintext environment secrets,
missing confinement, unbounded roots, or unknown extensions. A development
External Agent Profile is explicitly non-production and cannot be referenced by
production Invocation Admission or promoted implicitly.

Generic and user-defined ACP-speaking CLI profiles remain explicit migration,
development, and configuration inputs. Production support exists only after an
exact immutable profile is admitted by the Compatibility Set; generic input is
never a production compatibility promise.

## 5. ACP lifecycle

### 5.1 State machine

```text
declared
  -> launching
  -> initializing
  -> authenticating?       (only when negotiated and configured)
  -> creating_or_loading
  -> ready
  -> prompt_in_flight
  -> ready | cancelling | closing
  -> closed | lost | outcome_unknown
```

Every transition is idempotent for its logical effect id and records the
profile, process/transport correlation, ACP session id and negotiated features.
`closed`, `lost`, and `outcome_unknown` are terminal session states. Cleanup or
close after `lost`/`outcome_unknown` records a separate cleanup result and never
rewrites the terminal fact to `closed`. Unknown states fail closed.

Durable session record, ownership binding, lease, control state, opaque handle
and evidence correlation enter through exact admitted Port bindings supplied by
the Composition Root. The runtime/adapter owns only the currently admitted live
transport/process. V1 permits one connection or process per
`ExternalAgentSessionRef` and never pools it across principals, Program
Instances, profiles, or grant leases.

### 5.2 Initialization and negotiation

The adapter initializes first, validates the returned protocol version, records
both implementation identities, and intersects the exact profile requirements
with peer-advertised features. An omitted feature is unavailable. An unknown
extension is ignored only when the pinned schema declares it safely optional;
otherwise admission fails.

Authentication uses only the pinned ACP method and an Invocation-Admission
credential lease for that exact profile and adapter binding. The Composition
Root or effect boundary injects the short-lived material without retaining it;
the adapter cannot invent non-standard wire fields, read host-local
credentials, or become secret custodian. Session new/load and optional
`session/delete` behavior are used only when the exact ACP version and
negotiated profile support them. APXM `external_agent.session.close` never
assumes ACP has a baseline close method: it may invoke admitted
`session/delete`, then releases/terminates the transport according to the exact
profile. A session is never silently recreated after loss.

### 5.3 Prompt turns

An ACP prompt turn is not an APXM Conversational Agent Turn. It may contain
multiple private model/tool cycles. APXM treats the whole prompt as one outer
Capability NodeExecution and preserves ordered nested protocol events.

Source supplies an explicit typed prompt and explicit context projection. The
adapter never injects AAM state, the parent transcript, Skill bodies, Program
Context, grants, environment, filesystem or a hidden “turn zero.”

### 5.4 Cancellation and close

The following are different facts:

1. cancellation of the outer APXM Capability occurrence;
2. ACP `session/cancel` for the active prompt;
3. JSON-RPC request cancellation where applicable;
4. optional negotiated ACP `session/delete`, when admitted;
5. transport/process termination; and
6. proof that externally visible effects did or did not occur.

A `session.cancel` operation cancels only the active prompt; it does not close
or revoke the external-agent session. `session.close` is the only operation
that terminates that APXM session and invalidates its scoped handles. A
cancellation request never reports completed success before the prompt has a
terminal outcome. If termination cannot prove the result, the Capability ends
`ExternalAgentOutcomeUnknown`, fences all new reverse requests, and preserves
the uncertain session/effect lineage.

## 6. Authority and reverse requests

ACP feature negotiation uses the protocol word “capability”; it is not an APXM
Capability, Skill, Entitlement, or Grant. ACP permission choices communicate an
interactive answer; they cannot grant or widen APXM authority.

ACP client-directed filesystem, terminal, and permission requests are admitted
as nested APXM Capability occurrences under:

- root Program Invocation and outer effect id;
- Acting Principal and invoking APXM Agent Program's Agent Identity;
- adapter Workload Identity and negotiated ACP `agentInfo`;
- exact profile, session and peer request ids;
- complete Capability Definition and effective Grant;
- operation, resource, workspace root and path mapping;
- confinement, executable and egress ceiling;
- approval, budget and time constraints; and
- stable nested effect id.

The adapter presents only the choices permitted by the admission decision. A
local `ApproveAll` mode, editor trust assumption or agent request is never
authority. Missing admission, confinement or path evidence denies the request.

An attenuated Capability gateway is not an ACP reverse method. A configured
gateway entry may point only to an admitted Capability gateway; each gateway
operation is independently admitted and evidenced. Skill Discovery is exposed
on demand through that admitted Capability gateway and never injects Skill
bodies. The exact profile references one admitted gateway descriptor (for
example local stdio or Streamable HTTP). The gateway uses its own
audience-bound admission and never passes an upstream provider token through to
APXM or a downstream host. Agent-native tools that the peer does not delegate
to the ACP client remain inside the outer external-agent Capability. They are
still constrained by the profile's mandatory roots, executable, egress, process
and confinement ceilings and appear only as attributed external evidence, not
APXM child nodes.

## 7. Evidence and usage

The outer NodeExecution preserves:

- exact profile/adapter/schema/Compatibility Set digests;
- launch, initialization, authentication and negotiation facts;
- session and request correlation ids;
- hash/classification of projected input and ACP messages;
- ordered updates, plans, messages, diffs, tool-call states and terminal facts;
- `agent_thought_chunk` only when the peer actually emits it, classified,
  redacted and purpose-gated like other content; APXM never requests or
  fabricates hidden chain-of-thought;
- every reverse request, decision, approval and nested effect;
- cancellation, close, process exit and reconciliation;
- raw-payload object references and redaction decisions; and
- exact terminal result, including lost and outcome-unknown.

ACP-reported context usage, token estimates, cost, model identity and tool data
are attributed third-party evidence. They never overwrite canonical APXM usage
evidence, an APXM model NodeExecution, or an admitted operational-usage fact.
Evidence projections must label the provenance explicitly.

`apxm.external-agent-evidence.v1` is a closed union of
`session_transition`, `wire_message`, `peer_content`, `peer_plan`,
`peer_tool_state`, `peer_session_snapshot`, `peer_measurement`,
`reverse_capability_link`, `process_transport_fact`, and
`reconciliation_fact`. A `peer_measurement` distinguishes `context_window`,
`token_usage`, and `monetary_cost` and records scope, accumulation, precision,
origin, and availability. Unknown variants fail against the exact contract
digest.

ACP `usage_update.used/size` is preserved according to the exact adapter's
declared semantics; it is never relabelled input/output tokens merely because
the fields are numeric. Context-window capacity is not consumption. An ACP
prompt does not enter the native model `TokenAccountant`, acquire a fabricated
`GenerationIdentity`, or become a model NodeExecution.

Before opening or prompting, Invocation Admission includes the resource-ceiling
reservation for the outer Capability using the profile's configured maximum
duration/resource charge or another trusted bounded price rule. A deployment
may explicitly allow unknown-cost work only under a separate policy ceiling; a
hard ceiling fails closed when no trusted maximum can be established.
Cumulative peer usage is stored as provenance-labelled snapshots. A monotonic
delta is derived only when the exact profile declares reset/scope semantics and
the ordered sequence is continuous. Duplicate, reset, gap, cancellation, and
failure do not produce a guessed delta. Peer evidence never affects operational
usage unless a separate admitted contract produces an
`apxm.operational-usage-fact.v1`.

Metered provider/API-key execution may have a versioned derived estimate and
optional provider-reconciled value. Seat/subscription execution may
legitimately expose no per-run billed cost. An inbound external Capability
client does not reveal its upstream model usage or licence cost. All three
cases are first-class provenance/availability states, never zero-filled facts.

## 8. Exact model selection in v1

`model.call` carries one exact content-addressed `ModelTargetRef`. The artifact
records its portable model/checkpoint/configuration and deployment-eligibility
requirement, never an endpoint, credential, or environment deployment id.
The explicitly selected Runtime Profile and verified Deployment Composition
predeclare one exact inference implementation and one target-to-deployment
mapping. Invocation Admission materializes that mapping as one immutable
`ResolvedModelBinding` containing exact deployment, referenced inference Port
Binding, checkpoint/model, configuration, locality, price reference, credential
reference and effect id. There is no catalogue query or resolver search.
Runtime validates that binding and dispatches once.

There is no default, alias, role-based selection, first-healthy scan, provider
substitution, circuit-breaker fallthrough or post-send failover. Before-send
unavailability is a typed failure. After dispatch may have occurred, only the
same target's proven idempotency/reconciliation contract may establish a retry;
otherwise the outcome is `ModelOutcomeUnknown`.

## 9. Future APXM-owned routing contract

Routing implementation is future work. Lemonade and RouteLLM are research
examples only; APXM will not adopt either as its semantic router.

A future model route policy must contain a finite exact candidate-set digest,
hard feature/authority/locality/risk/budget constraints, objective, price
snapshot rules, deterministic tie-breaker, evaluation claim, scorer digest when
learned, and typed no-eligible behavior. Final pre-dispatch resolution, when
separately accepted by ADR, produces an immutable decision before runtime
receives the binding.

A future External Agent route policy follows the same selection-before-effect
rule over exact admitted profiles. Its Capability returns a decision only;
source must still explicitly open and prompt the selected profile.

Future routing cannot:

- choose an APXM Program;
- change authority or candidate eligibility;
- dispatch, retry, or fail over as part of scoring;
- depend on an unevidenced mutable catalogue;
- conceal rejected candidates or budget/price inputs; or
- become executable before evaluation and drift gates pass.

## 10. Owner map

| Owner | Responsibility |
| --- | --- |
| `agents` | frontend types, exact refs, portable Event/EventRef/occurrence/provenance and target-application/activation/effect semantics, ACP Capability/session ports, product-neutral Invocation Admission and exact Port binding verification, `ModelTargetRef`/`ResolvedModelBinding` validation, checkpoint/confinement/evidence contracts, and owner-local conformance |
| compiler/runtime | compiler records exact semantic requirements; runtime validates Verified Deployment Composition plus Invocation Admission and executes one immutable admitted binding |
| Composition Root | selects one Runtime Profile and Deployment Composition Manifest, supplies opaque external correlations and exact admitted bindings, injects short-lived credential material without retaining it, and owns process/placement lifecycle outside the semantic kernel |
| Exact Port bindings / adapters | provider/source protocol interpretation and execution, official-schema ACP Client and exact inference implementations, optional future pure scorer; durable session/event/store seams when admitted; no Program-target selection |
| Downstream products | may authorize work, evaluate quality and project evidence outside APXM; they never define APXM execution semantics or become APXM release dependencies |

## 11. Conformance gates

V1 release evidence must prove:

- Python/TypeScript wrappers emit only `capability.invoke` and equivalent
  graphs;
- five-op AIR contains no ACP/process operation;
- exact Claude, Codex and other admitted ACP profile negotiation;
- required-feature omission and version skew fail closed;
- no arbitrary command, runtime download, mutable tag or unconfined profile;
- path escape, terminal and permission requests are attenuated; Capability
  gateway use is only through a separately admitted gateway; agent-native tools
  remain confined;
- prompt/cancel/close/crash/race/session-loss/outcome-unknown behavior;
- session ownership/non-transfer, no cross-principal pooling, terminal-state
  immutability and result-versus-evidence schema separation;
- closed nested ACP evidence, third-party usage provenance, and rejection of
  peer measurements by the native model accountant;
- local stdio and Streamable HTTP Capability gateway attenuation, including
  audience validation and token-passthrough rejection;
- hard-ceiling bounded/unknown-cost denial plus cumulative-delta/reconciliation
  vectors;
- exact model binding with no registry fallback or Composition Root bypass of
  admission;
- embedded and reference Runtime Instances use the same
  `verify_deployment_composition` path for the exact ACP Client adapter binding;
  neither path searches or resolves an implementation; and
- absence of prototype `SPAWN_AGENT`, `COMMUNICATE`, `AgentRouter`, AAM preamble,
  ambient model registry, false `used/size` token mapping, fabricated model
  identity and fallback paths.

Future routing has its own admission gates and is not considered shipped by
passing the v1 exact-selection suite.

Canonical v1 contains no route-policy, candidate-set, scorer, route-decision,
or resolver-port field. A new accepted ADR and Compatibility Set are required
before any future routing schema becomes executable.
