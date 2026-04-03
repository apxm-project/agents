#!/usr/bin/env python3
"""
Runner for add-apxm-feature workflow.
Bakes a feature request into node 1's CONST_STR value, then executes.
Usage: python3 add-feature-runner.py "feature description"
"""
import json, sys, subprocess, tempfile, os

if len(sys.argv) < 2:
    print("Usage: add-feature-runner.py <feature description>")
    sys.exit(1)

feature = " ".join(sys.argv[1:])
print(f"Feature: {feature[:120]}...")

base = os.path.expanduser("~/projects/agents/apxm/examples/14-add-feature/add-apxm-feature.json")
with open(base) as f:
    graph = json.load(f)

for node in graph["nodes"]:
    if node["id"] == 1 and node["name"] == "feature_prompt":
        node["attributes"]["value"] = feature
        break

tmp = tempfile.NamedTemporaryFile(suffix=".json", delete=False, mode="w")
json.dump(graph, tmp, indent=2)
tmp.close()
print(f"Graph: {tmp.name}")

try:
    result = subprocess.run(
        ["dekk", "apxm", "execute", tmp.name],
        env=os.environ.copy()
    )
    sys.exit(result.returncode)
finally:
    os.unlink(tmp.name)
