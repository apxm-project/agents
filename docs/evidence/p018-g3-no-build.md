# Evidence — P-018 G3 no-build (hosted durable checkpoint/output)

- Issue: [agents#39](https://github.com/apxm-project/agents/issues/39)
- Decision: **no-build** hosted durable checkpoint/output reference implementation
- Decision dossier: [`docs/plans/p018-hosted-durable-decision.md`](../plans/p018-hosted-durable-decision.md)
- Date: 2026-08-03

## What shipped

| Artifact | Role |
| --- | --- |
| `crates/runtime/commit-local` (`apxm-commit-local`) | Owner-local in-memory + filesystem `ExecutionCommitPort` adapters |
| `crates/runtime/commit-local/tests/p018_owner_local_conformance.rs` | Size, restart, portability, conflict, outcome_unknown suite |
| `tools/tests/test_p018_no_hosted_durable.py` | Absence scan for hosted/speculative checkpoint services |
| `docs/plans/p018-hosted-durable-decision.md` | Closed decision dossier |

## Commands

```text
python tools/scripts/cargo.py test -p apxm-commit-local
python -m unittest tools.tests.test_p018_no_hosted_durable
```

## Results

- `apxm-commit-local` tests: **5 passed / 0 failed**
- Absence unittest: **5 passed / 0 failed**
- Owner descriptor validation: **PASS**
- Owner gate command validation: **5 passed / 0 failed**
- Evidence record SHA-256: `dd3edf334e79a9eaa7739b990ff51816c16d47e684bd0e1f9ed57d03e4f53906`

The evidence record covers the four lines above, in order, and is attached to
the issue conversation with the pushed commit identity.

## Repository gate notes

- `dekk agents test` reached the repository frontend gates but failed on the
  host's pre-existing arm64 PyO3 linker symbols; the dependent TypeScript
  native-library copy and example parity lane therefore could not complete.
- `dekk agents doctor`, `dekk agents test-kernel`, and the release check also
  reported host-level Dekk/Cargo or toolchain failures; no P-018 source or
  contract failure was reported.

## Absence proof checklist

- [x] No hosted checkpoint/output service crate, binary, Compose service, or route
- [x] No empty hosted abstraction / feature flag / speculative service stub for P-018
- [x] Kernel remains free of default durable-store wiring (`crates/runtime/kernel/src/bundle.rs`, `crates/runtime/kernel/src/lib.rs`)
- [x] Release contract covered by injected `ExecutionCommitPort` + owner-local adapters
- [x] No downstream product names in P-018 artifacts

## Admission contract coordination

Does not change `apxm.execution-admission.v1` verification laws or weaken
crash/replay/confinement behavior owned by agents#36.
