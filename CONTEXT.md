# APXM Agent Programs

This glossary defines the ubiquitous language for authoring, compiling, and
executing APXM Agent Programs. Detailed behavior belongs in contracts and
architecture documents; this file defines what the terms mean.

## Programs and execution

**Program Execution Model (PXM)**:
The semantic contract connecting Agent Program source, FrontendGraph, AIR,
artifact admission, generic execution, external effects, and evidence so every
layer agrees what a program means.
_Avoid_: runtime implementation, graph format, scheduler

**Agent Program**:
An APXM frontend program that defines the behavior of an agent or workflow and
can be compiled into an executable APXM artifact.
_Avoid_: program package, agent package, runtime agent

**Program Instance**:
The stateful logical realization of an exact Agent Program created by
`program.new` inside a program or by the APXM root lifecycle API, then invoked
through `instance.invoke`. It is single-flight and fail-busy: program yield
leaves it ready, while program return completes it.
_Avoid_: Runtime Instance, process, deployment, Agent version

**Program Invocation**:
One admitted execution or resumption of an Agent Program, either one-shot
through `program.invoke` or stateful through `instance.invoke`.
_Avoid_: Turn, HTTP request, process

**Program Yield**:
A typed invocation boundary that commits plain output, explicit next Program
Context, and compiler-owned continuation. The next stateful invocation input
resumes that continuation.
_Avoid_: Turn, region yield, await event, final return

**Program Return**:
The final typed boundary that commits plain output and completes the Program
Instance.
_Avoid_: program yield, response delivery, metadata envelope

**Structured Task Scope**:
The frontend concurrency construct that starts attached child work and joins
every child before exit. It has equivalent Python/TypeScript semantics and no
detach form.
_Avoid_: raw coroutine scheduling, dropped Promise, fire-and-forget

**Event Reference**:
An unforgeable typed handle consumed by `await.event` to park the current
Program Invocation until one authorized durable fulfillment, expiry, or
cancellation.
_Avoid_: polling URL, resume opcode, program yield

**Agent Identity Binding**:
The authenticated admission result binding one Agent Identity to an exact
Agent Program artifact and scope. Artifact metadata and parent identity are
not identity proof.
_Avoid_: agent name string, ProgramRef, Capability Grant

**Runtime Instance**:
One embedded APXM runtime kernel constructed by an application with explicit
adapters and resources.
_Avoid_: Program Instance, agent process, session

**Conversational Agent**:
An Agent Program whose frontend authoring construct defines a conversational
loop and controls context during every Turn. It is a frontend concept, not a
distinct runtime type.
_Avoid_: conversational runtime, chat service, host loop

**Gao**:
The APXM workflow-authoring Agent Program implemented as a named TypeScript
specialization of the standard Conversational Agent construct. Gao adds
workflow-authoring Skills, Capabilities, prompts, and Hooks; it does not add a
runtime, loop engine, compiler path, or privileged Studio callback system.
_Avoid_: Gao runtime, Studio orchestrator, built-in chat engine

**Turn**:
The Studio projection of one dynamic occurrence of a Conversational Agent's
annotated program-authored loop region, from input consumption to completion
or yield. It is not an AIR or runtime entity.
_Avoid_: model call, message, request, runtime type

**Node**:
One typed operation in the compiled execution graph of an Agent Program.
_Avoid_: hook, tool, arbitrary task

**NodeExecution**:
One actual visit to a static Node during a Program Invocation. Runtime retries
are attempts under the same NodeExecution; authored repetition creates another.
_Avoid_: static Node, retry attempt, Turn

## Context and memory

**Program Context**:
The typed local/state value explicitly accumulated by Agent Program source as
`agent.context`; a successful stateful invocation commits its next value.
Program Context carries information, not authority.
_Avoid_: global context, implicit prompt, Execution Context

**Model Context**:
The policy- and budget-constrained projection of Program Context admitted to
one model invocation.
_Avoid_: Program Context, prompt, model memory

**Capability Context**:
The least-privilege projection of invocation metadata and Program Context made
available to one Capability invocation.
_Avoid_: Execution Context, ambient capability state

**Execution Context**:
Runtime-private state used to schedule, observe, budget, and enforce one
execution. It is not program data and does not itself grant business authority.
_Avoid_: Program Context, Capability Grant, hook argument

**Memory**:
State stored outside a Program Instance through an explicit admitted
Capability and available beyond the local Program Context that produced it.
_Avoid_: Program Context, transcript string, implicit persistence

## Hooks

**Hook**:
An author-defined callback statically bound before or after an agent, loop,
Node, model, or Capability scope. It receives the Agent Facade; failure
recovery uses ordinary authored try/catch rather than an error Hook.
_Avoid_: middleware, runtime observer, arbitrary event handler

**Hook Event**:
A typed before/after callback point around an agent, loop, Node, model call, or
Capability invocation.
_Avoid_: untyped event name, log event

**Agent Facade**:
The portable whole program-visible agent passed to a Hook, including typed
context, current input/output/target, admitted handles, and lifecycle views,
but excluding the live frontend object and runtime-private Execution Context.
_Avoid_: Hook Context, Execution Context, service locator

**Execution Observer**:
A runtime-owned callback for scheduler telemetry and operational observation.
It is separate from a program-authored Hook and cannot change Agent Program
semantics.
_Avoid_: Hook, Agent Hook

## Skills and capabilities

**Skill**:
Context that teaches a model how and when to pursue a task without granting
authority to perform an action. Skill bodies are discovered through admitted
Capabilities and are never injected automatically.
_Avoid_: Capability, Tool, permission

**Capability**:
An admitted unit of executable action whose invocation is constrained by an
explicit authority contract.
_Avoid_: Skill Capability, Skill, ambient tool

**Tool**:
A model-callable operation exposed through an admitted Capability.
_Avoid_: Capability Grant, Skill, permission

## External agents and routing

**Model Target Reference**:
An immutable content-addressed v1 source/artifact requirement for one exact
model/checkpoint/configuration and deployment eligibility contract. It contains
no endpoint, credential, mutable alias, or environment placement. The selected
Runtime Profile and Deployment Composition Manifest predeclare exactly one
target-to-deployment-to-inference-binding mapping; the shared deployment
verifier validates and materializes it without candidate search. Invocation
Admission references that exact verified mapping for one model effect.
_Avoid_: model alias, runtime default, route policy

**Resolved Model Binding**:
The immutable invocation-admission proof joining one Model Target Reference to
one exact Model Deployment and its already verified inference Port Binding for
one model effect. Runtime rejects any digest mismatch and invokes it but never
selects it.
_Avoid_: fallback, runtime route, provider registry entry

**External Agent**:
A non-APXM protocol peer invoked through an admitted Capability. It is not an
Agent Program, Program Instance, model backend, or runtime identity.
_Avoid_: APXM specialist, ProgramRef, runtime agent

**ACP Client Adapter**:
The focused adapter through which APXM acts as an Agent Client Protocol client
for an exact External Agent Profile. ACP remains a protocol boundary rather
than an AIR operation or composition primitive. APXM core depends on this Port
Contract and exact profile, not directly on a Claude, Codex, or other vendor
agent SDK.
_Avoid_: ACP runtime, `program.invoke` transport, subprocess opcode

**External Agent Profile**:
A signed immutable composition of one exact external peer artifact or endpoint,
one ACP Client Adapter Implementation Descriptor, capability matrix, execution
ceiling, authentication-reference contract, limits, and conformance evidence.
The profile is not itself a Port implementation.
_Avoid_: mutable command, CLI nickname, arbitrary shell string

**External Agent Session**:
One adapter-owned ACP session correlated to the invoking Capability call and
Program Invocation. Its peer/process state is external state, not Program
Context or a Program Instance; source may retain only a typed opaque
`ExternalAgentSessionRef` in explicit Context.
_Avoid_: Turn, Program Instance, Agent version

**External Agent Evidence**:
Closed ordered evidence nested beneath the outer Capability NodeExecution:
session transition, wire message, peer content/plan/Tool state/session snapshot/
measurement, reverse-Capability link, process-transport fact, or reconciliation
fact. It never creates APXM Turns, child model nodes, or spend facts.
_Avoid_: flattened peer log, native model evidence, trusted invoice

**Peer Measurement**:
An External Agent evidence fact for context-window, token-usage, or monetary-
cost data with explicit scope, accumulation, precision, origin, and
availability. ACP `usage_update.used/size` retains its admitted peer meaning;
it is never automatically input/output tokens or native model accounting.
_Avoid_: zero-filled missing usage, `TokenAccountant` input, APXM billed cost

**External MCP Client Invocation**:
A local or remote external client invokes an admitted APXM MCP surface over
local stdio or managed Streamable HTTP. APXM observes its authenticated MCP
request and downstream APXM effects, not the upstream client's model, tokens,
subscription, or cost; those remain `unavailable_not_observed`.
_Avoid_: managed ACP session, provider token passthrough, upstream usage fact

**Model Route Policy**:
A planned future APXM-owned signed policy for pre-dispatch selection among
exact eligible model targets. It is not executable v1 semantics and contains
no resend-after-send or hidden model/Tool loop.
_Avoid_: fallback chain, model alias, runtime default

**Model Route Decision**:
The planned future immutable fact explaining which exact model target was
selected before dispatch and why other candidates were rejected.
_Avoid_: failover result, model response, mutable route

**External Agent Route Decision**:
The planned future immutable result of selecting one exact eligible External
Agent Profile before session creation. When admitted, selection and invocation
are separate Capability calls and both are evidenced; v1 selects the profile
exactly in source.
_Avoid_: `SPAWN_AGENT`, string dispatch, Program selection

## Distribution and compatibility

**Agent Program Source Bundle**:
The source, manifest, handlers, resources, and requirements used to build an
Agent Program artifact. The manifest identifies those inputs; frontend source
is the sole source of Hook, loop, graph, and behavior semantics.
_Avoid_: program package, behavior manifest, runtime agent

**Executable Agent Artifact**:
The immutable, signed, digest-addressed compiler output admitted by Server and
the runtime. It binds source, FrontendGraph, AIR, handlers, contracts,
requirements, identities, limits, and provenance without creating a user-
visible Agent version.
_Avoid_: ProgramPackage, editable Agent, floating artifact

**APXM Release Family**:
The complete set of separately installable APXM libraries, authoring
frontends, generated clients, and applications promoted through one release
decision.
_Avoid_: monolithic APXM package, unrelated package releases

**Installable Surface**:
One focused APXM library, language package, generated client, binary, or image
with a single responsibility and an explicit dependency/network ceiling.
_Avoid_: bundle everything, hidden service dependency

**Rust Embedding Library**:
One of the five focused APXM Rust roles for stable contracts/types, AIS
specification/authoring, artifact encoding/admission, compiler embedding, or
runtime embedding. It exposes one stable in-process responsibility and receives
external resources explicitly.
_Avoid_: umbrella `apxm` facade, application composition crate, hidden service

**Adapter Crate**:
A focused Rust library implementing one injected runtime interface for a model
provider, Capability family, atomic Execution Commit, artifact/output blob
access, event transport, handler, narrow Confinement, or telemetry observer.
Its authority, secrets, data, network, process, platform, and failure ceiling
are explicit and independently admitted. It cannot split authoritative
state/continuation/effect/evidence/output-reference commit.
_Avoid_: runtime default, feature-selected semantic mode, ambient backend

**Port Contract**:
A versioned single-purpose compiler/runtime boundary defining the typed
request, result, lifecycle, authority, evidence, and failure semantics every
interchangeable implementation must preserve.
_Avoid_: generic plugin hook, service locator, implementation API

**Implementation Descriptor**:
A signed immutable record binding one exact implementation artifact to one Port
Contract, supported closed features, configuration contract, platforms,
ceilings, and conformance evidence.
_Avoid_: provider string, feature flag, mutable registry entry

**Exact Port Binding**:
The immutable deployment association between one typed deployment slot, one
exact Implementation Descriptor, and that Port Contract's generated binding
payload. It contains only exact configuration and stable opaque resource
references plus binding-admission evidence; it contains no invocation identity,
Capability Grant, credential lease, budget reservation, or execution evidence.
_Avoid_: runtime lookup, fallback candidate, untyped backend map

**Composition Root**:
The outer APXM application or embedding host that explicitly selects one
Runtime Profile, supplies its deployment configuration and resources, invokes
the shared library-owned deployment-composition verifier, and constructs a
compiler or Runtime Instance from the verified result. It never searches for,
ranks, or falls back to an implementation.
_Avoid_: runtime core, global registry, environment discovery

**Closed Semantic Type**:
A finite contract-owned enum or tagged union whose variants exhaust one APXM
semantic vocabulary for an exact contract digest. Owner-defined epistemic
variants such as `OutcomeUnknown`, `UsageUnknown`, or
`CancellationUnconfirmed` are closed meaning; only generic catch-alls such as
`Unknown(raw)` or `Other(String)` are forbidden.
_Avoid_: string discriminator, `Other`, implementation catalogue

**Runtime Profile**:
Immutable release data in a Compatibility Set that predeclares one exact
Implementation Descriptor for every deployment slot plus binding-payload
contracts, platform constraints, and conformance evidence. A Composition Root
selects the profile explicitly. The profile contains no deployment resource
reference, credential, invocation authority, search rule, or fallback.
_Avoid_: default backend, mutually exclusive feature, hidden composition

**Deployment Composition Manifest**:
The signed deployment-specific materialization of one selected Runtime Profile,
containing the exact generated Port Binding payloads, stable opaque resource
references, configuration digests, and binding-admission evidence for one
installation. Managed and standalone Composition Roots validate it through the
same library-owned `verify_deployment_composition` path.
_Avoid_: release Compatibility Set, invocation grant, runtime registry

**Verified Deployment Composition**:
The typed immutable result of `verify_deployment_composition`, containing the
exact constructed Port Binding Set and profile/deployment proof accepted by one
Runtime Instance. It authorizes no Agent Program effect by itself.
_Avoid_: Runtime Profile, Capability Grant, mutable service locator

**Invocation Admission**:
The immutable per-invocation facts supplied separately from deployment
composition: authenticated Company, Acting Principal and Agent Identity,
Capability Grants, approval references, credential leases, budget reservation,
resolved model binding, deadlines, limits, cancellation, and lineage. Runtime
validates these facts at their owner boundaries; neither a Runtime Profile nor
an Exact Port Binding grants authority.
_Avoid_: deployment configuration, release manifest, ambient execution context

**Execution Commit Port**:
The single authoritative persistence boundary that atomically commits one
Program lifecycle/state revision, checkpoint or continuation, effect-journal
facts, canonical evidence, and references to already-written digest-addressed
Session Output. It prevents state, effects, and evidence from becoming
different truths after a crash.
_Avoid_: telemetry exporter, independent evidence write, best-effort log

**Confinement Port**:
The focused runtime boundary for preparing, attaching to, executing within,
cancelling, terminating, attesting, and cleaning up one exact isolation
environment under declared roots, mounts, executable, egress, and resource
ceilings. Remote worker placement or scheduling is a separate boundary when it
exists.
_Avoid_: Kubernetes scheduler, generic process service, Capability executor

**Compatibility Set**:
The immutable manifest identifying the exact artifact, contract, feature,
platform, source, production Implementation Descriptor, Runtime Profile
template, digest, and conformance combination supported as one APXM release
family. It contains no
deployment resource binding, customer secret reference, credential lease, or
invocation authority.
_Avoid_: latest versions, version range guess, mixed release

**Authoring Frontend**:
The Python or TypeScript library that provides language-native Agent Program
constructs and records the compiler-owned `FrontendGraph`.
_Avoid_: Python runtime, TypeScript runtime, AIR printer

**FrontendGraph**:
The canonical versioned, language-neutral value recorded by every APXM
Authoring Frontend and consumed by the Rust compiler. The target replacement is
`apxm.frontend-graph.v1`. The implementations that predate this contract are
prototype evidence, not another supported version.
_Avoid_: Python graph, TypeScript IR, handwritten AIR

**AIR**:
The compiler-owned portable execution representation. Target AIR exposes five
public semantic operations: model call, Capability invocation, Program
creation, Program invocation, and durable event wait.
_Avoid_: frontend API, raw operation builder, runtime configuration

**Compiler Bridge**:
An explicit local native binding or generated remote client that submits a
`FrontendGraph` to the owning Rust compiler contract. It owns transport/ABI
mapping, not compiler semantics or runtime execution.
_Avoid_: compiler fallback, CLI shell-out, frontend runtime

**Remote Client**:
A generated, separately installable client for an owning APXM service contract.
It transports typed requests, events, and faults but contains no compiler,
runtime, or copied service policy.
_Avoid_: handwritten wire model, frontend SDK, hidden local server
