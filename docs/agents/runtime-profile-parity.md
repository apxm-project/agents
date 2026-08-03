# Embedded and reference-runtime profile parity

Status: implementation slice for P-014/G5; the full gate remains open until a
reference runtime host is wired to the released profile and the golden vectors
below run through both hosts.

## Shared composition law

Both an embedded composition root and the APXM reference runtime host must:

1. verify one signed, expiring, nonce-bound `ExecutionAdmission`;
2. construct one `RuntimeAdmission` with the exact admitted implementations and
   confinement attestation;
3. join the exact durable-event and program-composition bindings through
   `apxm_execution::RuntimeProfile`; and
4. submit the same `ExecutionRequest` to the same canonical execution driver.

`RuntimeProfile` retains the immutable `ExecutionPortBundle` for its lifetime,
rejects construction when a required driver port is absent, and keeps shutdown
state on the profile instance. It does not discover implementations, select a
provider, create a product service, or provide a fallback path.

## Required G5 evidence still outstanding

- deterministic embedded and reference-host composition fixtures must use the
  same profile, bindings, injected adapters, and golden AIR;
- positive, negative, cancellation, shutdown, recovery, and boundary vectors
  must compare canonical result/evidence/error invariants; and
- the reference host must be built from an independently released APXM profile,
  not from the retired `apxm-server` client or a downstream product checkout.

The current CLIC E2E smoke is correctly blocked until that APXM-owned host
artifact exists; this document does not treat the local CLI development profile
as a service runtime.
