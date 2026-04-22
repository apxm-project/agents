#!/usr/bin/env bash
set -euo pipefail
# Run from the parent of the eval_harness package so `-m eval_harness.*` resolves.
cd "$(dirname "$0")/../.."

# Two equivalent traces (different node-id ordering)
cat > /tmp/a.json <<'EOF'
{"events":[
  {"node_id":1,"node_name":"ask_a","op":"ASK","prompt":"hi","model":"mock","params":"","parent_deps":[]},
  {"node_id":2,"node_name":"ask_b","op":"ASK","prompt":"yo","model":"mock","params":"","parent_deps":[]}
]}
EOF
cat > /tmp/b.json <<'EOF'
{"events":[
  {"node_id":2,"node_name":"ask_b","op":"ASK","prompt":"yo","model":"mock","params":"","parent_deps":[]},
  {"node_id":1,"node_name":"ask_a","op":"ASK","prompt":"hi","model":"mock","params":"","parent_deps":[]}
]}
EOF

result=$(python3 -m eval_harness.trace_diff /tmp/a.json /tmp/b.json)
[[ "$result" == "equivalent" ]] || { echo "expected equivalent, got: $result"; exit 1; }

# A divergent pair (different prompt)
sed -i 's/"hi"/"hi-different"/' /tmp/b.json
result=$(python3 -m eval_harness.trace_diff /tmp/a.json /tmp/b.json || true)
[[ "$result" == divergent_at_node:* ]] || { echo "expected divergent_at_node:N, got: $result"; exit 1; }

echo "PASS"
