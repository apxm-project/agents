---
status: accepted
date: 2026-07-16
decision: APXM-V1-E2E
owner: APXM agents
amended_by: ADR-0013
---

# Agent Program execution is one end-to-end spine

## Context

An Agent Program is meaningful only if the same semantics survive authoring,
compilation, admission, scheduling, runtime execution, model inference,
Capability execution, durability, and evidence. Treating the compiler,
runtime, or inference backend as independent semantic products would allow
each layer to invent context, retries, tool loops, state transitions, or
fallbacks that the program did not author.

APXM therefore needs one canonical v1 execution contract spanning every
component. Python and TypeScript remain authoring frontends. Rust owns the
compiler and runtime semantics. Model backends implement one admitted model
call; they do not become agent runtimes.

## Decision

APXM v1 has one execution spine:

```text
Python or TypeScript Agent Program source
  -> APXM Authoring Frontend
  -> apxm.frontend-graph.v1
  -> Rust compiler and verifier
  -> apxm.air.v1 and MLIR lowering
  -> apxm.executable-artifact.v1
  -> Server root admission and managed occurrence/delivery/application/activation durability
  -> Agents ActivationRunner, dependency readiness, and local execution
  -> exact admitted model or Capability adapter
  -> inference backend or external resource
  -> Agents fenced Execution Commit, apxm.runtime-evidence.v1, and Session Output
```

Every arrow is a typed, versioned boundary. No layer may reinterpret a
previous layer's semantics. Under the
[current event/runtime ownership](0018-event-readiness-and-local-scheduling-are-agents-semantics.md),
Server owns managed effect-work, schedule, Host-gateway, retry/DLQ, recovery,
and operational APIs, while product consumers use current-owner generated
bindings.

### Authoring and compiler boundary

- Python and TypeScript must express equivalent Agent Program behavior and
  emit semantically equivalent `FrontendGraph` values.
- Frontends never execute programs, print AIR, select a runtime, discover a
  backend, or implement a private lowering path.
- The Rust compiler is the sole verifier, AIR owner, structural-control-flow
  lowerer, MLIR lowerer, artifact builder, and source-map producer.
- Source owns loops, Hook bindings, context updates, model/Capability
  sequencing, program composition, yield, return, and error control flow.
- Compilation is deterministic for the same source bundle, graph, compiler
  options, and Compatibility Set.

### Artifact and admission boundary

The executable artifact pins exact program definitions, entrypoints, types,
static Hook bindings, model and Capability requirements, source maps, handler
digests, and compatibility metadata. It contains no credential, grant,
endpoint, provider secret, mutable context, runtime placement, dynamic
registry operation, or alternate execution policy.

Admission binds the artifact to independently authenticated Agent Identity,
authority, budgets, deadlines, model deployments, Capability bindings,
runtime profile, and placement. Artifact metadata is never identity or
authority proof. Unknown or incompatible values fail before execution.

### Runtime boundary

The runtime executes only the five public semantic AIR operations and
compiler-owned structural IR. It owns generic Program Instance and Invocation
state, structured children, cancellation, durability, effect fencing,
NodeExecution/attempt identity, and monotonic lifecycle evidence.

The runtime must not:

- invent a conversation, Turn, cognition, memory, planning, or tool loop;
- insert model, Capability, Skill, or child output into Program Context;
- select another program, model, provider, Capability, or backend after an
  admitted target fails;
- infer success from a stream, socket, process exit, or delivery receipt; or
- execute a retired operation or pre-canonical artifact through a translator.

### Model adapter and inference-backend boundary

`model.call` is the only AIR operation that reaches model inference. Admission
resolves its abstract `model_ref` to one exact admitted model deployment and
adapter. The adapter sends exactly the runtime-approved Model Context, message
sequence, Tool schemas, structured-output schema, sampling options, budgets,
deadline, cancellation token, and trace identity.

An inference backend may:

- execute one admitted model request;
- stream provisional output chunks;
- return text, structured output, refusals, provider-exposed reasoning
  summaries, usage, finish state, and Tool-call requests; and
- load-balance among replicas that are identical under the same admitted model
  deployment digest and configuration.

It must not:

- execute a requested Tool or Capability;
- call the model again to continue a hidden loop;
- change Program Context or persistent memory;
- create, invoke, or hand off an Agent Program;
- substitute another model, provider, deployment, quantization, prompt,
  Tool schema, or sampling policy when the admitted target is unavailable;
- retry a semantic model call as a new hidden NodeExecution; or
- claim unavailable hidden chain of thought.

A model Tool request is ordinary `ModelOutput` data. The authored Agent Program
decides whether to call `capability.invoke`, how to use the result, and whether
to issue another `model.call`. Each actual call is a distinct NodeExecution.

### Streaming, retry, cancellation, and budgets

- Stream chunks are provisional delivery evidence. Only the runtime's
  authoritative model-node commit records the final output and usage.
- Client or stream disconnection does not cancel the model call unless an
  explicit admitted cancellation wins the runtime state transition.
- Every model call has a stable request/effect id and request digest across
  attempts.
- Automatic retry is allowed only before the adapter may have sent the request
  or when the exact backend contract proves idempotent execution or supports
  reconciliation that prevents duplicate billable or stateful work.
- When backend acceptance is possible and reconciliation is unavailable, the
  node fails as typed `ModelOutcomeUnknown`, records uncertain usage/cost
  evidence, and is not automatically reissued.
- A safe runtime retry remains an attempt under the same NodeExecution and
  preserves the same effect id, admitted model identity and request digest.
- A new model selection, changed request, or authored repetition is a new
  NodeExecution.
- Deadline, cancellation, token, money, and rate ceilings propagate through
  the adapter and backend. The backend cannot widen them.
- If exact usage is unavailable, the result carries typed estimated or unknown
  usage provenance; the system never fabricates exact cost.

### Capability and effect boundary

Every executable action other than model inference goes through
`capability.invoke`. Adapters enforce the admitted grant and effect identity at
the final resource boundary. A model backend cannot bypass this chokepoint by
executing provider-native tools directly.

### Evidence boundary

The runtime emits canonical v1 evidence joining source, graph, AIR, artifact,
Agent Identity, Program Instance, Program Invocation, static Node,
NodeExecution, attempt, model deployment, adapter/backend identity, request
and output digests, usage provenance, cost inputs, cancellation, effects,
placement, and Session Output.

Backend telemetry may enrich this evidence but cannot override execution
truth. Trace spans, token streams, backend logs, and delivery receipts are
never authoritative lifecycle state.

## No fallback or compatibility execution

APXM v1 contains no alternate compiler, AIR printer, artifact reader, runtime,
model substitution, provider substitution, Capability bypass, old operation
handler, compatibility alias, or mixed-version execution path. A missing or
failed exact dependency returns a typed failure.

The pinned pre-canonical implementation may supply read-only fixtures and
failure vectors. It is not a supported execution mode and is deleted at the
v1 cutover.

## Required conformance

One release candidate must prove:

1. equivalent Python and TypeScript source produces equivalent graphs,
   canonical AIR, artifacts, and source maps;
2. in-process composition and Server-managed execution through the same Agents
   ActivationRunner, Execution Commit, and runtime evidence contracts produce
   equivalent Program lifecycle and NodeExecution evidence;
3. every admitted model backend observes the same context, Tool, budget,
   cancellation, safe-retry/reconciliation, outcome-unknown, streaming, and
   output contract;
4. a model Tool request is executed only by authored
   `capability.invoke` control flow;
5. backend unavailability never selects a different model or execution path;
6. runtime replay cannot duplicate a model call, child, event, Capability
   effect, context commit, or output commit; and
7. Studio can reconstruct the complete allowed execution timeline from
   canonical evidence without runtime-specific knowledge.

## Consequences

- Compiler, runtime, and inference work can proceed in separate worktrees only
  after their shared v1 vectors are frozen.
- The vLLM integration is a model adapter/backend implementation, not an Agent
  Program runtime or loop owner.
- Backend performance optimizations are valid only when semantic and evidence
  conformance remains unchanged.
- Release promotion requires end-to-end evidence for the exact compiler,
  runtime, adapters, and backend digests in one Compatibility Set.
- Delivery is coordinated by the APXM master plan and the Agent Program
  composition/AIR full-replacement plan.
