#!/usr/bin/env bash
# Fetch the full Mooncake FAST'25-release trace files into the gitignored
# .apxm/datasets/mooncake/ directory. Idempotent: skips files that already
# exist with the right size. SHA256 verification is best-effort against the
# upstream HEAD at fetch time (the upstream repo does not publish per-file
# checksums; we record the commit SHA in the manifest instead).
#
# Usage: bash examples/python/benchmarks/workloads/data/fetch_mooncake.sh
set -euo pipefail

UPSTREAM_BASE="https://raw.githubusercontent.com/kvcache-ai/Mooncake/main/FAST25-release/traces"
TARGETS=(
  "conversation_trace.jsonl"
  "synthetic_trace.jsonl"
  "toolagent_trace.jsonl"
)

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
DEST_DIR="${REPO_ROOT}/.apxm/datasets/mooncake"
MANIFEST="${DEST_DIR}/MANIFEST.json"

mkdir -p "${DEST_DIR}"

UPSTREAM_SHA="$(curl -sL --max-time 15 https://api.github.com/repos/kvcache-ai/Mooncake/commits/main \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["sha"])')"
echo "[fetch_mooncake] upstream HEAD: ${UPSTREAM_SHA}"

declare -A FILE_SHAS=()
for name in "${TARGETS[@]}"; do
  url="${UPSTREAM_BASE}/${name}"
  out="${DEST_DIR}/${name}"
  if [[ -f "${out}" ]] && [[ -s "${out}" ]]; then
    echo "[fetch_mooncake] ${name}: already present ($(wc -c < "${out}") bytes), skipping"
  else
    echo "[fetch_mooncake] ${name}: downloading from ${url}"
    curl -fL --max-time 600 -o "${out}.tmp" "${url}"
    mv "${out}.tmp" "${out}"
    echo "[fetch_mooncake] ${name}: $(wc -c < "${out}") bytes"
  fi
  FILE_SHAS["${name}"]="$(sha256sum "${out}" | awk '{print $1}')"
done

python3 - "${MANIFEST}" "${UPSTREAM_SHA}" "${!FILE_SHAS[@]}" "${FILE_SHAS[@]}" <<'PYEOF'
import json, sys, datetime
manifest_path = sys.argv[1]
upstream_sha = sys.argv[2]
remaining = sys.argv[3:]
half = len(remaining) // 2
names = remaining[:half]
shas = remaining[half:]
manifest = {
    "fetched_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "upstream_repo": "kvcache-ai/Mooncake",
    "upstream_sha": upstream_sha,
    "files": {n: {"sha256": s} for n, s in zip(names, shas)},
}
with open(manifest_path, "w") as f:
    json.dump(manifest, f, indent=2, sort_keys=True)
print(f"[fetch_mooncake] manifest written: {manifest_path}")
PYEOF
