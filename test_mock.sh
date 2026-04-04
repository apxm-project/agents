#!/bin/bash
export PATH="$HOME/.cargo/bin:$PATH"
cd $APXM_HOME
cargo test -p apxm-backends --test vllm_integration test_graph_registration_request_structure -- --exact --nocapture
