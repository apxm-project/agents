# agents owner contract surface

This directory publishes the `agents` signed owner descriptor against the
already-published contract constitution layer. The constitution and its common
envelopes live in the sibling `contracts` workspace and are referenced here by
`$id` and file digest only; they are never copied or redefined.

## Layout

```
schemas/         closed Agent Program semantic schemas owned by agents
port-contracts/  the atomic execution-commit Port Contract descriptor
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

## Referenced constitution envelopes

`apxm.contract-constitution.v1`, `apxm.contract-common.v1`,
`apxm.port-contract.v1`, `apxm.port-requirement.v1`, and
`apxm.execution-commit.v1`.

The atomic execution-commit Port Contract descriptor is built on top of the
`apxm.execution-commit.v1` envelope: it names `agents` as the semantic owner and
records one atomic compare-and-commit boundary with no split or partial path.

## Gate

```
python3 contracts/tools/validate_owner_descriptor.py
```

The gate validates every vector against its schema, enforces the
`artifact_semantic`-only abstraction rule and the single atomic write-set rule,
verifies every recorded content-addressed digest is current, and confirms the
descriptor carries no delivery-process references. Regenerate digests after any
change with `--write-digests`.

The owner descriptor records `signing.status = signing_pending_no_key`: no
descriptor signing key exists in this repository yet, so the exact unsigned
`descriptor_digest` is published and the signature is intentionally absent.
