---
name: test
description: Run the project test suite
user-invocable: true
---

# Test Instructions

Run the project's test suite. Check the project root for:
- `Makefile` → run `make test`
- `CMakeLists.txt` → run `ctest --test-dir build`
- `package.json` → run `npm test`
- `Cargo.toml` → run `cargo test`
- `pyproject.toml` → run `pytest`

Report test results: how many passed, failed, and any error output.
