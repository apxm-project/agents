---
name: test
description: Run the APXM test suite
user-invocable: true
---

# Test

Runs the APXM test suite through the Dekk-managed environment. By default, the MLIR compiler crate is excluded so the core tests can run without linking the compiler in every environment. All default tests use mocks and need no API keys or external services.

## Commands

```bash
dekk apxm test                     # default suite
dekk apxm test-all                 # full workspace, including compiler tests
dekk apxm test-cli                 # CLI tests with driver and metrics features
dekk apxm test-python-frontend     # Python frontend tests
dekk apxm test-quality-eval        # offline quality-eval harness tests
```

## What Gets Tested

The default suite (`--workspace --exclude apxm-compiler`) covers:

- **apxm-runtime** — Dataflow scheduler, parallel execution, token routing, work-stealing, retries
- **apxm-backends** — LLM backend protocol handling, mock responses, streaming, tool calls
- **apxm-core** — Type system, value conversions, error builders, constants
- **apxm-server** — HTTP API handlers, MCP/A2A protocol, request/response types
- **apxm-events** — Event serialization round-trips and structured logging
- **apxm-ais** — AIS operation specs, validation, and required field checking
- **apxm-acp** — Agent Communication Protocol sessions, spawning, AAM bridge
- **apxm-cli** — CLI subcommand integration
- **apxm-driver** — Compiler/runtime orchestration
- **apxm-credentials** — Credential storage, masking, and permission checks
- **apxm-artifact** — Artifact serialization and BLAKE3 hashing

## Why apxm-compiler Is Excluded

The compiler crate links against MLIR/LLVM C++ libraries at both build and test time. The default test command excludes it so tests work in lighter environments. Use `--all` or `--compiler` when the Dekk-managed MLIR toolchain is installed.

## Test Infrastructure

- **Mock LLM backend**: `MockLLMBackend` supports static responses, pattern matching (`when_prompt_contains`), call recording, and failure injection — no real API calls needed
- **Async tests**: async code paths use `#[tokio::test]`
- **Integration tests**: crates with `tests/` directories cover CLI binary spawning, sandbox lifecycle, and ACP sessions with mock agents
- **Snapshot testing**: The compiler crate uses `insta` for golden MLIR output verification

## CI Integration

The test script auto-detects CI environments and adjusts:
- Adds `--locked` for reproducible builds
- Limits parallelism based on available CPUs
- Sets `CARGO_INCREMENTAL=0` to save disk space

## When to Use

- After modifying any Rust source code, before committing
- As a quick smoke test: `dekk apxm test`
- To verify frontend bindings: `dekk apxm test-python-frontend`
- In CI: the default command works without MLIR installed
