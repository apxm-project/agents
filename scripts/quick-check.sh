#!/bin/bash
# Quick check: build + tests + autofix (no benchmarks)
# For pre-commit use - faster than full CI

set -e
cd "$(dirname "$0")/.."

echo "=== Quick Check ==="

echo "1. Building..."
dekk apxm build 2>&1 | tail -2

echo "2. Running Rust tests..."
cargo test --workspace --quiet 2>&1 | grep "test result" | tail -1

echo "3. Running autofix validation..."
PYTHONPATH=crates/apxm-frontend/python python3 scripts/apxm-autofix.py --report-only 2>&1 | grep "Passed:"

echo "=== Quick Check Complete ==="
