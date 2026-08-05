# agents owner contract surface

This directory publishes the `agents` signed owner descriptor against the
already-published contract constitution layer. The constitution and its common
envelopes live in the sibling `contracts` workspace and are referenced here by
`$id` and file digest only; they are never copied or redefined.

## Layout

```
schemas/         closed Agent Program semantic schemas owned by agents
port-contracts/  the Capability invocation and atomic execution-commit Port Contract descriptors
vectors/         positive and negative conformance vectors
descriptors/     the content-addressed agents owner descriptor
tools/           the validation and digest gate
```

## Owned schemas

| `$id` | Responsibility |
| --- | --- |
| `apxm.frontend-graph.v1` | Language-neutral graph recorded equivalently by Python, TypeScript, and generated source |
| `apxm.air.v1` | Five public semantic operations plus compiler-owned structural IR |
| `apxm.executable-artifact.v1` | Digest-bound artifact emitting only `artifact_semantic` Port Requirements |
| `apxm.runtime-evidence.v1` | Append-only monotonic Program Instance/Invocation/effect facts |
| `apxm.source-map.v1` | Non-executable mapping from operations and regions back to source spans |
| `apxm.handler-manifest.v1` | TypeScript-only artifact-local tool/hook handler sidecar; no Python package-local handler shape |
| `apxm.capability-invocation.v1` | Exact non-model Capability request with canonical arguments and separate admitted identity, authority, correlation, and effect facts |
| `apxm.capability-outcome.v1` | Closed completed, failed, and outcome-unknown result set for Capability invocation |
| `apxm.execution-admission.v1` | Signed, expiring, nonce-bound authority envelope with exact Port bindings, target and confinement claims |

## Referenced constitution envelopes

`apxm.contract-constitution.v1`, `apxm.contract-common.v1`,
`apxm.port-contract.v1`, `apxm.port-requirement.v1`, and
`apxm.execution-commit.v1`.

The Capability invocation descriptor binds the owned request and outcome
schemas. The atomic execution-commit descriptor is built on top of the
`apxm.execution-commit.v1` envelope. Both name `agents` as semantic owner; the
former keeps application arguments separate from invocation authority, while
the latter records one atomic compare-and-commit boundary with no split or
partial path.

The Capability lifecycle publishes the exact SHA-256 effect-id preimage and
the recursively key-sorted compact JSON request-identity preimage. Its Port
Contract binds one deterministic digest bundle over both the invocation and
outcome vector files, so neither half of the request/outcome boundary can drift
without changing the contract digest.

The owned request schema carries that digest as `sha256:<64 lowercase hex>`.
Boundaries whose schema fixes SHA-256 separately, including the Host effect
wire, carry the same digest bytes as bare `<64 lowercase hex>`; they do not
rehash a second preimage. Rust consumers use
`capability_request_digest_sha256_hex` for that exact representation rather
than stripping the prefix or rebuilding the request identity.

## Server and Host consumption

The executor boundary carries one `apxm_program::CapabilityRequest` inside
`apxm_capability_iface::CapabilityInvocation`. Server and Host-effect
implementations consume that typed request directly:

- `capability_id` comes from `capability_ref`;
- canonical Host `args` come from `arguments`, whose `type_ref` is the AIR
  `capability.invoke` arguments operand type;
- the Program Invocation and NodeExecution occurrence come only from
  `correlation`;
- Acting Principal, Agent Identity, the single governing Grant, and approval
  references come only from `authority` and are checked against verified Auth
  admission;
- the Host idempotency key is the canonical `effect_id`; and
- the Host request digest is `request_digest_sha256_hex()`, the bare encoding
  of the exact owner request digest.

`execution_id`, `graph_id`, and numeric receipt `node_id` remain operational
receipt coordinates. They never supply or replace a Program Invocation,
NodeExecution, argument type, identity, authority, effect id, or request
digest. A missing canonical request, a post-digest argument edit, or any
coordinate/authority mismatch fails before implementation dispatch.

This consumer is pinned to Host SDK source revision
`ff48332f2ce6af8a45eb38a15f4510a134223f5f`. Its
`apxm.host-sdk-owner-descriptor.v1` semantic digest is
`sha256:a007bb8daee44cfc5156a358bd4c0f4665adefc0d731738ead9c50a31734e14b`,
and the SHA-256 checksum of the exact descriptor repository bytes is
`sha256:d771d3f2c4a50fdeee0c57475c4c00fc6b4121b1e4bf5147a57a76bf11ad7a88`.
The semantic digest identifies the canonical descriptor content without its
self-digest field; the exact checksum detects any byte-level descriptor drift.

## Gate

```
dekk agents owner-descriptor
```

The gate validates every vector against its schema, enforces the
`artifact_semantic`-only abstraction rule and the single atomic write-set rule,
verifies every recorded content-addressed digest is current, pins the exact
referenced Host SDK cohort by revision plus semantic and exact descriptor
digests, and confirms the descriptor carries no delivery-process references.
Regenerate digests after any change with `dekk agents owner-descriptor-sync`.

The owner descriptor is identified by its deterministic canonical digest.
Optional distribution signatures remain detached metadata and are not part of
the owner contract or its development gate.
