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
dekk agents test-commit-local
dekk agents test-owner-gates
git diff --check

# Supplemental diagnostic commands used on this macOS arm64 host after the
# sanctioned Dekk cargo gate failed on repo-configured Linux compiler pins:
env -u CC -u CXX -u CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER \
  .dekk/env/bin/python tools/scripts/cargo.py test -p apxm-commit-local
env -u CC -u CXX -u CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER \
  .dekk/env/bin/python -m unittest tools.tests.test_p018_no_hosted_durable
```

## Results

- `dekk agents test-commit-local`: **blocked by repo-configured macOS native-toolchain readiness gate**
- Supplemental `apxm-commit-local` tests: **5 passed / 0 failed**
- Supplemental absence unittest: **5 passed / 0 failed**
- `dekk agents test-owner-gates`: **22 passed / 0 failed**
- `git diff --check`: **PASS**

The evidence record covers the listed owner-local and absence checks, and is attached to
the issue conversation with the pushed commit identity.

## Repository gate notes

- `dekk agents doctor` and `dekk agents test-commit-local` failed on this host
  before cargo execution because `.dekk.toml` injects Linux GNU `CC`/`CXX`
  compilers into macOS arm64 builds. This is a repository toolchain-policy
  blocker, not a P-018 source failure.
- `dekk agents owner-descriptor` currently fails on unresolved
  `apxm.contract-common.v1#/$defs/Identifier` references in checked-in schema
  vectors; no failure mentioned `apxm-commit-local`, the P-018 docs, or the
  absence gate.
- `dekk agents test` reached the repository frontend gates but failed on the
  same host-level native-toolchain readiness gate during the packed frontend
  and example lanes; its evaluator and privacy suites passed before the stop.

## Absence proof checklist

- [x] No hosted checkpoint/output service crate, binary, Compose service, or route
- [x] No empty hosted abstraction / feature flag / speculative service stub for P-018
- [x] Kernel remains free of default durable-store wiring (`crates/runtime/kernel/src/bundle.rs`, `crates/runtime/kernel/src/lib.rs`)
- [x] Release contract covered by injected `ExecutionCommitPort` + owner-local adapters
- [x] No downstream product names in P-018 artifacts

## Admission contract coordination

Does not change `apxm.execution-admission.v1` verification laws or weaken
crash/replay/confinement behavior owned by agents#36.
