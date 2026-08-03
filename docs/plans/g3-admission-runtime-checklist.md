# G3 admission/runtime/effects checklist (agents#36)

Owner issue: [agents#36](https://github.com/apxm-project/agents/issues/36)
Capabilities: **P-006, P-007, P-008, P-009**
Gate: **G3**
Frozen contract: [execution-admission-contract.md](../agents/execution-admission-contract.md)

## Work items

- [x] Verify signed, expiring, nonce-bound product-neutral Execution Admission
      (`apxm_kernel::admission`, schema `apxm.execution-admission.v1`).
- [x] Resolve every required Port and exact target before execution; reject
      missing or ambiguous bindings (`resolve_exact_bindings` /
      `verify_execution_admission`), including closed slot contract IDs.
- [x] Construct runtime admission only from byte-for-byte admitted bindings and
      attest the exact confinement sandbox and policy before use
      (`RuntimeAdmission::admit`).
- [x] Invocation lifecycle, checkpoints, resume surface, cancellation and
      safe-boundary revocation (kernel instance + effect reducer +
      `CheckpointAdvancer`).
- [x] Capability/effect boundaries, idempotency and `outcome_unknown` without
      blind replay (`EffectState` / G3 suite).
- [x] Atomic execution commit and crash recovery around checkpoint/effect
      boundaries (`ExecutionCommitPort` + G3 crash/replay/lost-reply suite).
- [x] Reject split evidence and malformed atomic write members before an adapter
      receives a commit request (`ExecutionCommitRequest::validate`).
- [x] Pin confinement backend/policy and fail closed when unavailable
      (required confinement binding + escape negative).
- [x] Prove no ambient credentials, filesystem, network, plugin or unconfined
      fallback (parse + correlation refusals).

## Acceptance suites (`crates/runtime/kernel/tests/g3_admission_runtime.rs`)

- [x] Positive admission
- [x] Negative: expiry, signature, nonce reuse, audience, revocation
- [x] Ambiguous-binding / missing confinement
- [x] Product-plane + ambient + unconfined boundary
- [x] Confinement-escape
- [x] Crash / replay / lost-reply — one checkpoint advancer, no duplicate effect
- [x] Cancellation / revocation / outcome_unknown no blind replay
- [x] Instance identity boundary

## Dependencies (not owned here)

| Dependency | Contract needed | Status |
| --- | --- | --- |
| [agents#35](https://github.com/apxm-project/agents/issues/35) | Stable artifact digests / FrontendGraph→AIR for golden programs | Independent: admission consumes digests only |
| [agents#37](https://github.com/apxm-project/agents/issues/37) | Exact inference driver + usage evidence | Independent: model_target field frozen; driver not required for G3 admission slice |
| [vllm#3](https://github.com/apxm-project/vllm/issues/3) | Backend conformance | Out of scope for #36 |
| [agents#39](https://github.com/apxm-project/agents/issues/39) | P-018 hosted durable checkpoint build/no-build | **G3 gate blocker remains open** — in-memory/filesystem commit fixtures satisfy owner-local conformance for this PR; hosted decision tracked on #39 |

## Non-goals

- Merging to `main` from this PR without review
- Closing P-018
- Implementing inference/vLLM
- Importing or naming any downstream product
