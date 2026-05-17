#!/usr/bin/env bash
# Fetch the LooGLE long-context benchmark splits into the gitignored
# .apxm/datasets/loogle/ directory. Idempotent; per-file sha256 recorded
# in MANIFEST.json.
#
# Usage: bash examples/python/benchmarks/workloads/data/fetch_loogle.sh
set -euo pipefail

# Upstream HF dataset: bigai-nlco/LooGLE. The two retrieval-style
# splits below are the most relevant for prefix-cache evaluation
# (a long document + multiple sub-questions each).
UPSTREAM_BASE="https://huggingface.co/datasets/bigai-nlco/LooGLE/resolve/main"
TARGETS=(
  "longdep_qa.jsonl"
  "shortdep_qa.jsonl"
)

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
DEST_DIR="${REPO_ROOT}/.apxm/datasets/loogle"
MANIFEST="${DEST_DIR}/MANIFEST.json"

mkdir -p "${DEST_DIR}"

declare -A FILE_SHAS=()
for name in "${TARGETS[@]}"; do
  url="${UPSTREAM_BASE}/${name}"
  out="${DEST_DIR}/${name}"
  if [[ -f "${out}" ]] && [[ -s "${out}" ]]; then
    echo "[fetch_loogle] ${name}: already present ($(wc -c < "${out}") bytes), skipping"
  else
    echo "[fetch_loogle] ${name}: downloading from ${url}"
    curl -fL --max-time 1800 -o "${out}.tmp" "${url}"
    mv "${out}.tmp" "${out}"
    echo "[fetch_loogle] ${name}: $(wc -c < "${out}") bytes"
  fi
  FILE_SHAS["${name}"]="$(sha256sum "${out}" | awk '{print $1}')"
done

python3 - "${MANIFEST}" "${!FILE_SHAS[@]}" "${FILE_SHAS[@]}" <<'PYEOF'
import datetime, json, sys
manifest_path = sys.argv[1]
remaining = sys.argv[2:]
half = len(remaining) // 2
names = remaining[:half]
shas = remaining[half:]
manifest = {
    "fetched_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "upstream_repo": "bigai-nlco/LooGLE",
    "files": {n: {"sha256": s} for n, s in zip(names, shas)},
}
with open(manifest_path, "w") as f:
    json.dump(manifest, f, indent=2, sort_keys=True)
print(f"[fetch_loogle] manifest written: {manifest_path}")
PYEOF
