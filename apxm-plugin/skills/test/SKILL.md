---
name: test
description: Run the APXM test suite
user-invocable: true
---

# Test

Runs the APXM test suite — over 1,000 unit, async, and integration tests across 14 crates. By default, the MLIR compiler crate is excluded because its tests require MLIR/LLVM 21 at runtime. All other tests run with mocks and need no API keys or external services.

## Commands

```bash
dekk apxm test                     # default: all tests except compiler (1000+ tests)
dekk apxm test --all               # include compiler tests (requires MLIR/LLVM 21)
dekk apxm test --runtime           # only apxm-runtime tests (381 tests: scheduler, execution)
dekk apxm test --compiler          # only apxm-compiler tests (35 tests, requires MLIR)
dekk apxm test --backends          # only apxm-backends tests (105 tests, uses mocks)
dekk apxm test --credentials       # only apxm-credentials tests (12 tests)
dekk apxm test -p apxm-graph       # tests for a specific crate by name
```

## What Gets Tested

The default suite (`--workspace --exclude apxm-compiler`) covers:

- **apxm-runtime** (381 tests) — Dataflow scheduler, parallel execution, token routing, work-stealing, retries
- **apxm-backends** (105 tests) — LLM backend protocol handling, mock responses, streaming, tool calls
- **apxm-core** (92 tests) — Type system, value conversions, error builders, constants
- **apxm-server** (43 tests) — HTTP API handlers, MCP/A2A protocol, request/response types
- **apxm-events** (42 tests) — Event serialization round-trips, structured logging
- **apxm-ais** (39 tests) — AIS operation specs, validation, required field checking
- **apxm-acp** (31 tests) — Agent Communication Protocol sessions, spawning, AAM bridge
- **apxm-graph** (28 tests) — Graph parsing, DAG validation, merge operations
- **apxm-cli** (25 tests) — CLI subcommand integration (validate, analyze, template, explain)
- **apxm-driver** (25 tests) — Compiler/runtime orchestration, sandbox E2E
- **apxm-sandbox** (18 tests) — Sandboxing, isolation levels, security manifests
- **apxm-credentials** (12 tests) — Credential storage, masking, permission checks
- **apxm-tools** (4 tests) — Tool registration and invocation
- **apxm-artifact** (2 tests) — Artifact serialization, BLAKE3 hashing

## Why apxm-compiler Is Excluded

The compiler crate links against MLIR/LLVM C++ libraries at both build and test time. Without the conda environment providing MLIR 21+, these tests fail to link. The default test command excludes it so tests work in any environment. Use `--all` or `--compiler` when you have MLIR available.

## Test Infrastructure

- **Mock LLM backend**: `MockLLMBackend` supports static responses, pattern matching (`when_prompt_contains`), call recording, and failure injection — no real API calls needed
- **Async tests**: 257 tests use `#[tokio::test]` for async code paths
- **Integration tests**: 4 crates have `tests/` directories with E2E scenarios (CLI binary spawning, sandbox lifecycle, ACP sessions with mock agents)
- **Snapshot testing**: The compiler crate uses `insta` for golden MLIR output verification

## CI Integration

The test script auto-detects CI environments and adjusts:
- Adds `--locked` for reproducible builds
- Limits parallelism based on available CPUs
- Sets `CARGO_INCREMENTAL=0` to save disk space

## When to Use

- After modifying any Rust source code, before committing
- As a quick smoke test: `dekk apxm test --runtime` (fastest, covers the scheduler)
- To verify LLM backend changes: `dekk apxm test --backends`
- In CI: the default command works without MLIR installed
