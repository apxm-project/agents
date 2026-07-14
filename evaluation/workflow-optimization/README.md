# Workflow optimization evaluation fixture

This directory is a checked-in deterministic fixture for the offline prompt
evaluation contract. It contains no live model output, backend telemetry, run
artifact, historical run pointer, working-tree snapshot, or record that an
owner gate passed. Generated evaluation artifacts belong under the gitignored
`.apxm/evaluation/` tree.

## What this fixture proves

The fixture fixes disjoint optimization and held-out inputs, their digests and
case counts, two distinct arms, a quality threshold, and portable source
provenance in [preregistration.json](preregistration.json). It exercises the
contract validator in
[`tools/scripts/offline_prompt_evaluation.py`](../../tools/scripts/offline_prompt_evaluation.py)
and is run directly by the focused test in
[`tools/tests/test_offline_prompt_evaluation.py`](../../tools/tests/test_offline_prompt_evaluation.py).

Its backend evidence is deliberately:

```json
{
  "kind": "deterministic-fixture",
  "measurement_source": "fixture-values"
}
```

The checked-in `token_count` and `latency_ms` values are deterministic contract
inputs. They are not model telemetry, inference measurements, cost data, or a
quality/performance comparison. The generated summary makes that distinction
machine-readable with `measurements.is_backend_telemetry=false`.

## Running the fixture

Use the owner surfaces below from `workspace/agents`:

```bash
dekk agents test-offline-prompt-evaluation
dekk agents offline-prompt-evaluation \
  --preregistration evaluation/workflow-optimization/preregistration.json \
  --bundle-root evaluation/workflow-optimization \
  --optimization evaluation/workflow-optimization/optimization.jsonl \
  --held-out evaluation/workflow-optimization/held-out.jsonl \
  --baseline evaluation/workflow-optimization/baseline.jsonl \
  --candidate evaluation/workflow-optimization/candidate.jsonl
```

The focused test runs this exact repository bundle with a temporary artifact
root. The required `dekk agents test` owner gate also invokes the focused
evaluator suite through [`.dekk.toml`](../../.dekk.toml), so changing the
runner, fixture, or its evidence schema cannot be treated as covered by only
the Rust workspace tests.

## Observed-backend evidence

An `observed-backend-run` may use `measurement_source=backend-telemetry` only
when every baseline and candidate output row carries an exact, bundle-relative
`evidence` reference. The reference must name a file by portable path and
SHA-256. The runner verifies that receipt's digest and requires it to bind the
preregistered backend id/revision, arm id, case id, output digest, token count,
and latency value. Extra output fields and free-form backend metadata are
rejected.

This validation proves that the emitted result is bound to a portable receipt;
it does not independently attest that a backend actually executed the request.
A claim-bearing observed run still needs the owning backend's recorded or
signed execution evidence and its owner gates. Those backend attestation
surfaces are outside this local fixture.

The live execution counterpart is the preregistered bundle in
[`../workflow-optimization-observed/`](../workflow-optimization-observed/).
It runs through
[`tools/scripts/observed_prompt_evaluation.py`](../../tools/scripts/observed_prompt_evaluation.py),
which emits the request, provider response, recorded execution document, and
receipt required by the observed evidence contract.

## Claim boundary

The fixture is a regression test for integrity, portability, and decision
logic. It supports no live-backend quality, latency, cost, throughput, or
production-readiness claim. Any such claim must cite a separately produced
`.apxm/evaluation/.../manifest.json` and `summary.json`, the preregistration
used for that run, the receipt-backed backend evidence, and every affected
owner's current verification result.
