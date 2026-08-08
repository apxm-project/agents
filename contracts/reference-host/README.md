# Reference-host publication

This subtree publishes the exact APXM reference-host contract cohort without
making the Rust binary the semantic authority. The canonical sources are the
`contracts/schemas/` and `contracts/vectors/` files in this checkout; the
publication copies are pinned to those bytes by the manifest.

- `manifests/` contains the executable publication manifest plus the
  product-neutral release manifest that wraps the full APXM-owned cohort.
- `schemas/` contains byte-for-byte copies of the canonical host execution,
  startup-input, JSONL host-request, host-response, private host-transport,
  invocation admission, runtime readiness, and runtime drain/quiescence schemas.
- `vectors/` contains the canonical startup-input, admission, readiness,
  host-response, private host-transport, and drain/quiescence conformance
  vectors plus deterministic
  embedded/reference-host parity goldens for positive commit, negative
  admission/provenance rejection, invalid AIR failure, cancellation,
  drain/shutdown, restart recovery, revocation, and boundary behavior.

The dedicated contract test verifies exact source/copy equality, current
SHA-256 file digests, schema and vector identity, closed publication paths,
release-cohort membership, required parity coverage, and rejection of the
retired `apxm.execution-admission.v1` alias.

The owner-descriptor gate (`python contracts/tools/validate_owner_descriptor.py`)
also validates this subtree directly. It fails closed on stale cohort digests,
publication drift from canonical APXM sources, and forbidden standalone
reverse-dependency names such as `clic` or `coordinator` in machine-readable
publication data.

Build the canonical executable and record its exact receipt with:

```bash
dekk agents reference-host-receipt
```

Run the executable only with an explicit startup-input document:

```bash
apxm-reference-host --startup-input /absolute/path/to/startup-input.json
```

That startup input must bind to the exact reference-host release manifest and
carry non-placeholder admitted digests for the release, port bindings, and
resource ceiling. Missing, mismatched, or placeholder digests are rejected at
startup.

The startup input also attests the selected transport. The default
`jsonl-stdin-stdout` input is valid only for the stdio launch; a private socket
launch requires an input generated with
`--transport-protocol jsonl-unix-stream`. The host rejects a mismatch before
binding an endpoint.

For `invoke`, send one `apxm.runtime.host-request.v1` JSONL record containing
canonical `apxm.air.v1` plus an already-materialized
`apxm.invocation-admission.v1`. The reference host verifies both against its
startup inputs; it does not compile source or mint authority.

The optional product-neutral private transport uses the same request and
response bytes over an owner-only AF_UNIX endpoint with one JSON object per
line. The endpoint path is injected by the runtime owner; it is not public,
does not mint authority, and a disconnect or timeout is outcome-unknown to the
caller until the exact invocation receipt is reconciled. Committed receipts are
also appended to a `0600` journal beside the socket and recovered on process
restart; conflicting or malformed journal records fail closed.
