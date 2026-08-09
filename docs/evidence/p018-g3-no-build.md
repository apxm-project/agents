# Evidence — P-018 G3 no-build (hosted durable checkpoint/output)

- Issue: [agents#39](https://github.com/apxm-project/agents/issues/39)
- Decision: **no-build** hosted durable checkpoint/output reference implementation
- Decision dossier: [`docs/plans/p018-hosted-durable-decision.md`](../plans/p018-hosted-durable-decision.md)
- Date: 2026-08-05

This is a pre-merge publication candidate. It becomes authoritative only after
this PR is merged to `main`; the root coordinator reruns this verifier against
the merged Agents `main` ref and confirms issue #39 remains open.

## What shipped

| Artifact | Role |
| --- | --- |
| `crates/runtime/commit-local` (`apxm-commit-local`) | Owner-local in-memory + filesystem `ExecutionCommitPort` adapters |
| `crates/runtime/commit-local/tests/p018_owner_local_conformance.rs` | Size, restart, portability, conflict, outcome_unknown suite |
| `tools/tests/test_p018_no_hosted_durable.py` | Absence scan for hosted/speculative checkpoint services |
| `tools/tests/test_p018_no_hosted_durable_verifier.py` | Regression coverage for the digest-anchored verifier |
| `tools/scripts/verify_p018_no_hosted_durable.py` | Deterministic verifier for digest-anchored evidence, authority text, and bounded owner-scope absence |
| `docs/plans/p018-hosted-durable-decision.md` | Pre-merge decision dossier |

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
- `python -m unittest tools.tests.test_p018_no_hosted_durable_verifier`: **6 passed / 0 failed**
- `dekk agents verify-p018-no-hosted-durable`: **PASS**
- Bounded authoritative absence check over the complete `crates/runtime/commit-local` owner scope: **PASS**
- `dekk agents test-owner-gates`: **23 passed / 0 failed**
- `dekk agents test-canonical-only`: **14 passed / 0 failed**
- `dekk agents owner-descriptor`: **blocked by pre-existing unresolved cross-schema refs**
- `git diff --check`: **PASS**

The evidence record covers the listed owner-local and absence checks, and is attached to
the issue conversation with the pushed commit identity.

## Evidence digests

- `crates/runtime/commit-local/Cargo.toml` — `sha256:0d0fb0e7e14bfe8bc1e5ac4764ec59ce870b837ba1a7f5034168553f50f47d70`
- `crates/runtime/commit-local/src/filesystem.rs` — `sha256:c7559cde4ac38faf88951aaee03bc202bc7c0dada0f0fcf60841013a03f66dad`
- `crates/runtime/commit-local/src/lib.rs` — `sha256:844ad7f2f7b961bf5f7b869a050e9475093322eff9a5fc5a04ccf7d7f2d531c3`
- `crates/runtime/commit-local/src/memory.rs` — `sha256:c1be15b2168e87bdca10a71b04f08a920c38c8eb2ec7c2ab5dc1b28a4ad212bb`
- `crates/runtime/commit-local/src/store.rs` — `sha256:188f99ebca50db818b1c38c3ec43d54388aa48cf7dd79f00a634d164cde9205e`
- `crates/runtime/commit-local/tests/p018_owner_local_conformance.rs` — `sha256:4a6912d93b57a14b2b384e3b1336049480e711f14ada6da2cdf851854733683c`
- `docs/plans/p018-hosted-durable-decision.md` — `sha256:19fec55ab36f6f52d0afae64a15d63a32e78bc2d87924308a9fea19db07572c9`
- `docs/plans/g3-admission-runtime-checklist.md` — `sha256:49f91fe539d53ecb538e9284360fbd7b5e178447967b0c124737cdf5d467f7c0`
- `tools/scripts/verify_p018_no_hosted_durable.py` — `sha256:9bcd944c6ad1ce3e34dc168cc585b78f3674c89b8cf1bcc2f0dfa7754bf144b6`
- `tools/tests/test_p018_no_hosted_durable.py` — `sha256:d7fb8e3ba6f2a07696a139886b08958877b5179f10a02e086e915a7e7736e8dd`
- `tools/tests/test_p018_no_hosted_durable_verifier.py` — `sha256:24cf3b3998beb21f980ecf0f6dd7ab7c928d129ea7dd16ed867e8e9f3c358179`

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
- [x] Deterministic bounded owner-scope verifier rejects hosted/networked,
      multi-tenant, downstream product, alias, fallback, speculative and
      second-writer, feature-flag, placeholder surfaces
- [x] Digest-anchored verifier matches the checked-in dossier, checklist, and owner-local artifacts

## Admission contract coordination

Does not change `apxm.execution-admission.v1` verification laws or weaken
crash/replay/confinement behavior owned by agents#36.
