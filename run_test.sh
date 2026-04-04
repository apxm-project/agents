#!/bin/bash
# Temporary test runner script
source ~/.bashrc 2>/dev/null || true
export PATH="$HOME/.cargo/bin:$PATH"
cd $APXM_HOME
cargo test -p apxm-backends --test vllm_integration
