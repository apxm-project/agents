# Test Status Report

**Date**: 2026-04-08
**Status**: 100% CLEAN ✓

## Test Suites

| Suite | Tests | Pass | Fail | Ignored |
|-------|-------|------|------|---------|
| Rust workspace | 1261 | 1248 | 0 | 13 |
| Python frontend | 42 | 42 | 0 | 0 |
| Example validation | 53 | 53 | 0 | 0 |
| Policy checks | 4 | 4 | 0 | 0 |

**Total**: 1360 tests, 1347 passed, 0 failed, 13 ignored

## Rust Workspace Tests

All 1248 tests pass across 16 crates:
- apxm-core: 31 tests
- apxm-graph: 39 tests
- apxm-ais: 2 tests
- apxm-compiler: 135 tests
- apxm-artifact: 7 tests
- apxm-driver: 7 tests
- apxm-credentials: 25 tests
- apxm-backends: 50 tests
- apxm-tools: 59 tests
- apxm-events: 110 tests
- apxm-acp: 19 tests
- apxm-sandbox: 27 tests
- apxm-server: 490 tests
- apxm-cli: 72 tests
- Integration tests: 176 tests

13 tests ignored (mostly WIP or platform-specific).

## Python Frontend Tests

All 42 tests pass:
- test_decorators.py: 5 tests (compile decorator, typed params, team sugar)
- test_dspy_bridge.py: 23 tests (DSPy optimization integration)
- test_graph_construction.py: 9 tests (graph building, edges, validation)
- test_imports.py: 4 tests (module imports)
- test_smoke.py: 1 test (basic integration)

## Example Validation

All 53 Python examples validate and compile:
- examples/python/acp-agents: 9 examples
- examples/python/basics: 2 examples
- examples/python/benchmarks: 11 examples
- examples/python/demo: 1 example (showcase)
- examples/python/multi-agent: 4 examples
- examples/python/patterns: 6 examples
- examples/python/self-hosted: 7 examples
- examples/python/workflows: 13 examples

## Policy Checks

All 4 policy checks pass:
1. No hardcoded 'ais.*' strings in lower_mlir.rs ✓
2. Python uses _generated/ constants ✓
3. AISOps.td matches AISOperationType enum ✓
4. Memory space uses typed enum attributes ✓

## Fixes Applied (2026-04-08)

### Python Frontend
- **test_graph_construction.py**: Fixed import path from `apxm.graph.constants` → `apxm.constants`

### Examples
- **examples/python/basics/hello.py**: Restored (was empty), updated to current API
- **examples/python/demo/showcase.py**: Fixed all positional argument usage
  - Removed third positional arg from `g.ask()` calls (use named placeholders)
  - Changed `g.think()` to use named placeholders instead of `{0}`, `{1}`, etc.
  - Changed `g.print()` to use named placeholders
  - Fixed `query_memory` usage: use control dependency (`>>`) instead of data dependency
  - Memory context provided through AAM, not as template variable

## Current State

- **0 test failures** across all suites
- **0 compilation errors**
- **0 validation errors**
- **0 policy violations**

All systems green. ✓
