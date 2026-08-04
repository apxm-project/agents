# Reference-host publication

This subtree publishes the exact APXM reference-host contract cohort without
making the Rust binary the semantic authority. The canonical sources are the
`contracts/schemas/` and `contracts/vectors/` files in this checkout; the
publication copies are pinned to those bytes by the manifest.

- `manifests/` contains the executable publication manifest plus the
  product-neutral release manifest that wraps the full APXM-owned cohort.
- `schemas/` contains byte-for-byte copies of the canonical host execution,
  invocation admission, runtime readiness, and runtime drain/quiescence
  schemas.
- `vectors/` contains the canonical admission/readiness/drain-quiescence
  conformance vectors and deterministic embedded/reference-host parity goldens
  for positive commit, negative admission/provenance rejection, invalid AIR
  failure, cancellation, drain/shutdown, restart recovery, revocation, and
  boundary behavior.

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
