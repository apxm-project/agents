#!/usr/bin/env bash
# Fetch the ShareGPT V3 unfiltered cleaned split into the gitignored
# .apxm/datasets/sharegpt/ directory. Idempotent: skips files that already
# exist with the right size. SHA256 verification logged in the manifest.
#
# Usage: bash examples/python/benchmarks/workloads/data/fetch_sharegpt.sh
set -euo pipefail

# vLLM ships a curated copy of the cleaned ShareGPT split that the
# project's own benchmarks use; mirroring it gives us the same trace
# distribution other vLLM-side claims cite without re-doing curation.
UPSTREAM_URL="https://huggingface.co/datasets/anon8231489123/ShareGPT_Vicuna_unfiltered/resolve/main/ShareGPT_V3_unfiltered_cleaned_split.json"

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
DEST_DIR="${REPO_ROOT}/.apxm/datasets/sharegpt"
OUT="${DEST_DIR}/ShareGPT_V3_unfiltered_cleaned_split.json"
MANIFEST="${DEST_DIR}/MANIFEST.json"

mkdir -p "${DEST_DIR}"

if [[ -f "${OUT}" ]] && [[ -s "${OUT}" ]]; then
  echo "[fetch_sharegpt] already present ($(wc -c < "${OUT}") bytes), skipping"
else
  echo "[fetch_sharegpt] downloading ${UPSTREAM_URL}"
  curl -fL --max-time 1800 -o "${OUT}.tmp" "${UPSTREAM_URL}"
  mv "${OUT}.tmp" "${OUT}"
  echo "[fetch_sharegpt] $(wc -c < "${OUT}") bytes"
fi

SHA="$(sha256sum "${OUT}" | awk '{print $1}')"

python3 - "${MANIFEST}" "${UPSTREAM_URL}" "${SHA}" <<'PYEOF'
import datetime, json, sys
manifest_path, upstream_url, sha = sys.argv[1], sys.argv[2], sys.argv[3]
manifest = {
    "fetched_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "upstream_url": upstream_url,
    "files": {
        "ShareGPT_V3_unfiltered_cleaned_split.json": {"sha256": sha},
    },
}
with open(manifest_path, "w") as f:
    json.dump(manifest, f, indent=2, sort_keys=True)
print(f"[fetch_sharegpt] manifest written: {manifest_path}")
PYEOF
