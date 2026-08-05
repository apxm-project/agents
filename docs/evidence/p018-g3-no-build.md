# Evidence — P-018 G3 no-build (hosted durable checkpoint/output)

- Issue: [agents#39](https://github.com/apxm-project/agents/issues/39)
- Decision: **no-build** hosted durable checkpoint/output reference implementation
- Decision dossier: [`docs/plans/p018-hosted-durable-decision.md`](../plans/p018-hosted-durable-decision.md)
- Date: 2026-08-05

## What shipped

| Artifact | Role |
| --- | --- |
| `crates/runtime/commit-local` (`apxm-commit-local`) | Owner-local in-memory + filesystem `ExecutionCommitPort` adapters |
| `crates/runtime/commit-local/tests/p018_owner_local_conformance.rs` | Size, restart, portability, conflict, outcome_unknown suite |
| `tools/tests/test_p018_no_hosted_durable.py` | Absence scan for hosted/speculative checkpoint services |
| `tools/tests/test_p018_no_hosted_durable_verifier.py` | Regression coverage for the digest-anchored verifier |
| `tools/scripts/verify_p018_no_hosted_durable.py` | Deterministic verifier for digest-anchored evidence and checklist authority text |
| `docs/plans/p018-hosted-durable-decision.md` | Closed decision dossier |

## Commands

```text
dekk agents test-commit-local
dekk agents verify-p018-no-hosted-durable
dekk agents test-owner-gates
dekk agents test-canonical-only
dekk agents owner-descriptor
git diff --check

# Focused cargo-wrapper fallback used on this macOS arm64 host after the
# sanctioned Dekk cargo gate failed on repo-configured Linux compiler pins:
env -u CC -u CXX -u CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER \
  .dekk/env/bin/python tools/scripts/cargo.py test -p apxm-commit-local
```

## Results

- `dekk agents test-commit-local`: **blocked by repo-configured macOS native-toolchain readiness gate**
- Focused cargo-wrapper `apxm-commit-local` tests: **13 passed / 0 failed**
- `python -m unittest tools.tests.test_p018_no_hosted_durable`: **6 passed / 0 failed**
- `python -m unittest tools.tests.test_p018_no_hosted_durable_verifier`: **2 passed / 0 failed**
- `dekk agents verify-p018-no-hosted-durable`: **PASS**
- `dekk agents test-owner-gates`: **23 passed / 0 failed**
- `dekk agents test-canonical-only`: **14 passed / 0 failed**
- `dekk agents owner-descriptor`: **blocked by pre-existing unresolved cross-schema refs**
- `git diff --check`: **PASS**

The evidence record covers the listed owner-local and absence checks, and is attached to
the issue conversation with the pushed commit identity.

## Evidence digests

- `crates/runtime/commit-local/Cargo.toml` — `sha256:0d0fb0e7e14bfe8bc1e5ac4764ec59ce870b837ba1a7f5034168553f50f47d70`
- `crates/runtime/commit-local/src/lib.rs` — `sha256:844ad7f2f7b961bf5f7b869a050e9475093322eff9a5fc5a04ccf7d7f2d531c3`
- `crates/runtime/commit-local/src/store.rs` — `sha256:188f99ebca50db818b1c38c3ec43d54388aa48cf7dd79f00a634d164cde9205e`
- `crates/runtime/commit-local/tests/p018_owner_local_conformance.rs` — `sha256:4a6912d93b57a14b2b384e3b1336049480e711f14ada6da2cdf851854733683c`
- `docs/plans/p018-hosted-durable-decision.md` — `sha256:30c07ca0d2fd3af77d5a7f5a91eaf0d07aad00473005e2a1476e85464f9124b7`
- `docs/plans/g3-admission-runtime-checklist.md` — `sha256:e2c1c06b52ae92b86749aa3b288a25b79481fef6f908fb40701010b0eaa4e474`
- `tools/scripts/verify_p018_no_hosted_durable.py` — `sha256:0a78f2ba8cb3d0db5595f6917d4f0a2c44d8e781557cfdd031aec63e797df012`
- `tools/tests/test_p018_no_hosted_durable.py` — `sha256:a77fd4223ea1a3607735e97c31d4e9147d5828a71b40874ed3c6a99c79579f96`
- `tools/tests/test_p018_no_hosted_durable_verifier.py` — `sha256:5f50ac828cca4ff848b1957c67ff85cb55e2ebadbcf9ffe2d4ffd33d7b4aea13`

## Repository gate notes

- `dekk agents doctor` and `dekk agents test-commit-local` failed on this host
  before cargo execution because `.dekk.toml` injects Linux GNU `CC`/`CXX`
  compilers into macOS arm64 builds. This is a repository toolchain-policy
  blocker, not a P-018 source failure.
- `dekk agents owner-descriptor` currently fails on unresolved
  `apxm.contract-common.v1#/$defs/Identifier` references in checked-in schema
  vectors; no failure mentioned `apxm-commit-local`, the P-018 docs, or the
  verifier/absence gate.

## Absence proof checklist

- [x] No hosted checkpoint/output service crate, binary, Compose service, or route
- [x] No empty hosted abstraction / feature flag / speculative service stub for P-018
- [x] Kernel remains free of default durable-store wiring (`crates/runtime/kernel/src/bundle.rs`, `crates/runtime/kernel/src/lib.rs`)
- [x] Release contract covered by injected `ExecutionCommitPort` + owner-local adapters
- [x] No downstream product names in P-018 artifacts
- [x] Digest-anchored verifier matches the checked-in dossier, checklist, and owner-local artifacts

## Admission contract coordination

Does not change `apxm.execution-admission.v1` verification laws or weaken
crash/replay/confinement behavior owned by agents#36.
