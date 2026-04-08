# CI Benchmark Scripts

Automated testing and benchmark regression checking for APXM.

## Scripts

### `ci-benchmark.sh`
Full CI benchmark suite that runs:
1. Build (dekk apxm build)
2. Autofix validation (all examples must pass)
3. Policy check
4. Python tests
5. Benchmark compilation (O0 and O2 for all benchmarks)
6. Rust workspace tests

**Usage:**
```bash
bash scripts/ci-benchmark.sh
```

Exit code 0 = all checks passed
Exit code 1 = one or more checks failed

### `quick-check.sh`
Faster pre-commit check that runs:
1. Build
2. Rust tests
3. Autofix validation

**Usage:**
```bash
bash scripts/quick-check.sh
```

Recommended for pre-commit hooks or quick validation before pushing.

## GitHub Actions

The CI benchmark suite is configured to run on every push and pull request via `.github/workflows/benchmark.yml`.

**Features:**
- Runs on Ubuntu latest
- Uses Rust nightly toolchain
- Caches cargo registry, index, and target directory
- Uploads benchmark artifacts on completion

**Manual trigger:**
```bash
gh workflow run benchmark.yml
```

## Environment Requirements

Both scripts expect:
- `dekk` wrapper for conda environment management
- Rust nightly toolchain
- Python 3.11+
- APXM conda environment configured via `environment.yaml`

## Benchmark Coverage

Currently tests compilation of:
- `shared_prefix_fanout` - Tests prefix sharing optimization
- `chained_llm` - Tests sequential LLM calls
- `mixed_priority` - Tests priority scheduling
- `multi_model` - Tests multi-model routing

Both O0 (no optimization) and O2 (with optimization passes like FuseReasoning) are tested.

## Adding New Benchmarks

To add a new benchmark to CI:
1. Create benchmark in `examples/python/benchmarks/<name>.py`
2. Add `<name>` to the `for graph in` loop in `ci-benchmark.sh`
3. Ensure the benchmark exposes a `_graph` attribute

## Troubleshooting

**"No module named 'apxm.graph'"**: Known issue with Python test imports. The benchmark compilation step will still run.

**Timing test failures**: The `scheduler::lane_queue::tests::different_sessions_can_run_in_parallel` test can occasionally fail under heavy load. This is a known flaky test.

**Compilation failures**: Check that:
- All graph nodes use valid AIS operations
- Required attributes are present
- No circular dependencies exist
